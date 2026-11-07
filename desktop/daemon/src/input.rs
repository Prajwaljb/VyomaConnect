//! The phone as this PC's touchpad and keyboard (`kdeconnect.mousepad.request`), through the
//! RemoteDesktop portal, as kdeconnectd does on Wayland: every desktop has it (KDE, GNOME, COSMIC),
//! and the desktop asks the user once; a restore token reopens it after that.
//!
//! Characters are pressed as the real key that types them in the desktop's layout, with Shift when
//! that key needs it: sending keysyms through the portal doubled letters and then lost keys (seen
//! with the companion 2026-11-04, and here 2026-11-07). Only what no key types goes as a keysym.
//! Special keys are KDE Connect's numbers.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use vyoma_core::link::packet;
use zbus::{Connection, MatchRule, MessageStream, OwnedObjectPath, OwnedVariant, Variant};

pub const MOUSEPAD_REQUEST: &str = "kdeconnect.mousepad.request";
pub const MOUSEPAD_ECHO: &str = "kdeconnect.mousepad.echo";
pub const MOUSEPAD_KEYBOARDSTATE: &str = "kdeconnect.mousepad.keyboardstate";

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const REMOTE: &str = "org.freedesktop.portal.RemoteDesktop";

const BTN_LEFT: i32 = 0x110;
const BTN_RIGHT: i32 = 0x111;
const BTN_MIDDLE: i32 = 0x112;
const KEY_LEFTSHIFT: i32 = 42;
const KEY_LEFTCTRL: i32 = 29;
const KEY_LEFTALT: i32 = 56;
const KEY_LEFTMETA: i32 = 125;
const KEYSYM_RETURN: i32 = 0xff0d;
const KEYSYM_TAB: i32 = 0xff09;

/// KDE Connect's special key numbers as evdev codes (0: none).
fn special(n: i64) -> Option<i32> {
    Some(match n {
        1 => 14,        // Backspace
        2 => 15,        // Tab
        3 | 12 => 28,   // Linefeed, Return
        4 => 105,       // Left
        5 => 103,       // Up
        6 => 106,       // Right
        7 => 108,       // Down
        8 => 104,       // Page Up
        9 => 109,       // Page Down
        10 => 102,      // Home
        11 => 107,      // End
        13 => 111,      // Delete
        14 => 1,        // Escape
        15 => 99,       // SysRq / Print
        16 => 70,       // Scroll Lock
        21..=30 => 59 + (n - 21) as i32, // F1..F10
        31 => 87,       // F11
        32 => 88,       // F12
        _ => return None,
    })
}

/// The keysym that types [c]: Latin-1 is its own code, the rest Unicode's (0x0100_0000 + code).
fn keysym(c: char) -> i32 {
    match c {
        '\n' | '\r' => KEYSYM_RETURN,
        '\t' => KEYSYM_TAB,
        _ => {
            let u = c as u32;
            if (0x20..=0x7e).contains(&u) || (0xa0..=0xff).contains(&u) { u as i32 } else { (0x0100_0000 | u) as i32 }
        }
    }
}

#[repr(C)]
struct RuleNames {
    rules: *const std::ffi::c_char,
    model: *const std::ffi::c_char,
    layout: *const std::ffi::c_char,
    variant: *const std::ffi::c_char,
    options: *const std::ffi::c_char,
}

#[link(name = "xkbcommon")]
unsafe extern "C" {
    fn xkb_context_new(flags: std::ffi::c_int) -> *mut std::ffi::c_void;
    fn xkb_keymap_new_from_names(ctx: *mut std::ffi::c_void, names: *const RuleNames, flags: std::ffi::c_int) -> *mut std::ffi::c_void;
    fn xkb_keymap_min_keycode(keymap: *mut std::ffi::c_void) -> u32;
    fn xkb_keymap_max_keycode(keymap: *mut std::ffi::c_void) -> u32;
    fn xkb_keymap_key_get_syms_by_level(keymap: *mut std::ffi::c_void, key: u32, layout: u32, level: u32, syms: *mut *const u32) -> std::ffi::c_int;
}

