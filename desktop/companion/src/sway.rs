//! Sway, through its own IPC socket (i3's protocol, what swaymsg talks to): workspaces, the
//! displays' power, log out, the keyboard layout and swaybg's wallpaper from its config. Found by
//! probing, not by the desktop's name: `$SWAYSOCK`, else the only listening
//! `$XDG_RUNTIME_DIR/sway-ipc.<uid>.<pid>.sock` (a unit started before Sway handed systemd its
//! environment still finds it). i3 speaks the same protocol, but on X11 EWMH already does all this.

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// What a request gets before it's given up on.
const TIMEOUT: Duration = Duration::from_secs(3);
const MAGIC: &[u8; 6] = b"i3-ipc";
/// Answers bigger than this aren't Sway's (a tree of hundreds of windows is well under it).
const MAX_ANSWER: usize = 16 << 20;

/// i3-ipc's message types used here.
#[derive(Clone, Copy)]
#[repr(u32)]
pub enum Ask {
    Command = 0,
    Workspaces = 1,
    Config = 9,
}

/// The running Sway's socket. Looked for again every few seconds at most (the wallpaper watch
/// asks often, and each look knocks on the socket).
fn socket() -> Option<PathBuf> {
    static SEEN: std::sync::Mutex<Option<(std::time::Instant, Option<PathBuf>)>> = std::sync::Mutex::new(None);
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, found)) = seen.as_ref()
        && at.elapsed() < Duration::from_secs(5)
    {
        return found.clone();
    }
    let found = look_for_socket();
    *seen = Some((std::time::Instant::now(), found.clone()));
    found
}

fn look_for_socket() -> Option<PathBuf> {
    // Listening, not just there: a Sway that crashed leaves its socket file behind.
    let live = |p: &PathBuf| UnixStream::connect(p).is_ok();
    if let Some(named) = std::env::var_os("SWAYSOCK").map(PathBuf::from)
        && live(&named)
    {
        return Some(named);
    }
    // SAFETY: getuid has no failure mode.
    let prefix = format!("sway-ipc.{}.", unsafe { libc::getuid() });
    let base = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?);
    let mut found = std::fs::read_dir(base)
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_str().is_some_and(|n| n.starts_with(&prefix) && n.ends_with(".sock")))
        .map(|e| e.path())
        .filter(live);
    let first = found.next()?;
    // Two at once (a nested test one): only the environment can say which.
    found.next().is_none().then_some(first)
}

/// Whether this session is Sway's.
pub fn active() -> bool {
    socket().is_some()
}

/// A message as i3-ipc frames it: the magic, the payload's length and the type (native order).
fn frame(ask: Ask, payload: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(14 + payload.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(payload.len() as u32).to_ne_bytes());
    out.extend_from_slice(&(ask as u32).to_ne_bytes());
    out.extend_from_slice(payload.as_bytes());
    out
}

/// The answer's length from its header, once the header checks out.
fn answer_len(header: &[u8; 14]) -> Result<usize> {
    anyhow::ensure!(&header[..6] == MAGIC, "not Sway's answer");
    let len = u32::from_ne_bytes(header[6..10].try_into().unwrap_or_default()) as usize;
    anyhow::ensure!(len <= MAX_ANSWER, "Sway's answer is too big ({len} bytes)");
    Ok(len)
}

/// One request on the socket at [path]; its JSON answer.
pub async fn ask_at(path: &Path, ask: Ask, payload: &str) -> Result<Value> {
    let talk = async {
        let mut s = tokio::net::UnixStream::connect(path).await?;
        s.write_all(&frame(ask, payload)).await?;
        let mut header = [0u8; 14];
        s.read_exact(&mut header).await?;
        let mut body = vec![0u8; answer_len(&header)?];
        s.read_exact(&mut body).await?;
        anyhow::Ok(serde_json::from_slice(&body).context("Sway's answer isn't JSON")?)
    };
    tokio::time::timeout(TIMEOUT, talk).await.context("Sway didn't answer")?.context("couldn't reach Sway")
}

fn ask_at_blocking(path: &Path, ask: Ask, payload: &str) -> Result<Value> {
    use std::io::{Read, Write};
    let mut s = UnixStream::connect(path)?;
    s.set_read_timeout(Some(TIMEOUT))?;
    s.set_write_timeout(Some(TIMEOUT))?;
    s.write_all(&frame(ask, payload))?;
    let mut header = [0u8; 14];
    s.read_exact(&mut header)?;
    let mut body = vec![0u8; answer_len(&header)?];
    s.read_exact(&mut body)?;
    serde_json::from_slice(&body).context("Sway's answer isn't JSON")
}

