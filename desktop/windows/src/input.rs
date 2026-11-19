//! The phone's touchpad and keyboard (`kdeconnect.mousepad.request`) through SendInput: moves,
//! clicks, holds, scrolls, typing as characters (any language: Unicode key events) and shortcuts
//! on this PC's own keyboard layout (Ctrl+C presses whatever key types c here).
//!
//! Windows refuses input into windows of programs running as administrator (UIPI) and the secure
//! desktop (UAC prompts, the lock screen); there SendInput lands nowhere, as with any normal app.
//! Built and tested on CI and under Wine only; not yet run on a real Windows PC.

use serde_json::{Value, json};
use vyoma_core::link::packet;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
    MOUSE_EVENT_FLAGS, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE,
    MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEINPUT, SendInput, VkKeyScanW,
};

pub const MOUSEPAD_REQUEST: &str = "kdeconnect.mousepad.request";
pub const MOUSEPAD_ECHO: &str = "kdeconnect.mousepad.echo";

/// Virtual keys.
pub const SHIFT: u16 = 0x10;
pub const CONTROL: u16 = 0x11;
pub const ALT: u16 = 0x12;
pub const WIN: u16 = 0x5B;
const BACK: u16 = 0x08;
pub const TAB: u16 = 0x09;
const RETURN: u16 = 0x0D;
const ESCAPE: u16 = 0x1B;
const PRIOR: u16 = 0x21;
const NEXT: u16 = 0x22;
const END: u16 = 0x23;
const HOME: u16 = 0x24;
pub const LEFT: u16 = 0x25;
const UP: u16 = 0x26;
pub const RIGHT: u16 = 0x27;
const DOWN: u16 = 0x28;
const DELETE: u16 = 0x2E;
const F1: u16 = 0x70;

/// Pixels of the phone's scroll per wheel unit (120 units a notch, about three lines).
const SCROLL_SCALE: f64 = 2.4;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

/// One thing the phone asked, in the order it's done.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    Move(f64, f64),
    /// Positive: down and right, as a wheel turned towards you.
    Scroll(f64, f64),
    Button(Button, bool),
    /// A key (virtual key) pressed with the modifiers held.
    Key(u16, Vec<u16>),
    /// Characters typed as such.
    Text(String),
    /// The pointer to a place on the cast display, 0-1 across and down (the live screen's touches).
    At(f64, f64),
}

/// A display's place among all of them (pixels: left, top, width, height), and all of them together.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Area {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
}

impl Area {
    /// All the displays, as Windows lays them out.
    pub fn everything() -> Area {
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN};
        // SAFETY: GetSystemMetrics takes no pointers.
        unsafe { Area { left: GetSystemMetrics(SM_XVIRTUALSCREEN), top: GetSystemMetrics(SM_YVIRTUALSCREEN), width: GetSystemMetrics(SM_CXVIRTUALSCREEN), height: GetSystemMetrics(SM_CYVIRTUALSCREEN) } }
    }
}

/// SendInput's absolute coordinates (0-65535 across all displays) for ([x], [y]) on [display].
fn absolute(x: f64, y: f64, display: Area, all: Area) -> (i32, i32) {
    let px = display.left as f64 + x.clamp(0.0, 1.0) * (display.width - 1).max(0) as f64;
    let py = display.top as f64 + y.clamp(0.0, 1.0) * (display.height - 1).max(0) as f64;
    let ax = (px - all.left as f64) * 65535.0 / (all.width - 1).max(1) as f64;
    let ay = (py - all.top as f64) * 65535.0 / (all.height - 1).max(1) as f64;
    (ax.round() as i32, ay.round() as i32)
}