/// Keysym → the key that types it in the desktop's first layout (evdev code) and whether Shift
/// is needed. Plasma's layout from kxkbrc; others get "us" until their desktops are done.
fn layout() -> &'static HashMap<u32, (i32, bool)> {
    static LAYOUT: std::sync::OnceLock<HashMap<u32, (i32, bool)>> = std::sync::OnceLock::new();
    LAYOUT.get_or_init(|| {
        let mut keys = HashMap::new();
        let name = std::env::var("HOME")
            .ok()
            .and_then(|h| std::fs::read_to_string(format!("{h}/.config/kxkbrc")).ok())
            .and_then(|c| c.lines().find_map(|l| l.strip_prefix("LayoutList=").map(|v| v.split(',').next().unwrap_or("us").trim().to_owned())))
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| "us".into());
        let Ok(name) = std::ffi::CString::new(name) else { return keys };
        // SAFETY: plain libxkbcommon calls; the keymap and context live as long as the process.
        unsafe {
            let ctx = xkb_context_new(0);
            if ctx.is_null() {
                return keys;
            }
            let names = RuleNames { rules: std::ptr::null(), model: std::ptr::null(), layout: name.as_ptr(), variant: std::ptr::null(), options: std::ptr::null() };
            let km = xkb_keymap_new_from_names(ctx, &names, 0);
            if km.is_null() {
                return keys;
            }
            for kc in xkb_keymap_min_keycode(km)..=xkb_keymap_max_keycode(km) {
                // xkb keycodes are evdev's plus 8.
                let evdev = kc.saturating_sub(8) as i32;
                for (level, shift) in [(0, false), (1, true)] {
                    let mut syms: *const u32 = std::ptr::null();
                    if xkb_keymap_key_get_syms_by_level(km, kc, 0, level, &mut syms) == 1 {
                        keys.entry(*syms).or_insert((evdev, shift));
                    }
                }
            }
        }
        keys
    })
}

/// One thing the phone asked, in the order it's done.
#[derive(Clone, Debug, PartialEq)]
enum Act {
    Move(f64, f64),
    Scroll(f64, f64),
    Button(i32, bool),
    Key(i32, bool),
    Keysym(i32, bool),
}

/// What a mousepad packet does, as portal calls.
fn acts(b: &Value) -> Vec<Act> {
    acts_in(b, &|sym| layout().get(&sym).copied())
}

/// [acts], with [key] giving the key and Shift that type a keysym.
fn acts_in(b: &Value, key: &dyn Fn(u32) -> Option<(i32, bool)>) -> Vec<Act> {
    let f = |k: &str| b[k].as_f64().unwrap_or(0.0);
    let on = |k: &str| b[k].as_bool() == Some(true);
    let click = |button| vec![Act::Button(button, true), Act::Button(button, false)];
    let mut out = Vec::new();
    if on("singleclick") {
        out = click(BTN_LEFT);
    } else if on("doubleclick") {
        out = [click(BTN_LEFT), click(BTN_LEFT)].concat();
    } else if on("rightclick") {
        out = click(BTN_RIGHT);
    } else if on("middleclick") {
        out = click(BTN_MIDDLE);
    } else if on("singlehold") {
        out.push(Act::Button(BTN_LEFT, true));
    } else if on("singlerelease") {
        out.push(Act::Button(BTN_LEFT, false));
    } else if on("scroll") {
        out.push(Act::Scroll(f("dx"), f("dy")));
    } else if b["key"].is_string() || b["specialKey"].is_i64() {
        let special_key = b["specialKey"].as_i64().and_then(special);
        let text = b["key"].as_str().unwrap_or("");
        // A shortcut (Ctrl+Shift+T) holds what it names. Plain typing gets its Shift from the
        // layout instead: the phone guesses shifted runs from a US keyboard.
        let shortcut = special_key.is_some() || on("ctrl") || on("alt") || on("super");
        let mods: Vec<i32> = [("ctrl", KEY_LEFTCTRL), ("alt", KEY_LEFTALT), ("shift", KEY_LEFTSHIFT), ("super", KEY_LEFTMETA)]
            .into_iter()
            .filter(|(k, _)| on(k) && (shortcut || *k != "shift"))
            .map(|(_, c)| c)
            .collect();
        if special_key.is_none() && text.is_empty() {
            return out;
        }
        out.extend(mods.iter().map(|m| Act::Key(*m, true)));
        if let Some(code) = special_key {
            out.extend([Act::Key(code, true), Act::Key(code, false)]);
        }
        for c in text.chars().take(if special_key.is_some() { 0 } else { usize::MAX }) {
            let sym = keysym(c) as u32;
            // A shortcut's letter is its key as such: Ctrl+C presses C, whatever case it came in.
            let found = key(sym).or_else(|| shortcut.then(|| key(keysym(c.to_ascii_lowercase()) as u32)).flatten());
            match found {
                Some((code, shift)) => {
                    let shift = shift && !shortcut;
                    if shift {
                        out.push(Act::Key(KEY_LEFTSHIFT, true));
                    }
                    out.extend([Act::Key(code, true), Act::Key(code, false)]);
                    if shift {
                        out.push(Act::Key(KEY_LEFTSHIFT, false));
                    }
                }
                None => out.extend([Act::Keysym(sym as i32, true), Act::Keysym(sym as i32, false)]),
            }
        }
        out.extend(mods.iter().rev().map(|m| Act::Key(*m, false)));
    } else if b["dx"].is_number() || b["dy"].is_number() {
        out.push(Act::Move(f("dx"), f("dy")));
    }
    out
}