/// Runs a Sway command; Err with Sway's reason when it refused.
pub async fn command(what: &str) -> Result<()> {
    command_at(&socket().context("Sway isn't running")?, what).await
}

async fn command_at(path: &Path, what: &str) -> Result<()> {
    refused(&ask_at(path, Ask::Command, what).await?).map_or(Ok(()), |why| bail!("Sway: {why}"))
}

/// A command's answer (`[{"success": true}, ...]`, one per command): why the first refusal, if any.
fn refused(answer: &Value) -> Option<String> {
    let list = answer.as_array()?;
    list.iter()
        .find(|r| r["success"].as_bool() != Some(true))
        .map(|r| r["error"].as_str().unwrap_or("it refused").to_owned())
}

/// The workspace beside this one, as Hyprland's swipe goes (the same rules): right to the next one
/// on this display, or past the last to a new one; left to the one before, nowhere past the first.
pub async fn switch_workspace(right: bool) -> Result<()> {
    let path = socket().context("Sway isn't running")?;
    let list = ask_at(&path, Ask::Workspaces, "").await?;
    match beside(&list, right) {
        Some(to) => command_at(&path, &to).await,
        None => Ok(()),
    }
}

/// The command a swipe from the focused workspace in Sway's [list] runs, if it goes anywhere.
/// Numbered workspaces only (`num` -1 is a named one, which has no place in the order).
fn beside(list: &Value, right: bool) -> Option<String> {
    let all = list.as_array()?;
    let here = all.iter().find(|w| w["focused"].as_bool() == Some(true))?;
    let (num, output) = (here["num"].as_i64()?, here["output"].as_str()?);
    let mine: Vec<i64> = all.iter().filter(|w| w["output"].as_str() == Some(output)).filter_map(|w| w["num"].as_i64()).filter(|&n| n >= 0).collect();
    match crate::hypr::workspace_beside(num, &mine, right)? {
        crate::hypr::Beside::InUse => Some(if right { "workspace next_on_output" } else { "workspace prev_on_output" }.into()),
        // A new one after every numbered workspace (on any display: a number names one workspace).
        crate::hypr::Beside::New => {
            let last = all.iter().filter_map(|w| w["num"].as_i64()).max().unwrap_or(num);
            Some(format!("workspace number {}", last.max(num) + 1))
        }
    }
}

/// Lights every display again (the screen went dark idle).
pub async fn displays_on() -> Result<()> {
    let path = socket().context("Sway isn't running")?;
    // "power" since Sway 1.8, "dpms" before it (removed later).
    match command_at(&path, "output * power on").await {
        Ok(()) => Ok(()),
        Err(e) => command_at(&path, "output * dpms on").await.map_err(|_| e),
    }
}

/// Ends the Sway session (the phone's Log out).
pub async fn exit() -> Result<()> {
    command("exit").await
}

/// The config Sway loaded (`get_config`): its text, includes not followed. Blocking and quick.
fn config() -> Option<String> {
    let answer = ask_at_blocking(&socket()?, Ask::Config, "").ok()?;
    answer["config"].as_str().map(str::to_owned)
}

/// The first keyboard layout and variant (`xkb_layout de`, `xkb_variant nodeadkeys` in an `input`
/// block or line), else the XKB_DEFAULT_* environment Sway also reads. Blocking.
pub fn keyboard_layout() -> Option<(String, String)> {
    config()
        .and_then(|c| parse_layout(&c))
        .or_else(|| {
            let layout = std::env::var("XKB_DEFAULT_LAYOUT").ok()?;
            let first = layout.split(',').next()?.trim().to_owned();
            let variant = std::env::var("XKB_DEFAULT_VARIANT").unwrap_or_default().split(',').next().unwrap_or("").trim().to_owned();
            (!first.is_empty()).then_some((first, variant))
        })
}

fn parse_layout(config: &str) -> Option<(String, String)> {
    let first = |key: &str| {
        words(config).find_map(|w| {
            let at = w.iter().position(|x| *x == key)?;
            let value = w.get(at + 1)?.split(',').next()?.trim_matches('"').trim();
            (!value.is_empty()).then(|| value.to_owned())
        })
    };
    Some((first("xkb_layout")?, first("xkb_variant").unwrap_or_default()))
}

/// The config's lines as words, comments dropped.
fn words(config: &str) -> impl Iterator<Item = Vec<&str>> {
    config.lines().map(|l| l.split('#').next().unwrap_or("").split_whitespace().collect::<Vec<_>>()).filter(|w| !w.is_empty())
}

