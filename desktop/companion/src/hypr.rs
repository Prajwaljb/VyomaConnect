//! Hyprland, through its own IPC socket (`$XDG_RUNTIME_DIR/hypr/<instance>/.socket.sock`, what
//! hyprctl talks to): workspaces, the displays' power, the keyboard layout. Found by probing, not
//! by the desktop's name: the instance the environment names, else the only one running (a unit
//! started before Hyprland handed systemd its environment still finds it).

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// What a request gets before it's given up on.
const TIMEOUT: Duration = Duration::from_secs(3);

/// The running Hyprland's folder: its sockets are in it. Looked for again every few seconds at
/// most (the wallpaper watch asks often, and each look knocks on Hyprland's socket).
fn instance() -> Option<PathBuf> {
    static SEEN: std::sync::Mutex<Option<(std::time::Instant, Option<PathBuf>)>> = std::sync::Mutex::new(None);
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, found)) = seen.as_ref()
        && at.elapsed() < Duration::from_secs(5)
    {
        return found.clone();
    }
    let found = look_for_instance();
    *seen = Some((std::time::Instant::now(), found.clone()));
    found
}

fn look_for_instance() -> Option<PathBuf> {
    let base = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?).join("hypr");
    // Listening, not just there: a Hyprland that crashed or ended leaves its folder behind.
    let live = |dir: &PathBuf| std::os::unix::net::UnixStream::connect(dir.join(".socket.sock")).is_ok();
    if let Some(sig) = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE") {
        let dir = base.join(sig);
        if live(&dir) {
            return Some(dir);
        }
    }
    let mut found = std::fs::read_dir(&base).ok()?.flatten().map(|e| e.path()).filter(live);
    let first = found.next()?;
    // Two at once (a nested test one): only the environment can say which.
    found.next().is_none().then_some(first)
}

/// Whether this session is Hyprland's.
pub fn active() -> bool {
    instance().is_some()
}

/// One request on Hyprland's socket ("dispatch workspace e+1", "j/monitors"); its answer.
pub async fn request(what: &str) -> Result<String> {
    let path = instance().context("Hyprland isn't running")?.join(".socket.sock");
    let talk = async {
        let mut s = tokio::net::UnixStream::connect(&path).await?;
        s.write_all(what.as_bytes()).await?;
        let mut out = String::new();
        s.read_to_string(&mut out).await?;
        anyhow::Ok(out)
    };
    tokio::time::timeout(TIMEOUT, talk).await.context("Hyprland didn't answer")?.context("couldn't reach Hyprland")
}

/// A dispatcher, as Lua (Hyprland 0.57's `hl.dsp.focus({ workspace = "e+1" })`) or, for older
/// ones, the classic words (`workspace e+1`). Hyprland answers "ok" or why not.
pub async fn dispatch(lua: &str, classic: &str) -> Result<()> {
    let out = request(&format!("dispatch {lua}")).await?;
    if out.trim() == "ok" {
        return Ok(());
    }
    let older = request(&format!("dispatch {classic}")).await?;
    if older.trim() == "ok" {
        return Ok(());
    }
    bail!("Hyprland: {}", out.trim())
}

/// A JSON answer ("monitors", "getoption input:kb_layout").
pub async fn json(what: &str) -> Result<Value> {
    let out = request(&format!("j/{what}")).await?;
    serde_json::from_str(&out).with_context(|| format!("Hyprland's {what} isn't JSON"))
}

/// The workspace beside this one, as Hyprland's own touchpad swipe goes: right to the next one in
/// use, or past the last to a new empty one; left to the one before, and nowhere past the first.
pub async fn switch_workspace(right: bool) -> Result<()> {
    let here = json("activeworkspace").await?["id"].as_i64().context("no active workspace")?;
    let ids: Vec<i64> = json("workspaces").await?.as_array().into_iter().flatten().filter_map(|w| w["id"].as_i64()).filter(|&id| id > 0).collect();
    let to = match workspace_beside(here, &ids, right) {
        Some(Beside::InUse) => if right { "e+1" } else { "e-1" },
        Some(Beside::New) => "r+1",
        None => return Ok(()),
    };
    dispatch(&format!("hl.dsp.focus({{ workspace = \"{to}\" }})"), &format!("workspace {to}")).await
}

#[derive(Debug, PartialEq)]
pub(crate) enum Beside {
    /// One in use that way (Hyprland's `e±1`, which only wraps round when there's none).
    InUse,
    /// A new one after the last (`r+1`).
    New,
}

/// Where a swipe from workspace [here] goes, among the (normal) workspaces [ids] in use.
pub(crate) fn workspace_beside(here: i64, ids: &[i64], right: bool) -> Option<Beside> {
    if right {
        Some(if ids.iter().any(|&id| id > here) { Beside::InUse } else { Beside::New })
    } else {
        ids.iter().any(|&id| id < here).then_some(Beside::InUse)
    }
}

/// Lights every display again (the screen went dark idle).
pub async fn displays_on() -> Result<()> {
    // Exactly "on": a word it doesn't know toggles them (off, if they were on).
    dispatch("hl.dsp.dpms({ action = \"on\" })", "dpms on").await
}

