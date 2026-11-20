//! The phone's touchpad and keyboard (`kdeconnect.mousepad.request`) and the live screen's touches
//! (`vyoma.screen.input`/`touch`), read once for every PC side: what each packet asks, in the order
//! it's done, in the phone's own terms. Each side turns these into its own events (SendInput,
//! CGEvents, the portal or X11 or the wlr protocols), with its own key codes and its own reading
//! of the phone's modifiers (on the Mac its Ctrl is Command).

use serde_json::{Value, json};
use vyoma_core::link::packet;

pub const MOUSEPAD_REQUEST: &str = "kdeconnect.mousepad.request";
pub const MOUSEPAD_ECHO: &str = "kdeconnect.mousepad.echo";
pub const MOUSEPAD_KEYBOARDSTATE: &str = "kdeconnect.mousepad.keyboardstate";

/// The most characters one live-screen touch types.
const TEXT_MAX: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

/// A key that types nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Backspace,
    Tab,
    Return,
    Left,
    Up,
    Right,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    Delete,
    Escape,
    Print,
    ScrollLock,
    /// F1 to F12.
    F(u8),
    /// The Windows key, Command on a Mac keyboard: on its own, the Start menu or the overview.
    Super,
}

/// A modifier as the phone names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mod {
    Ctrl,
    Alt,
    Super,
    Shift,
}

/// One thing the phone asked.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    Move(f64, f64),
    /// Positive: down and right, as a wheel turned towards you.
    Scroll(f64, f64),
    Click(Button),
    /// Two clicks that make a double click.
    DoubleClick,
    /// A button held (true) or let go, for a drag.
    Button(Button, bool),
    /// [Key] pressed with the modifiers held.
    Key(Key, Vec<Mod>),
    /// The key that types this character, pressed with the modifiers held: a shortcut (Ctrl+C),
    /// where the key matters, not what it types.
    Shortcut(char, Vec<Mod>),
    /// Characters typed as such (no line breaks or tabs: those come as [Key::Return] and [Key::Tab]).
    Text(String),
    /// The pointer to a place on the cast display, 0-1 across and down (the live screen's touches).
    At(f64, f64),
}

/// KDE Connect's special key numbers.
fn special(n: i64) -> Option<Key> {
    Some(match n {
        1 => Key::Backspace,
        2 => Key::Tab,
        // Linefeed and Return.
        3 | 12 => Key::Return,
        4 => Key::Left,
        5 => Key::Up,
        6 => Key::Right,
        7 => Key::Down,
        8 => Key::PageUp,
        9 => Key::PageDown,
        10 => Key::Home,
        11 => Key::End,
        13 => Key::Delete,
        14 => Key::Escape,
        15 => Key::Print,
        16 => Key::ScrollLock,
        21..=32 => Key::F((n - 20) as u8),
        _ => return None,
    })
}

/// The live screen's names for keys.
fn named(name: &str) -> Option<Key> {
    Some(match name {
        "BackSpace" => Key::Backspace,
        "Tab" => Key::Tab,
        "Return" => Key::Return,
        "Escape" => Key::Escape,
        "Delete" => Key::Delete,
        "Home" => Key::Home,
        "End" => Key::End,
        "Left" => Key::Left,
        "Right" => Key::Right,
        "Up" => Key::Up,
        "Down" => Key::Down,
        "PageUp" => Key::PageUp,
        "PageDown" => Key::PageDown,
        "Super" => Key::Super,
        _ => return None,
    })
}

const MODS: [(&str, Mod); 4] = [("ctrl", Mod::Ctrl), ("alt", Mod::Alt), ("super", Mod::Super), ("shift", Mod::Shift)];