/// The echo a device asks for with `sendAck`.
fn echo(b: &Value) -> Option<Value> {
    if b["sendAck"].as_bool() != Some(true) {
        return None;
    }
    let mut body = b.clone();
    body["isAck"] = json!(true);
    body.as_object_mut()?.remove("sendAck");
    Some(packet(MOUSEPAD_ECHO, body))
}

pub fn keyboard_state() -> Value {
    packet(MOUSEPAD_KEYBOARDSTATE, json!({ "state": true }))
}

struct Session {
    conn: Connection,
    path: OwnedObjectPath,
}

/// Does what [requests] (when it came, device id, packet) ask, one after another. [out]: echoes.
pub async fn run(dir: PathBuf, mut requests: mpsc::UnboundedReceiver<(Instant, String, Value)>, out: mpsc::UnboundedSender<(String, Value)>) {
    let token = dir.join("remotedesktop-token");
    let mut session: Option<Session> = None;
    // Everything asked while the desktop's dialog was up: the user wasn't watching it happen.
    let mut ready_at = Instant::now();
    // After a refusal, the dialog isn't put up again for every touch.
    let mut refused_at: Option<Instant> = None;
    while let Some((at, id, p)) = requests.recv().await {
        let body = &p["body"];
        if let Some(e) = echo(body) {
            let _ = out.send((id.clone(), e));
        }
        let acts = acts(body);
        if acts.is_empty() {
            continue;
        }
        if session.is_none() {
            if refused_at.is_some_and(|r| r.elapsed().as_secs() < 30) {
                continue;
            }
            match Session::open(&token).await {
                Ok(s) => {
                    tracing::info!("input from devices allowed");
                    session = Some(s);
                    ready_at = Instant::now();
                    refused_at = None;
                }
                Err(e) => {
                    tracing::warn!("no input from devices: {e:#}");
                    refused_at = Some(Instant::now());
                    continue;
                }
            }
        }
        if at < ready_at {
            continue;
        }
        let s = session.as_ref().unwrap();
        tracing::debug!("{id}: {} input call(s), {} ms after arriving", acts.len(), at.elapsed().as_millis());
        let mut moved = false;
        for a in acts {
            let done = match a {
                Act::Move(dx, dy) => {
                    moved = true;
                    s.fast("NotifyPointerMotion", &(&s.path, opts(), dx, dy)).await
                }
                Act::Scroll(dx, dy) => s.fast("NotifyPointerAxis", &(&s.path, opts(), dx, dy)).await,
                Act::Button(b, down) => {
                    // The portal handles calls concurrently: a click mustn't land before its move.
                    if std::mem::take(&mut moved) {
                        let _ = s.call("NotifyPointerMotion", &(&s.path, opts(), 0.0f64, 0.0f64)).await;
                    }
                    s.call("NotifyPointerButton", &(&s.path, opts(), b, down as u32)).await
                }
                Act::Key(code, down) => s.call("NotifyKeyboardKeycode", &(&s.path, opts(), code, down as u32)).await,
                Act::Keysym(sym, down) => s.call("NotifyKeyboardKeysym", &(&s.path, opts(), sym, down as u32)).await,
            };
            if let Err(e) = done {
                // Closed on the desktop's side (the user stopped it): ask again next time.
                tracing::info!("input session ended: {e:#}");
                session = None;
                break;
            }
        }
    }
}

