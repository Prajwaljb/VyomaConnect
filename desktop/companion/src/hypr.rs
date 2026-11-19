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
    let mut seen = SEEN.lock().unwrap();
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

/// The next workspace with something on it, or the one before (round the end, as Hyprland's own `e+1`).
pub async fn switch_workspace(right: bool) -> Result<()> {
    let to = if right { "e+1" } else { "e-1" };
    dispatch(&format!("hl.dsp.focus({{ workspace = \"{to}\" }})"), &format!("workspace {to}")).await
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
    let quiet = |program: &str| {
        let mut c = std::process::Command::new(program);
        c.stderr(std::process::Stdio::null()).stdin(std::process::Stdio::null());
        c
    };
    if let Some(sig) = instance().and_then(|d| d.file_name().map(|n| n.to_owned()))
        && let Ok(out) = quiet("hyprctl").env("HYPRLAND_INSTANCE_SIGNATURE", sig).args(["hyprpaper", "listactive"]).output()
        && let Some(p) = parse_hyprpaper(&String::from_utf8_lossy(&out.stdout))
    {
        return Some(p);
    }
    let out = quiet("swww").arg("query").output().ok()?;
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
fn parse_swww(text: &str) -> Option<PathBuf> {
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
    fn reads_the_wallpaper_tools() {
        assert_eq!(parse_hyprpaper("DP-1 = /home/a/w.png\nHDMI-A-1 = /home/a/v.jpg\n"), Some(PathBuf::from("/home/a/w.png")));
        assert_eq!(parse_hyprpaper("WAYLAND-1: /usr/share/hypr/wall0.png"), Some(PathBuf::from("/usr/share/hypr/wall0.png")));
        assert_eq!(parse_hyprpaper("no wallpapers active"), None);
        assert_eq!(parse_swww(": DP-1: 2560x1440, scale: 1, currently displaying: image: /home/a/w.jpg"), Some(PathBuf::from("/home/a/w.jpg")));
        assert_eq!(parse_swww(": DP-1: 2560x1440, scale: 1, currently displaying: color: 000000"), None);
    }
}