/// The wallpaper's file: swaybg's, as the config sets it (`output * bg ~/a.jpg fill`), else swww's
/// (it runs on Sway too). Blocking and quick (the wallpaper watch asks every few seconds).
pub fn wallpaper() -> Option<PathBuf> {
    if let Some(p) = config().and_then(|c| parse_bg(&c, std::env::var_os("HOME").map(PathBuf::from).as_deref())) {
        return Some(p);
    }
    if !crate::setup::on_path("swww") {
        return None;
    }
    let out = crate::run::output(std::process::Command::new("swww").arg("query"), crate::run::QUICK)?;
    crate::hypr::parse_swww(&String::from_utf8_lossy(&out.stdout))
}

/// The last `output <name> bg|background <file> <mode>` (later lines win, as in Sway), `~` and
/// `$HOME` expanded; a colour (`#rrggbb solid_color`) isn't a file.
fn parse_bg(config: &str, home: Option<&Path>) -> Option<PathBuf> {
    let mut found = None;
    for w in words(config) {
        let Some(at) = w.iter().position(|x| *x == "bg" || *x == "background") else { continue };
        if !w.contains(&"output") || at + 1 >= w.len() {
            continue;
        }
        // The file may hold spaces when quoted: everything up to the mode word.
        let rest = &w[at + 1..];
        let end = if rest.len() > 1 { rest.len() - 1 } else { 1 };
        let file = rest[..end].join(" ");
        let file = file.trim_matches(|c| c == '"' || c == '\'');
        if file.starts_with('#') || file.is_empty() {
            continue;
        }
        let path = match (file.strip_prefix("~/").or_else(|| file.strip_prefix("$HOME/")), home) {
            (Some(rel), Some(home)) => home.join(rel),
            _ => PathBuf::from(file),
        };
        if path.is_absolute() {
            found = Some(path);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn frames_as_i3_ipc() {
        let f = frame(Ask::Command, "exit");
        assert_eq!(&f[..6], b"i3-ipc");
        assert_eq!(u32::from_ne_bytes(f[6..10].try_into().unwrap()), 4);
        assert_eq!(u32::from_ne_bytes(f[10..14].try_into().unwrap()), 0);
        assert_eq!(&f[14..], b"exit");
        let mut header = [0u8; 14];
        header.copy_from_slice(&frame(Ask::Workspaces, "abc")[..14]);
        assert_eq!(answer_len(&header).unwrap(), 3);
        header[0] = b'x';
        assert!(answer_len(&header).is_err());
    }

    fn ws(num: i64, output: &str, focused: bool) -> Value {
        json!({ "num": num, "name": num.to_string(), "output": output, "focused": focused })
    }

    #[test]
    fn swipes_as_hyprland_does() {
        let list = json!([ws(1, "DP-1", false), ws(2, "DP-1", true), ws(3, "HDMI-A-1", false)]);
        assert_eq!(beside(&list, false).as_deref(), Some("workspace prev_on_output"));
        // 3 is on the other display: right makes a new one after every number.
        assert_eq!(beside(&list, true).as_deref(), Some("workspace number 4"));
        let first = json!([ws(1, "DP-1", true), ws(4, "DP-1", false)]);
        assert_eq!(beside(&first, false), None, "nowhere past the first");
        assert_eq!(beside(&first, true).as_deref(), Some("workspace next_on_output"));
        let named = json!([{ "num": -1, "name": "mail", "output": "DP-1", "focused": true }]);
        assert_eq!(beside(&named, true).as_deref(), Some("workspace number 0"), "a named one: the first number after it");
        assert_eq!(beside(&json!([]), true), None);
    }

    #[test]
    fn reads_refusals() {
        assert_eq!(refused(&json!([{ "success": true }])), None);
        assert_eq!(refused(&json!([{ "success": true }, { "success": false, "error": "Unknown command" }])).as_deref(), Some("Unknown command"));
    }

    #[test]
    fn reads_the_layout_and_wallpaper_from_the_config() {
        let config = "# the keyboard\ninput type:keyboard {\n    xkb_layout \"de,us\"\n    xkb_variant nodeadkeys\n}\n\
                      output * bg ~/Pictures/old.png fill\noutput DP-1 bg /usr/share/backgrounds/sway/Sway_Wallpaper_Blue_1920x1080.png fill\n\
                      output HDMI-A-1 bg #203040 solid_color\n";
        assert_eq!(parse_layout(config), Some(("de".into(), "nodeadkeys".into())));
        assert_eq!(parse_layout("input * xkb_layout fr # french\n"), Some(("fr".into(), String::new())));
        assert_eq!(parse_layout("bindsym $mod+Return exec foot\n"), None);
        let home = Path::new("/home/me");
        assert_eq!(parse_bg(config, Some(home)), Some(PathBuf::from("/usr/share/backgrounds/sway/Sway_Wallpaper_Blue_1920x1080.png")), "the last file wins; a colour isn't one");
        assert_eq!(parse_bg("output * bg ~/Pictures/my sky.jpg fill\n", Some(home)), Some(PathBuf::from("/home/me/Pictures/my sky.jpg")));
        assert_eq!(parse_bg("output * background $HOME/a.png stretch\n", Some(home)), Some(PathBuf::from("/home/me/a.png")));
        assert_eq!(parse_bg("# output * bg /a.png fill\n", Some(home)), None);
    }

    /// A fake Sway on a socket: answers each request with [answer] and keeps what was asked.
    async fn fake(dir: &Path, answer: Value) -> (PathBuf, tokio::sync::mpsc::UnboundedReceiver<(u32, String)>) {
        let path = dir.join("sway-ipc.test.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = listener.accept().await {
                let mut header = [0u8; 14];
                s.read_exact(&mut header).await.unwrap();
                let mut body = vec![0u8; answer_len(&header).unwrap()];
                s.read_exact(&mut body).await.unwrap();
                let kind = u32::from_ne_bytes(header[10..14].try_into().unwrap());
                let _ = tx.send((kind, String::from_utf8(body).unwrap()));
                let text = answer.to_string();
                let mut reply = Vec::new();
                reply.extend_from_slice(MAGIC);
                reply.extend_from_slice(&(text.len() as u32).to_ne_bytes());
                reply.extend_from_slice(&kind.to_ne_bytes());
                reply.extend_from_slice(text.as_bytes());
                s.write_all(&reply).await.unwrap();
            }
        });
        (path, rx)
    }

    /// Against a running Sway (a nested one in tests): `VYOMA_SWAY_TEST=1 cargo test --release --lib sway::`
    /// with its SWAYSOCK; its config sets `xkb_layout de` and a swaybg file in VYOMA_SWAY_WALLPAPER.
    /// Switches its workspaces.
    #[tokio::test]
    async fn talks_to_sway() {
        if std::env::var_os("VYOMA_SWAY_TEST").is_none() {
            return;
        }
        assert!(active());
        let path = socket().unwrap();
        let on = || async {
            let list = ask_at(&socket().unwrap(), Ask::Workspaces, "").await.unwrap();
            list.as_array().unwrap().iter().find(|w| w["focused"] == true).unwrap()["num"].as_i64().unwrap()
        };
        let first = on().await;
        switch_workspace(false).await.unwrap();
        assert_eq!(on().await, first, "nowhere past the first");
        switch_workspace(true).await.unwrap();
        assert_eq!(on().await, first + 1, "past the last: a new one");
        switch_workspace(false).await.unwrap();
        assert_eq!(on().await, first);
        displays_on().await.unwrap();
        assert!(command_at(&path, "nosuchthing").await.is_err());
        let layout = tokio::task::spawn_blocking(keyboard_layout).await.unwrap();
        assert_eq!(layout.map(|l| l.0), Some("de".to_owned()));
        if let Ok(file) = std::env::var("VYOMA_SWAY_WALLPAPER") {
            assert_eq!(tokio::task::spawn_blocking(wallpaper).await.unwrap(), Some(PathBuf::from(file)));
        }
    }

    /// A fresh folder of its own under the system's temporary one.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vyoma-sway-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn talks_to_a_sway_socket() {
        let (dir, other) = (scratch("ok"), scratch("no"));
        let (ok, mut asked) = fake(&dir, json!([{ "success": true }])).await;
        command_at(&ok, "output * power on").await.unwrap();
        assert_eq!(asked.recv().await.unwrap(), (0, "output * power on".into()));
        let workspaces = ask_at(&ok, Ask::Workspaces, "").await.unwrap();
        assert!(workspaces.is_array());
        assert_eq!(asked.recv().await.unwrap().0, 1);

        let (no, _) = fake(&other, json!([{ "success": false, "error": "Unknown/invalid command 'power'" }])).await;
        let e = command_at(&no, "output * power on").await.unwrap_err();
        assert!(format!("{e:#}").contains("Unknown/invalid command"), "{e:#}");
        assert!(ask_at(&dir.join("nothing.sock"), Ask::Command, "exit").await.is_err());
        let _ = (std::fs::remove_dir_all(dir), std::fs::remove_dir_all(other));
    }
}