/// Plain typing: runs of characters, with line breaks and tabs as their keys.
fn typing(text: &str) -> Vec<Act> {
    let mut out = Vec::new();
    let mut run = String::new();
    for c in text.chars() {
        match c {
            '\n' | '\r' | '\t' => {
                if !run.is_empty() {
                    out.push(Act::Text(std::mem::take(&mut run)));
                }
                out.push(Act::Key(if c == '\t' { Key::Tab } else { Key::Return }, Vec::new()));
            }
            _ => run.push(c),
        }
    }
    if !run.is_empty() {
        out.push(Act::Text(run));
    }
    out
}

/// What a mousepad packet's body asks.
pub fn acts(b: &Value) -> Vec<Act> {
    let f = |k: &str| b[k].as_f64().unwrap_or(0.0);
    let on = |k: &str| b[k].as_bool() == Some(true);
    if on("singleclick") {
        return vec![Act::Click(Button::Left)];
    }
    if on("doubleclick") {
        return vec![Act::DoubleClick];
    }
    if on("rightclick") {
        return vec![Act::Click(Button::Right)];
    }
    if on("middleclick") {
        return vec![Act::Click(Button::Middle)];
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
        let mods: Vec<Mod> = MODS.into_iter().filter(|(k, _)| on(k)).map(|(_, m)| m).collect();
        if let Some(n) = b["specialKey"].as_i64() {
            return special(n).map(|key| vec![Act::Key(key, mods)]).unwrap_or_default();
        }
        // A shortcut holds what it names; plain typing (Shift alone included) is its characters.
        if mods.iter().any(|&m| m != Mod::Shift) {
            return text.chars().map(|c| Act::Shortcut(c, mods.clone())).collect();
        }
        return typing(text);
    }
    if b["dx"].is_number() || b["dy"].is_number() {
        return vec![Act::Move(f("dx"), f("dy"))];
    }
    Vec::new()
}

/// A touch on the live screen (`vyoma.screen.input`, a line of the touch stream): x, y 0..1 across
/// the picture. Kinds: "click", "right", "down" / "move" / "up" (a left-button drag), "scroll" (dx,
/// dy in pixels, positive down); typing as "text" and "key" (BackSpace, Return, Escape, Tab,
/// Delete, Home, End, arrows, PageUp/PageDown, Super, or one character, with `mods`).
pub fn touch_acts(i: &Value) -> Vec<Act> {
    let at = || Act::At(i["x"].as_f64().unwrap_or(0.0).clamp(0.0, 1.0), i["y"].as_f64().unwrap_or(0.0).clamp(0.0, 1.0));
    match i["kind"].as_str().unwrap_or("") {
        "click" => vec![at(), Act::Click(Button::Left)],
        "right" => vec![at(), Act::Click(Button::Right)],
        "down" => vec![at(), Act::Button(Button::Left, true)],
        "move" => vec![at()],
        "up" => vec![at(), Act::Button(Button::Left, false)],
        "scroll" => vec![at(), Act::Scroll(i["dx"].as_f64().unwrap_or(0.0), i["dy"].as_f64().unwrap_or(0.0))],
        "text" => {
            let text: String = i["text"].as_str().unwrap_or("").chars().take(TEXT_MAX).collect();
            acts(&json!({ "key": text }))
        }
        "key" => {
            let mods: Vec<Mod> = i["mods"].as_array().into_iter().flatten().filter_map(|m| MODS.iter().find(|(k, _)| Some(*k) == m.as_str()).map(|(_, m)| *m)).collect();
            let name = i["key"].as_str().unwrap_or("");
            if let Some(key) = named(name) {
                return vec![Act::Key(key, mods)];
            }
            let mut chars = name.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => vec![Act::Shortcut(c, mods)],
                _ => Vec::new(),
            }
        }
        _ => Vec::new(),
    }
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

