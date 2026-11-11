//! What this Mac does for the stock plugins, through the tools every macOS has: `pmset` for the
//! battery and the screen, `pbcopy`/`pbpaste` for the clipboard, `osascript` for notifications and
//! the pairing question, `open` for links and files, `afplay` for the ring, `scutil` for the name.
//!
//! Text from a device (its name, a ping's message) never becomes AppleScript source: it goes to the
//! script as arguments (`on run argv`), so nothing in it can run as a script. The first argument is
//! always our own word, so osascript's options end before any of the device's text (BSD getopt
//! stops at the first argument that isn't an option); the device's text starts at item 2.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, Command};

/// argv's item 1: ours, never an option, so what follows is never read as one.
const FIRST: &str = "vyoma";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Battery {
    pub charge: i64,
    pub charging: bool,
}

/// The Mac's battery, or None on a Mac without one.
pub async fn battery() -> Option<Battery> {
    let out = Command::new("pmset").args(["-g", "batt"]).output().await.ok()?;
    parse_battery(&String::from_utf8_lossy(&out.stdout))
}

/// `pmset -g batt`: "Now drawing from 'AC Power'" then, with a battery,
/// " -InternalBattery-0 (id=…)\t87%; charging; 1:02 remaining present: true".
fn parse_battery(text: &str) -> Option<Battery> {
    let line = text.lines().find(|l| l.contains("InternalBattery"))?;
    let (before, after) = line.split_once('%')?;
    let digits: String = before.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<Vec<_>>().into_iter().rev().collect();
    let charge = digits.parse::<i64>().ok()?.clamp(0, 100);
    let state = after.split(';').nth(1).unwrap_or("").trim();
    // "charging" and "finishing charge" are charging; "discharging", "charged" and
    // "AC attached; not charging" aren't.
    let charging = state == "charging" || state.starts_with("finishing charge");
    Some(Battery { charge, charging })
}

/// The Mac's name as set in System Settings > General > Sharing.
pub async fn name() -> String {
    let named = Command::new("scutil").args(["--get", "ComputerName"]).output().await;
    let raw = match named {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_owned(),
        _ => String::new(),
    };
    let raw = if raw.is_empty() { std::env::var("HOST").unwrap_or_else(|_| "Mac".into()) } else { raw };
    vyoma_core::lan::device_name(&raw)
}

/// Runs an AppleScript whose text comes from us alone, with [args] as `argv` from item 2 on.
fn script(lines: &[&str], args: &[&str]) -> Command {
    let mut c = Command::new("osascript");
    for l in lines {
        c.arg("-e").arg(l);
    }
    c.arg(FIRST).args(args);
    c.stdin(Stdio::null()).kill_on_drop(true);
    c
}

/// A notification: Vyoma's own from inside Vyoma.app ([crate::notify]), else by osascript, which
/// macOS credits to Script Editor.
pub async fn notify(title: &str, body: &str) {
    #[cfg(target_os = "macos")]
    if crate::notify::post(title, body) {
        return;
    }
    let lines = ["on run argv", "display notification (item 3 of argv) with title (item 2 of argv)", "end run"];
    match script(&lines, &[title, body]).output().await {
        Ok(out) if out.status.success() => {}
        Ok(out) => tracing::warn!("couldn't show \"{title}\": {}", String::from_utf8_lossy(&out.stderr).trim()),
        Err(e) => tracing::warn!("couldn't show \"{title}\": {e}"),
    }
}

/// A phone's notification, [app]'s under its [title]: a later one under the same [key] replaces
/// it, and [withdraw] takes it down (inside Vyoma.app; osascript's can't be).
pub async fn phone_notification(key: &str, app: &str, title: &str, text: &str) {
    #[cfg(target_os = "macos")]
    if crate::notify::post_as(Some(key), title, app, text) {
        return;
    }
    let _ = key;
    let lines = ["on run argv", "display notification (item 4 of argv) with title (item 2 of argv) subtitle (item 3 of argv)", "end run"];
    if let Err(e) = script(&lines, &[title, app, text]).output().await {
        tracing::warn!("couldn't show a notification: {e}");
    }
}

/// Opens Vyoma.app (its window), from the agent inside it or wherever it's installed.
pub fn open_app() {
    if let Ok(mut child) = std::process::Command::new("open").args(["-b", crate::launchd::BUNDLE_ID]).stdin(Stdio::null()).spawn() {
        std::thread::spawn(move || child.wait());
    }
}