/// The live screen's touches (`vyoma.screen.input`/`touch`) as acts on the cast display: taps and
/// drags with the left button, a long press's right click, scrolls there, typing and keys (the
/// phone's names; its Super is the Start menu).
pub fn touch_acts(i: &Value) -> Vec<Act> {
    let at = || Act::At(i["x"].as_f64().unwrap_or(0.0).clamp(0.0, 1.0), i["y"].as_f64().unwrap_or(0.0).clamp(0.0, 1.0));
    match i["kind"].as_str().unwrap_or("") {
        "click" => vec![at(), Act::Button(Button::Left, true), Act::Button(Button::Left, false)],
        "right" => vec![at(), Act::Button(Button::Right, true), Act::Button(Button::Right, false)],
        "down" => vec![at(), Act::Button(Button::Left, true)],
        "move" => vec![at()],
        "up" => vec![at(), Act::Button(Button::Left, false)],
        "scroll" => vec![at(), Act::Scroll(i["dx"].as_f64().unwrap_or(0.0), i["dy"].as_f64().unwrap_or(0.0))],
        "text" => {
            let text: String = i["text"].as_str().unwrap_or("").chars().take(4096).collect();
            acts(&json!({ "key": text }))
        }
        "key" => {
            let mods: Vec<u16> = i["mods"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|m| match m.as_str()? {
                    "ctrl" => Some(CONTROL),
                    "alt" => Some(ALT),
                    "super" => Some(WIN),
                    "shift" => Some(SHIFT),
                    _ => None,
                })
                .collect();
            let code = match i["key"].as_str().unwrap_or("") {
                "BackSpace" => BACK,
                "Tab" => TAB,
                "Return" => RETURN,
                "Escape" => ESCAPE,
                "Delete" => DELETE,
                "Home" => HOME,
                "End" => END,
                "Left" => LEFT,
                "Right" => RIGHT,
                "Up" => UP,
                "Down" => DOWN,
                "PageUp" => PRIOR,
                "PageDown" => NEXT,
                "Super" => WIN,
                one if one.chars().count() == 1 => match one.chars().next().and_then(key_of) {
                    Some(code) => code,
                    None => return Vec::new(),
                },
                _ => return Vec::new(),
            };
            vec![Act::Key(code, mods)]
        }
        _ => Vec::new(),
    }
}

/// KDE Connect's special key numbers as virtual keys.
fn special(n: i64) -> Option<u16> {
    Some(match n {
        1 => BACK,
        2 => TAB,
        3 | 12 => RETURN,
        4 => LEFT,
        5 => UP,
        6 => RIGHT,
        7 => DOWN,
        8 => PRIOR,
        9 => NEXT,
        10 => HOME,
        11 => END,
        13 => DELETE,
        14 => ESCAPE,
        21..=32 => F1 + (n - 21) as u16,
        _ => return None,
    })
}

/// The key that types [c] on this PC's layout, for shortcuts; letters and digits fall back to
/// their own virtual keys (the same on every layout's positions Windows names them by).
fn key_of(c: char) -> Option<u16> {
    let mut units = [0u16; 2];
    if c.encode_utf16(&mut units).len() == 1 {
        // SAFETY: a plain lookup; -1 means no key types it.
        let k = unsafe { VkKeyScanW(units[0]) };
        if k != -1 {
            return Some((k as u16) & 0xff);
        }
    }
    let up = c.to_ascii_uppercase();
    up.is_ascii_alphanumeric().then_some(up as u16)
}