/// That this PC takes typing (the phone's keyboard button).
pub fn keyboard_state() -> Value {
    packet(MOUSEPAD_KEYBOARDSTATE, json!({ "state": true }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_touchpad() {
        assert_eq!(acts(&json!({ "dx": 3.0, "dy": -2.0 })), [Act::Move(3.0, -2.0)]);
        assert_eq!(acts(&json!({ "singleclick": true })), [Act::Click(Button::Left)]);
        assert_eq!(acts(&json!({ "doubleclick": true })), [Act::DoubleClick]);
        assert_eq!(acts(&json!({ "rightclick": true })), [Act::Click(Button::Right)]);
        assert_eq!(acts(&json!({ "singlehold": true })), [Act::Button(Button::Left, true)]);
        assert_eq!(acts(&json!({ "singlerelease": true })), [Act::Button(Button::Left, false)]);
        assert_eq!(acts(&json!({ "scroll": true, "dx": 0.0, "dy": 12.0 })), [Act::Scroll(0.0, 12.0)]);
        assert!(acts(&json!({})).is_empty());
    }

    #[test]
    fn typing_is_characters_and_shortcuts_are_keys() {
        assert_eq!(acts(&json!({ "key": "Hé!" })), [Act::Text("Hé!".into())]);
        assert_eq!(acts(&json!({ "key": "A", "shift": true })), [Act::Text("A".into())], "Shift alone is just typing");
        assert_eq!(acts(&json!({ "key": "hi\nyo\t" })), [Act::Text("hi".into()), Act::Key(Key::Return, vec![]), Act::Text("yo".into()), Act::Key(Key::Tab, vec![])]);
        assert_eq!(acts(&json!({ "key": "T", "ctrl": true, "shift": true })), [Act::Shortcut('T', vec![Mod::Ctrl, Mod::Shift])]);
        assert_eq!(acts(&json!({ "specialKey": 12 })), [Act::Key(Key::Return, vec![])]);
        assert_eq!(acts(&json!({ "specialKey": 4, "shift": true })), [Act::Key(Key::Left, vec![Mod::Shift])], "Shift+Left selects");
        assert_eq!(acts(&json!({ "specialKey": 21 })), [Act::Key(Key::F(1), vec![])]);
        assert_eq!(acts(&json!({ "specialKey": 32 })), [Act::Key(Key::F(12), vec![])]);
        assert_eq!(acts(&json!({ "specialKey": 1, "key": "ignored" })), [Act::Key(Key::Backspace, vec![])], "a special key types nothing else");
        assert!(acts(&json!({ "specialKey": 99 })).is_empty(), "unknown keys press nothing");
        assert!(acts(&json!({ "key": "", "ctrl": true })).is_empty());
    }

    #[test]
    fn touches_on_the_cast_screen() {
        assert_eq!(touch_acts(&json!({ "kind": "click", "x": 0.5, "y": 2.0 })), [Act::At(0.5, 1.0), Act::Click(Button::Left)]);
        assert_eq!(touch_acts(&json!({ "kind": "down", "x": 0.1, "y": 0.1 }))[1], Act::Button(Button::Left, true));
        assert_eq!(touch_acts(&json!({ "kind": "key", "key": "Return", "mods": ["ctrl"] })), [Act::Key(Key::Return, vec![Mod::Ctrl])]);
        assert_eq!(touch_acts(&json!({ "kind": "key", "key": "Super" })), [Act::Key(Key::Super, vec![])]);
        assert_eq!(touch_acts(&json!({ "kind": "key", "key": "z", "mods": ["super", "nonsense"] })), [Act::Shortcut('z', vec![Mod::Super])]);
        assert_eq!(touch_acts(&json!({ "kind": "text", "text": "ok\n" })), [Act::Text("ok".into()), Act::Key(Key::Return, vec![])]);
        assert!(touch_acts(&json!({ "kind": "key", "key": "Hyper" })).is_empty());
        assert!(touch_acts(&json!({ "kind": "launch" })).is_empty());
    }

    #[test]
    fn echoes_only_when_asked() {
        assert!(echo(&json!({ "dx": 1 })).is_none());
        let e = echo(&json!({ "key": "a", "sendAck": true })).unwrap();
        assert_eq!((e["type"].as_str(), &e["body"]), (Some(MOUSEPAD_ECHO), &json!({ "key": "a", "isAck": true })));
    }
}