/// The phone dismissed the notification shown under [key].
pub fn withdraw(key: &str) {
    #[cfg(target_os = "macos")]
    crate::notify::withdraw(key);
    let _ = key;
}

/// [notify] from a thread of its own (a command that didn't start).
pub fn notify_now(title: &str, body: &str) {
    #[cfg(target_os = "macos")]
    if crate::notify::post(title, body) {
        return;
    }
    let _ = std::process::Command::new("osascript")
        .args(["-e", "on run argv", "-e", "display notification (item 3 of argv) with title (item 2 of argv)", "-e", "end run", FIRST, title, body])
        .stdin(Stdio::null())
        .status();
}

/// A question with two buttons: true for [yes]. Asked as a dialog that gives up after
/// [timeout]; the child is killed (and the dialog closed) if this is dropped.
pub fn ask(title: &str, text: &str, no: &str, yes: &str, timeout: Duration) -> Result<Child> {
    let secs = timeout.as_secs().max(1).to_string();
    let lines = [
        "on run argv",
        "set answer to display dialog (item 3 of argv) with title (item 2 of argv) buttons {item 4 of argv, item 5 of argv} default button 2 giving up after ((item 6 of argv) as integer)",
        "if gave up of answer then return \"\"",
        "return button returned of answer",
        "end run",
    ];
    let child = script(&lines, &[title, text, no, yes, &secs]).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().context("no osascript")?;
    Ok(child)
}

/// A list to pick one of [items] from, with [ok] as the button: its output is the chosen item, or
/// nothing for Cancel. Brought to the front, as it answers something the person just did.
pub fn choose(title: &str, prompt: &str, ok: &str, items: &[String]) -> Result<Child> {
    let lines = [
        "on run argv",
        "activate",
        "set picked to choose from list (items 5 thru -1 of argv) with title (item 2 of argv) with prompt (item 3 of argv) OK button name (item 4 of argv) default items {item 5 of argv}",
        "if picked is false then return \"\"",
        "return item 1 of picked",
        "end run",
    ];
    let mut args = vec![title, prompt, ok];
    args.extend(items.iter().map(String::as_str));
    utf8(script(&lines, &args)).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().context("no osascript")
}

/// What [ask]'s dialog answered: true only for the [yes] button (not giving up, not Cancel).
pub fn answered_yes(output: &[u8], yes: &str) -> bool {
    String::from_utf8_lossy(output).trim() == yes
}

/// The clipboard's text, unless it holds none.
pub async fn clipboard() -> Option<String> {
    let out = utf8(Command::new("pbpaste")).output().await.ok()?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    (!text.is_empty()).then_some(text)
}

/// Puts [text] on the clipboard.
pub async fn set_clipboard(text: &str) -> Result<()> {
    let mut child = utf8(Command::new("pbcopy")).stdin(Stdio::piped()).spawn().context("no pbcopy")?;
    child.stdin.take().context("pbcopy's input")?.write_all(text.as_bytes()).await?;
    let status = child.wait().await?;
    if !status.success() {
        bail!("pbcopy failed");
    }
    Ok(())
}

/// pbcopy and pbpaste read the locale for the text's encoding, and a LaunchAgent has none: without
/// it they turn anything past ASCII into garbage.
fn utf8(mut c: Command) -> Command {
    c.env("LANG", "en_US.UTF-8").env("LC_CTYPE", "UTF-8");
    c
}

/// Opens [target] (a web link, a file or a folder) in the user's app for it.
pub fn open(target: &str) {
    if let Err(e) = Command::new("open").arg(target).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
        tracing::warn!("can't open it: {e}");
    }
}

/// Turns the display off; with "Require password after screen saver begins or display is turned
/// off" set to Immediately (the default), that locks the Mac. Unlocking from the phone isn't
/// possible: macOS lets only the person at the Mac in.
pub async fn sleep_display() -> Result<()> {
    let status = Command::new("pmset").arg("displaysleepnow").status().await.context("no pmset")?;
    if !status.success() {
        bail!("pmset couldn't turn the display off");
    }
    Ok(())
}

/// ~/Downloads, where shared files land.
pub fn downloads() -> PathBuf {
    home().join("Downloads")
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"))
}

/// A sound played over and over until stopped (a phone looking for this Mac).
#[derive(Default)]
pub struct Ringer(Option<tokio::task::JoinHandle<()>>);

/// A ring stops by itself after this, as the phone's does, should nobody press Found it.
const RING_FOR: Duration = Duration::from_secs(60);
const RING_SOUND: &str = "/System/Library/Sounds/Glass.aiff";