/// What a mousepad packet does. The phone's Ctrl is Ctrl here, its Super the Windows key.
pub fn acts(b: &Value) -> Vec<Act> {
    let f = |k: &str| b[k].as_f64().unwrap_or(0.0);
    let on = |k: &str| b[k].as_bool() == Some(true);
    let click = |button| vec![Act::Button(button, true), Act::Button(button, false)];
    if on("singleclick") {
        return click(Button::Left);
    }
    if on("doubleclick") {
        // Two clicks close together: Windows makes them a double click by itself.
        return [click(Button::Left), click(Button::Left)].concat();
    }
    if on("rightclick") {
        return click(Button::Right);
    }
    if on("middleclick") {
        return click(Button::Middle);
    }
    if on("singlehold") {
        return vec![Act::Button(Button::Left, true)];
    }
    if on("singlerelease") {
        return vec![Act::Button(Button::Left, false)];
    }
    if on("scroll") {
        return vec![Act::Scroll(f("dx"), f("dy"))];
    }
    if b["key"].is_string() || b["specialKey"].is_i64() {
        let text = b["key"].as_str().unwrap_or("");
        let mods: Vec<u16> = [("ctrl", CONTROL), ("alt", ALT), ("super", WIN), ("shift", SHIFT)].into_iter().filter(|(k, _)| on(k)).map(|(_, v)| v).collect();
        if let Some(n) = b["specialKey"].as_i64() {
            return special(n).map(|code| vec![Act::Key(code, mods)]).unwrap_or_default();
        }
        // A shortcut holds what it names; plain typing (Shift alone included) is its characters.
        if mods.iter().any(|&m| m != SHIFT) {
            return text.chars().filter_map(key_of).map(|code| Act::Key(code, mods.clone())).collect();
        }
        let mut out = Vec::new();
        let mut run = String::new();
        for c in text.chars() {
            match c {
                '\n' | '\r' | '\t' => {
                    if !run.is_empty() {
                        out.push(Act::Text(std::mem::take(&mut run)));
                    }
                    out.push(Act::Key(if c == '\t' { TAB } else { RETURN }, Vec::new()));
                }
                _ => run.push(c),
            }
        }
        if !run.is_empty() {
            out.push(Act::Text(run));
        }
        return out;
    }
    if b["dx"].is_number() || b["dy"].is_number() {
        return vec![Act::Move(f("dx"), f("dy"))];
    }
    Vec::new()
}

/// The echo a device asks for with `sendAck`.
pub fn echo(b: &Value) -> Option<Value> {
    if b["sendAck"].as_bool() != Some(true) {
        return None;
    }
    let mut body = b.clone();
    body["isAck"] = json!(true);
    body.as_object_mut()?.remove("sendAck");
    Some(packet(MOUSEPAD_ECHO, body))
}

/// Keys Windows wants marked extended: on the arrow/navigation block, not the number pad.
fn extended(vk: u16) -> bool {
    matches!(vk, PRIOR | NEXT | END | HOME | LEFT | UP | RIGHT | DOWN | DELETE | WIN | 0x2D | 0x5C | 0x5D)
}

fn mouse(flags: MOUSE_EVENT_FLAGS, dx: i32, dy: i32, data: i32) -> INPUT {
    INPUT { r#type: INPUT_MOUSE, Anonymous: INPUT_0 { mi: MOUSEINPUT { dx, dy, mouseData: data as u32, dwFlags: flags, time: 0, dwExtraInfo: 0 } } }
}

fn key(vk: u16, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 } } }
}

fn vk_flags(vk: u16, up: bool) -> KEYBD_EVENT_FLAGS {
    (if extended(vk) { KEYEVENTF_EXTENDEDKEY } else { 0 }) | (if up { KEYEVENTF_KEYUP } else { 0 })
}