/// Ends the Hyprland session (the phone's Log out).
pub async fn exit() -> Result<()> {
    dispatch("hl.dsp.exit()", "exit").await
}

/// Whether a lock screen holds the session: Hyprland's own answer, whichever program draws it
/// (a shell's, hyprlock...). logind's LockedHint is only set by lockers that bother (Caelestia's doesn't).
pub async fn locked() -> Result<bool> {
    json("locked").await?["locked"].as_bool().context("Hyprland didn't say whether it's locked")
}

/// How long a lock screen has to follow logind's Lock or Unlock before Vyoma does it itself.
const FOLLOW: Duration = Duration::from_secs(2);
/// How long hyprlock has to come up, or go, once asked here.
const SETTLE: Duration = Duration::from_secs(3);

/// After logind was asked to lock ([lock]) or unlock this session. What listens to logind does it
/// (a shell such as Caelestia's, hypridle's lock_cmd and unlock_cmd); where nothing does, hyprlock
/// is started (by Hyprland, so it outlives Vyoma) or told to unlock (its SIGUSR1). Err with what
/// to do when the session still isn't as asked.
pub async fn follow_logind(lock: bool) -> Result<()> {
    if reaches(lock, FOLLOW).await {
        return Ok(());
    }
    if lock {
        anyhow::ensure!(
            crate::setup::on_path("hyprlock"),
            "Nothing locked the screen: no lock screen here follows logind (hypridle's lock_cmd or your shell's), and hyprlock isn't installed"
        );
        tracing::info!("no lock screen followed logind: starting hyprlock");
        dispatch("hl.dsp.exec_cmd(\"hyprlock\")", "exec hyprlock").await?;
    } else {
        anyhow::ensure!(signal_mine("hyprlock", libc::SIGUSR1) > 0, "The lock screen didn't unlock: it doesn't follow logind's unlock (hypridle's unlock_cmd can make it)");
        tracing::info!("the lock screen didn't follow logind: told hyprlock to unlock");
    }
    anyhow::ensure!(reaches(lock, SETTLE).await, "The screen didn't {}", if lock { "lock" } else { "unlock" });
    Ok(())
}