impl Ringer {
    pub fn start(&mut self) {
        self.stop();
        self.0 = Some(tokio::spawn(async {
            let ring = async {
                loop {
                    let Ok(status) = Command::new("afplay").arg(RING_SOUND).kill_on_drop(true).status().await else { break };
                    if !status.success() {
                        break;
                    }
                }
            };
            let _ = tokio::time::timeout(RING_FOR, ring).await;
        }));
    }

    /// Stops it; aborting drops the afplay child, which kills it.
    pub fn stop(&mut self) {
        if let Some(task) = self.0.take() {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_battery_from_pmset() {
        let charging = "Now drawing from 'AC Power'\n -InternalBattery-0 (id=4653155)\t87%; charging; 1:02 remaining present: true\n";
        assert_eq!(parse_battery(charging), Some(Battery { charge: 87, charging: true }));
        let on_battery = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=4653155)\t5%; discharging; 0:20 remaining present: true\n";
        assert_eq!(parse_battery(on_battery), Some(Battery { charge: 5, charging: false }));
        let full = "Now drawing from 'AC Power'\n -InternalBattery-0 (id=1)\t100%; charged; 0:00 remaining present: true\n";
        assert_eq!(parse_battery(full), Some(Battery { charge: 100, charging: false }));
        let held = "Now drawing from 'AC Power'\n -InternalBattery-0 (id=1)\t80%; AC attached; not charging present: true\n";
        assert_eq!(parse_battery(held), Some(Battery { charge: 80, charging: false }));
        let finishing = "Now drawing from 'AC Power'\n -InternalBattery-0 (id=1)\t98%; finishing charge; 0:05 remaining present: true\n";
        assert_eq!(parse_battery(finishing), Some(Battery { charge: 98, charging: true }));
        assert_eq!(parse_battery("Now drawing from 'AC Power'\n"), None, "a Mac without a battery");
    }

    #[test]
    fn only_the_yes_button_is_yes() {
        assert!(answered_yes(b"Accept\n", "Accept"));
        assert!(!answered_yes(b"Reject\n", "Accept"));
        assert!(!answered_yes(b"\n", "Accept"), "it gave up");
        assert!(!answered_yes(b"", "Accept"), "closed or failed");
    }

    #[test]
    fn device_text_is_passed_as_arguments_never_as_script() {
        let evil = "\" & (do shell script \"touch /tmp/x\") & \"";
        let c = script(&["on run argv", "return item 2 of argv", "end run"], &["-x", evil]);
        let args: Vec<_> = c.as_std().get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        // The scripts are ours alone; the device's text comes after our own first word, untouched.
        assert_eq!(args, ["-e", "on run argv", "-e", "return item 2 of argv", "-e", "end run", FIRST, "-x", evil]);
    }
}

/// Whether the screen is locked, from the login session's own record (CoreGraphics'
/// `CGSSessionScreenIsLocked`, there only while it's locked); None outside a login session.
#[cfg(target_os = "macos")]
pub fn screen_locked() -> Option<bool> {
    use std::ffi::{c_char, c_void};
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGSessionCopyCurrentDictionary() -> *const c_void;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFDictionaryGetValue(dict: *const c_void, key: *const c_void) -> *const c_void;
        fn CFStringCreateWithCString(alloc: *const c_void, text: *const c_char, encoding: u32) -> *const c_void;
        fn CFGetTypeID(cf: *const c_void) -> usize;
        fn CFBooleanGetTypeID() -> usize;
        fn CFBooleanGetValue(b: *const c_void) -> u8;
        fn CFRelease(cf: *const c_void);
    }
    const UTF8: u32 = 0x0800_0100;
    // SAFETY: a copied dictionary and a created string, each released once; the value is borrowed
    // from the dictionary and read as a boolean only after its type is checked.
    unsafe {
        let session = CGSessionCopyCurrentDictionary();
        if session.is_null() {
            return None;
        }
        let key = CFStringCreateWithCString(std::ptr::null(), c"CGSSessionScreenIsLocked".as_ptr(), UTF8);
        let value = if key.is_null() { std::ptr::null() } else { CFDictionaryGetValue(session, key) };
        let locked = !value.is_null() && CFGetTypeID(value) == CFBooleanGetTypeID() && CFBooleanGetValue(value) != 0;
        if !key.is_null() {
            CFRelease(key);
        }
        CFRelease(session);
        Some(locked)
    }
}

#[cfg(not(target_os = "macos"))]
pub fn screen_locked() -> Option<bool> {
    None
}