fn opts() -> HashMap<&'static str, Variant<'static>> {
    HashMap::new()
}

impl Session {
    /// A RemoteDesktop session for keyboard and pointer, no screen.
    async fn open(token_file: &std::path::Path) -> Result<Self> {
        let conn = Connection::session().await.context("no session bus")?;
        let sender = conn.unique_name().context("not on the bus")?.trim_start_matches(':').replace('.', "_");
        let created = request(&conn, &sender, "CreateSession", |t| (HashMap::from([("handle_token", Variant::from(t.to_owned())), ("session_handle_token", Variant::from("vyomainput"))]),)).await?;
        let path: OwnedObjectPath = created
            .get("session_handle")
            .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
            .and_then(|s| OwnedObjectPath::try_from(s).ok())
            .context("the portal made no session")?;
        let token = std::fs::read_to_string(token_file).ok().map(|t| t.trim().to_owned()).filter(|t| !t.is_empty());
        let p = path.clone();
        request(&conn, &sender, "SelectDevices", move |t| {
            let mut o = HashMap::from([
                ("handle_token", Variant::from(t.to_owned())),
                ("types", Variant::U32(1 | 2)), // keyboard, pointer
                ("persist_mode", Variant::U32(2)),
            ]);
            if let Some(tok) = token {
                o.insert("restore_token", Variant::from(tok));
            }
            (p, o)
        })
        .await?;
        let p = path.clone();
        let started = request(&conn, &sender, "Start", move |t| (p, "", HashMap::from([("handle_token", Variant::from(t.to_owned()))]))).await?;
        if let Some(tok) = started.get("restore_token").and_then(|v| String::try_from(v.try_clone().ok()?).ok()) {
            let _ = write_private(token_file, tok.as_bytes());
        }
        let devices: u32 = started.get("devices").and_then(|v| u32::try_from(v.try_clone().ok()?).ok()).unwrap_or(0);
        if devices & 3 != 3 {
            bail!("the PC didn't allow keyboard and pointer control");
        }
        Ok(Self { conn, path })
    }

    /// Keys and buttons wait for the portal's answer, one after another, or a release could
    /// overtake its press and leave the key held.
    async fn call<B: serde::Serialize + zbus::zvariant::DynamicType>(&self, method: &str, body: &B) -> Result<()> {
        self.conn.call_method(Some(PORTAL), PORTAL_PATH, Some(REMOTE), method, body).await?;
        Ok(())
    }

    /// Moves and scrolls don't wait: one overtaking another changes nothing the next doesn't fix.
    async fn fast<B: serde::Serialize + zbus::zvariant::DynamicType>(&self, method: &str, body: &B) -> Result<()> {
        let msg = zbus::message::Message::method_call(PORTAL_PATH, method).destination(PORTAL).interface(REMOTE).with_flags(zbus::message::Flags::NO_REPLY_EXPECTED).build(body)?;
        self.conn.send(&msg).await?;
        Ok(())
    }
}