/// The events for [act]; [rest] keeps the fractions of moves and scrolls between packets.
fn events(act: &Act, rest: &mut [f64; 4], display: Area) -> Vec<INPUT> {
    let mut whole = |i: usize, v: f64| {
        rest[i] += v;
        let w = rest[i].trunc();
        rest[i] -= w;
        w as i32
    };
    match act {
        Act::Move(dx, dy) => {
            let (x, y) = (whole(0, *dx), whole(1, *dy));
            if x == 0 && y == 0 { Vec::new() } else { vec![mouse(MOUSEEVENTF_MOVE, x, y, 0)] }
        }
        Act::Scroll(dx, dy) => {
            // Wheel: positive is away from you (up), so the phone's down is negative.
            let (v, h) = (whole(2, -dy * SCROLL_SCALE), whole(3, dx * SCROLL_SCALE));
            let mut out = Vec::new();
            if v != 0 {
                out.push(mouse(MOUSEEVENTF_WHEEL, 0, 0, v));
            }
            if h != 0 {
                out.push(mouse(MOUSEEVENTF_HWHEEL, 0, 0, h));
            }
            out
        }
        Act::Button(b, down) => {
            let flags = match (b, down) {
                (Button::Left, true) => MOUSEEVENTF_LEFTDOWN,
                (Button::Left, false) => MOUSEEVENTF_LEFTUP,
                (Button::Right, true) => MOUSEEVENTF_RIGHTDOWN,
                (Button::Right, false) => MOUSEEVENTF_RIGHTUP,
                (Button::Middle, true) => MOUSEEVENTF_MIDDLEDOWN,
                (Button::Middle, false) => MOUSEEVENTF_MIDDLEUP,
            };
            vec![mouse(flags, 0, 0, 0)]
        }
        Act::Key(vk, mods) => {
            let mut out: Vec<INPUT> = mods.iter().map(|&m| key(m, 0, vk_flags(m, false))).collect();
            out.push(key(*vk, 0, vk_flags(*vk, false)));
            out.push(key(*vk, 0, vk_flags(*vk, true)));
            out.extend(mods.iter().rev().map(|&m| key(m, 0, vk_flags(m, true))));
            out
        }
        Act::At(x, y) => {
            let (ax, ay) = absolute(*x, *y, display, Area::everything());
            vec![mouse(MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK, ax, ay, 0)]
        }
        Act::Text(text) => text
            .encode_utf16()
            .flat_map(|u| [key(0, u, KEYEVENTF_UNICODE), key(0, u, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP)])
            .collect(),
    }
}

/// Sends the phone's acts; the fractions of moves and scrolls carry over to the next packet.
#[derive(Default)]
pub struct Pointer {
    rest: [f64; 4],
    /// The display [Act::At] is on (the cast one).
    display: Area,
}

impl Pointer {
    /// Places from the live screen land on [display].
    pub fn cast_on(&mut self, display: Area) {
        self.display = display;
    }

    /// Err when Windows took none of it (the secure desktop, or an administrator's window in front).
    pub fn handle(&mut self, acts: &[Act]) -> anyhow::Result<()> {
        let all: Vec<INPUT> = acts.iter().flat_map(|a| events(a, &mut self.rest, self.display)).collect();
        send(&all)
    }
}

/// Presses [vk] with [mods] (the desktop gestures: Task View, the desktops either side).
pub fn press(vk: u16, mods: &[u16]) -> anyhow::Result<()> {
    send(&events(&Act::Key(vk, mods.to_vec()), &mut [0.0; 4], Area::default()))
}