/// Whether the session is (or within [within] gets) locked as [lock] says.
async fn reaches(lock: bool, within: Duration) -> bool {
    let until = std::time::Instant::now() + within;
    loop {
        match locked().await {
            Ok(now) if now == lock => return true,
            Err(_) => return true, // Hyprland can't say: nothing more to check against.
            _ => {}
        }
        if std::time::Instant::now() >= until {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Sends [signal] to this user's processes named [name]; how many there were.
fn signal_mine(name: &str, signal: libc::c_int) -> usize {
    // SAFETY: getuid has no failure mode.
    let me = unsafe { libc::getuid() };
    let mut sent = 0;
    for entry in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<libc::pid_t>().ok()) else { continue };
        let ours = std::fs::metadata(entry.path()).is_ok_and(|m| std::os::unix::fs::MetadataExt::uid(&m) == me);
        if ours && std::fs::read_to_string(entry.path().join("comm")).is_ok_and(|c| c.trim() == name) {
            // SAFETY: a signal to a process of our own user, found just now.
            if unsafe { libc::kill(pid, signal) } == 0 {
                sent += 1;
            }
        }
    }
    sent
}

/// Whether a plugin of this name is loaded (hyprexpo gives Hyprland an overview).
pub async fn has_plugin(name: &str) -> bool {
    match json("plugin list").await {
        Ok(Value::Array(list)) => list.iter().any(|p| p["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(name))),
        _ => request("plugin list").await.is_ok_and(|t| t.to_lowercase().contains(&name.to_lowercase())),
    }
}

/// The first keyboard layout and its variant Hyprland is set to ("de", "nodeadkeys"). Blocking:
/// the layout is read once, from wherever (inside the runtime or not).
pub fn keyboard_layout() -> Option<(String, String)> {
    let read = |key: &str| {
        let out = request_blocking(&format!("j/getoption {key}")).ok()?;
        // An option that isn't set comes back as "[[EMPTY]]".
        serde_json::from_str::<Value>(&out).ok()?["str"].as_str().filter(|s| *s != "[[EMPTY]]").map(str::to_owned)
    };
    let first = |s: String| s.split(',').next().unwrap_or("").trim().to_owned();
    let layout = read("input:kb_layout").map(first).filter(|l| !l.is_empty())?;
    Some((layout, read("input:kb_variant").map(first).unwrap_or_default()))
}

fn request_blocking(what: &str) -> Result<String> {
    ask_blocking(&instance().context("Hyprland isn't running")?.join(".socket.sock"), what)
}

fn ask_blocking(path: &std::path::Path, what: &str) -> Result<String> {
    use std::io::{Read, Write};
    let mut s = std::os::unix::net::UnixStream::connect(path)?;
    s.set_read_timeout(Some(TIMEOUT))?;
    s.set_write_timeout(Some(TIMEOUT))?;
    s.write_all(what.as_bytes())?;
    let mut out = String::new();
    s.read_to_string(&mut out)?;
    Ok(out)
}

/// The wallpaper's file: hyprpaper's (asked through hyprctl, which speaks its protocol: 0.8's is
/// binary), else swww's. Blocking and quick (the wallpaper watch asks every few seconds).
pub fn wallpaper() -> Option<PathBuf> {
    use std::process::Command;
    // Each with a deadline: hyprctl waits on hyprpaper's socket and swww on its daemon, and either
    // hung would hold the watch (and whoever waits on it) for good.
    if let Some(sig) = instance().and_then(|d| d.file_name().map(|n| n.to_owned()))
        && crate::setup::on_path("hyprctl")
        && let Some(out) = crate::run::output(Command::new("hyprctl").env("HYPRLAND_INSTANCE_SIGNATURE", sig).args(["hyprpaper", "listactive"]), crate::run::QUICK)
        && let Some(p) = parse_hyprpaper(&String::from_utf8_lossy(&out.stdout))
    {
        return Some(p);
    }
    if !crate::setup::on_path("swww") {
        return None;
    }
    let out = crate::run::output(Command::new("swww").arg("query"), crate::run::QUICK)?;
    parse_swww(&String::from_utf8_lossy(&out.stdout))
}

/// hyprpaper's `listactive`: "DP-1: /path/to/a.png" (0.8), "DP-1 = /path/to/a.png" before, a line per monitor.
fn parse_hyprpaper(text: &str) -> Option<PathBuf> {
    text.lines()
        .filter_map(|l| l.split_once(" = ").or_else(|| l.split_once(": ")))
        .map(|(_, p)| PathBuf::from(p.trim()))
        .find(|p| p.is_absolute())
}

/// swww's `query`: ": DP-1: 2560x1440, scale: 1, currently displaying: image: /path/a.jpg".
pub(crate) fn parse_swww(text: &str) -> Option<PathBuf> {
    text.lines().find_map(|l| l.split_once("image: ")).map(|(_, p)| PathBuf::from(p.trim())).filter(|p| p.is_absolute())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Against a running Hyprland (a nested one in tests): `VYOMA_HYPR_TEST=1 cargo test --release --lib hypr::`
    /// with its XDG_RUNTIME_DIR. Switches its workspaces.
    #[tokio::test]
    async fn talks_to_hyprland() {
        if std::env::var_os("VYOMA_HYPR_TEST").is_none() {
            return;
        }
        assert!(active());
        assert!(json("monitors").await.unwrap().as_array().is_some_and(|m| !m.is_empty()));
        let on = || async { json("activeworkspace").await.unwrap()["id"].as_i64().unwrap() };
        let first = on().await;
        switch_workspace(true).await.unwrap();
        let next = on().await;
        assert_ne!(first, next, "e+1 moved to the other workspace with a window");
        switch_workspace(false).await.unwrap();
        assert_eq!(on().await, first);
        displays_on().await.unwrap();
        assert!(!has_plugin("hyprexpo").await);
        assert!(dispatch("hl.dsp.nosuchthing()", "nosuchthing").await.is_err());
        let layout = tokio::task::spawn_blocking(keyboard_layout).await.unwrap();
        assert_eq!(layout.map(|l| l.0), Some("us".to_owned()));
        if let Ok(path) = std::env::var("VYOMA_HYPR_WALLPAPER") {
            assert_eq!(tokio::task::spawn_blocking(wallpaper).await.unwrap(), Some(PathBuf::from(path)));
        }
    }

    #[test]
    fn swipes_go_as_hyprlands_own() {
        assert_eq!(workspace_beside(1, &[1], true), Some(Beside::New), "past the last: a new one");
        assert_eq!(workspace_beside(1, &[1, 3], true), Some(Beside::InUse));
        assert_eq!(workspace_beside(3, &[1, 3], true), Some(Beside::New), "no wrapping round to 1");
        assert_eq!(workspace_beside(1, &[1, 3], false), None, "nothing before the first");
        assert_eq!(workspace_beside(3, &[1, 3], false), Some(Beside::InUse));
    }

    #[test]
    fn reads_the_wallpaper_tools() {
        assert_eq!(parse_hyprpaper("DP-1 = /home/a/w.png\nHDMI-A-1 = /home/a/v.jpg\n"), Some(PathBuf::from("/home/a/w.png")));
        assert_eq!(parse_hyprpaper("WAYLAND-1: /usr/share/hypr/wall0.png"), Some(PathBuf::from("/usr/share/hypr/wall0.png")));
        assert_eq!(parse_hyprpaper("no wallpapers active"), None);
        assert_eq!(parse_swww(": DP-1: 2560x1440, scale: 1, currently displaying: image: /home/a/w.jpg"), Some(PathBuf::from("/home/a/w.jpg")));
        assert_eq!(parse_swww(": DP-1: 2560x1440, scale: 1, currently displaying: color: 000000"), None);
    }
}