/// A portal request: called, then answered by a Response signal on its own path.
async fn request<B: serde::Serialize + zbus::zvariant::DynamicType>(conn: &Connection, sender: &str, method: &str, body: impl FnOnce(&str) -> B) -> Result<HashMap<String, OwnedVariant>> {
    let token = format!("vyoma{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos()) ^ std::process::id());
    let path = format!("{PORTAL_PATH}/request/{sender}/{token}");
    // Listen before calling, or a fast answer is missed.
    let rule = MatchRule::builder().msg_type(zbus::message::Type::Signal).interface("org.freedesktop.portal.Request").member("Response").path(path.as_str()).build()?;
    let mut responses = MessageStream::for_match_rule(rule, conn, Some(std::num::NonZeroUsize::MIN)).await?;
    conn.call_method(Some(PORTAL), PORTAL_PATH, Some(REMOTE), method, &body(&token)).await.with_context(|| format!("the remote-desktop portal refused {method}"))?;
    let msg = responses.next().await.context("the portal went away")??;
    let (code, results): (u32, HashMap<String, OwnedVariant>) = msg.body().deserialize()?;
    match code {
        0 => Ok(results),
        1 => bail!("cancelled on the PC"),
        _ => bail!("the PC couldn't allow it"),
    }
}

fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let part = path.with_extension("part");
    std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&part)?.write_all(bytes)?;
    std::fs::rename(part, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packets_as_portal_calls() {
        assert_eq!(acts(&json!({ "dx": 3.0, "dy": -1.5 })), [Act::Move(3.0, -1.5)]);
        assert_eq!(acts(&json!({ "scroll": true, "dx": 0.0, "dy": 2.0 })), [Act::Scroll(0.0, 2.0)]);
        assert_eq!(acts(&json!({ "singleclick": true })), [Act::Button(BTN_LEFT, true), Act::Button(BTN_LEFT, false)]);
        assert_eq!(acts(&json!({ "doubleclick": true })).len(), 4);
        assert_eq!(acts(&json!({ "singlehold": true })), [Act::Button(BTN_LEFT, true)]);
        // A few keys of a US layout: a/A on 30, c/C on 46, 1/! on 2.
        let us = |sym: u32| match char::from_u32(sym)? {
            'a' => Some((30, false)),
            'A' => Some((30, true)),
            'c' => Some((46, false)),
            'C' => Some((46, true)),
            '1' => Some((2, false)),
            '!' => Some((2, true)),
            _ => None,
        };
        let t = |b: Value| acts_in(&b, &us);
        let shifted = |code| [Act::Key(KEY_LEFTSHIFT, true), Act::Key(code, true), Act::Key(code, false), Act::Key(KEY_LEFTSHIFT, false)];
        assert_eq!(t(json!({ "key": "a1" })), [Act::Key(30, true), Act::Key(30, false), Act::Key(2, true), Act::Key(2, false)]);
        // The phone's Shift is ignored for typing: the layout says which keys need it.
        assert_eq!(t(json!({ "key": "A!", "shift": true })), [shifted(30), shifted(2)].concat());
        assert_eq!(
            t(json!({ "key": "C", "ctrl": true })),
            [Act::Key(KEY_LEFTCTRL, true), Act::Key(46, true), Act::Key(46, false), Act::Key(KEY_LEFTCTRL, false)],
            "Ctrl+C presses C alone"
        );
        assert_eq!(t(json!({ "key": "c", "ctrl": true, "shift": true }))[1], Act::Key(KEY_LEFTSHIFT, true), "a shortcut's Shift is held");
        assert_eq!(t(json!({ "specialKey": 12 })), [Act::Key(28, true), Act::Key(28, false)]);
        assert_eq!(t(json!({ "specialKey": 32, "alt": true }))[1], Act::Key(88, true), "F12");
        assert!(t(json!({ "specialKey": 99 })).is_empty(), "unknown keys press nothing");
        assert_eq!(t(json!({ "key": "✓" }))[0], Act::Keysym(0x0100_2713, true), "no key types it");
        assert!(acts(&json!({})).is_empty());

        let e = echo(&json!({ "key": "a", "sendAck": true })).unwrap();
        assert_eq!(e["body"], json!({ "key": "a", "isAck": true }));
        assert!(echo(&json!({ "key": "a" })).is_none());
    }
}