fn send(all: &[INPUT]) -> anyhow::Result<()> {
    if all.is_empty() {
        return Ok(());
    }
    // SAFETY: an array of filled-in INPUTs and its length.
    let sent = unsafe { SendInput(all.len() as u32, all.as_ptr(), size_of::<INPUT>() as i32) };
    if sent == 0 {
        anyhow::bail!("Windows didn't take the input: {}", std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_phone_as_the_mac_does() {
        assert_eq!(acts(&json!({ "dx": 3.0, "dy": -2.0 })), [Act::Move(3.0, -2.0)]);
        assert_eq!(acts(&json!({ "singleclick": true })), [Act::Button(Button::Left, true), Act::Button(Button::Left, false)]);
        assert_eq!(acts(&json!({ "doubleclick": true })).len(), 4);
        assert_eq!(acts(&json!({ "scroll": true, "dx": 0.0, "dy": 12.0 })), [Act::Scroll(0.0, 12.0)]);
        assert_eq!(acts(&json!({ "specialKey": 1 })), [Act::Key(BACK, vec![])]);
        assert_eq!(acts(&json!({ "specialKey": 32 })), [Act::Key(F1 + 11, vec![])], "F12");
        assert_eq!(acts(&json!({ "key": "hi\nyo" })), [Act::Text("hi".into()), Act::Key(RETURN, vec![]), Act::Text("yo".into())]);
        assert_eq!(acts(&json!({ "key": "\u{263a}" })), [Act::Text("\u{263a}".into())]);
        assert_eq!(acts(&json!({ "key": "A", "shift": true })), [Act::Text("A".into())], "Shift alone is typing");
        assert!(acts(&json!({ "nothing": 1 })).is_empty());
    }

    #[test]
    fn shortcuts_hold_their_modifiers_around_the_key() {
        let shortcut = acts(&json!({ "key": "c", "ctrl": true }));
        assert_eq!(shortcut.len(), 1);
        let Act::Key(_, mods) = &shortcut[0] else { panic!("{shortcut:?}") };
        assert_eq!(mods, &[CONTROL]);
        let ev = events(&shortcut[0], &mut [0.0; 4], Area::default());
        // SAFETY: keyboard inputs, made above.
        let keys: Vec<(u16, bool)> = ev.iter().map(|i| unsafe { (i.Anonymous.ki.wVk, i.Anonymous.ki.dwFlags & KEYEVENTF_KEYUP != 0) }).collect();
        assert_eq!(keys[0], (CONTROL, false));
        assert_eq!(keys[3], (CONTROL, true));
        assert!(!keys[1].1 && keys[2].1, "the key goes down, then up");
    }

    #[test]
    fn fractions_add_up_and_wheels_turn_the_right_way() {
        let mut rest = [0.0; 4];
        assert!(events(&Act::Move(0.4, 0.0), &mut rest, Area::default()).is_empty());
        let ev = events(&Act::Move(0.7, 0.0), &mut rest, Area::default());
        // SAFETY: mouse inputs, made above.
        assert_eq!(unsafe { ev[0].Anonymous.mi.dx }, 1);
        let ev = events(&Act::Scroll(0.0, 50.0), &mut rest, Area::default());
        // SAFETY: as above.
        assert_eq!(unsafe { ev[0].Anonymous.mi.mouseData } as i32, -120, "a notch down");
    }

    #[test]
    fn typing_is_unicode_key_events() {
        let ev = events(&Act::Text("a\u{1f600}".into()), &mut [0.0; 4], Area::default());
        // 'a' is one unit, the emoji two (a surrogate pair): each pressed and let go.
        assert_eq!(ev.len(), 6);
        // SAFETY: keyboard inputs, made above.
        assert!(ev.iter().all(|i| unsafe { i.Anonymous.ki.dwFlags } & KEYEVENTF_UNICODE != 0));
    }

    #[test]
    fn touches_land_on_the_cast_display() {
        // Two displays side by side, the cast one on the right.
        let all = Area { left: 0, top: 0, width: 3840, height: 1080 };
        let right = Area { left: 1920, top: 0, width: 1920, height: 1080 };
        assert_eq!(absolute(0.0, 0.0, right, all), (32776, 0));
        assert_eq!(absolute(1.0, 1.0, right, all), (65535, 65535));
        assert_eq!(absolute(9.0, -1.0, right, all), (65535, 0), "clamped to the display");
        // A display left of the main one has negative coordinates.
        let left = Area { left: -1280, top: 0, width: 1280, height: 1024 };
        assert_eq!(absolute(0.0, 0.0, left, Area { left: -1280, top: 0, width: 3200, height: 1080 }).0, 0);
        let tap = touch_acts(&json!({ "kind": "click", "x": 0.5, "y": 0.25 }));
        assert_eq!(tap, [Act::At(0.5, 0.25), Act::Button(Button::Left, true), Act::Button(Button::Left, false)]);
        assert_eq!(touch_acts(&json!({ "kind": "key", "key": "Return", "mods": ["ctrl"] })), [Act::Key(RETURN, vec![CONTROL])]);
        assert_eq!(touch_acts(&json!({ "kind": "key", "key": "Super" })), [Act::Key(WIN, vec![])], "the Start menu");
        assert!(touch_acts(&json!({ "kind": "launch" })).is_empty());
    }
}
