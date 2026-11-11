//! The phone as this Mac's touchpad and keyboard (`kdeconnect.mousepad.request`), through
//! CoreGraphics events. macOS lets an app do that only once the person allows it in System
//! Settings > Privacy & Security > Accessibility: until then the events go nowhere, so the first
//! request asks macOS to show that Allow ([allowed]).
//!
//! Typing goes as the characters themselves (any language, any keyboard layout); shortcuts press
//! the keys they name on a US layout, as a Mac's own shortcuts are named. The phone's Ctrl is
//! the Mac's Command (so Ctrl+C copies, as on the PC), Alt is Option and Super is Control.
//! Built on CI only; not yet tried on a real Mac.

// Only macOS does anything with most of this; the tests run everywhere.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use serde_json::{Value, json};
use vyoma_core::link::packet;

pub const MOUSEPAD_REQUEST: &str = "kdeconnect.mousepad.request";
pub const MOUSEPAD_ECHO: &str = "kdeconnect.mousepad.echo";

/// CGEventFlags.
pub const SHIFT: u64 = 0x2_0000;
pub const CONTROL: u64 = 0x4_0000;
pub const OPTION: u64 = 0x8_0000;
pub const COMMAND: u64 = 0x10_0000;

const RETURN: u16 = 36;
const TAB: u16 = 48;

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
    /// A second click right after a first, which macOS reads as a double click.
    Double,
    /// A key by its US-layout virtual code, with the flags held.
    Key(u16, u64),
    /// Characters typed as such.
    Text(String),
}

/// KDE Connect's special key numbers as Mac virtual key codes.
fn special(n: i64) -> Option<u16> {
    Some(match n {
        1 => 51,       // Backspace (the Mac's Delete)
        2 => TAB,      // Tab
        3 | 12 => RETURN, // Linefeed, Return
        4 => 123,      // Left
        5 => 126,      // Up
        6 => 124,      // Right
        7 => 125,      // Down
        8 => 116,      // Page Up
        9 => 121,      // Page Down
        10 => 115,     // Home
        11 => 119,     // End
        13 => 117,     // Delete (forward)
        14 => 53,      // Escape
        21 => 122,     // F1
        22 => 120,
        23 => 99,
        24 => 118,
        25 => 96,
        26 => 97,
        27 => 98,
        28 => 100,
        29 => 101,
        30 => 109,     // F10
        31 => 103,     // F11
        32 => 111,     // F12
        _ => return None,
    })
}

/// The US-layout key for [c], for shortcuts.
fn key_of(c: char) -> Option<u16> {
    const KEYS: &[(char, u16)] = &[
        ('a', 0), ('s', 1), ('d', 2), ('f', 3), ('h', 4), ('g', 5), ('z', 6), ('x', 7), ('c', 8), ('v', 9), ('b', 11), ('q', 12),
        ('w', 13), ('e', 14), ('r', 15), ('y', 16), ('t', 17), ('1', 18), ('2', 19), ('3', 20), ('4', 21), ('6', 22), ('5', 23),
        ('=', 24), ('9', 25), ('7', 26), ('-', 27), ('8', 28), ('0', 29), (']', 30), ('o', 31), ('u', 32), ('[', 33), ('i', 34),
        ('p', 35), ('l', 37), ('j', 38), ('\'', 39), ('k', 40), (';', 41), ('\\', 42), (',', 43), ('/', 44), ('n', 45), ('m', 46),
        ('.', 47), ('`', 50), (' ', 49), ('\n', RETURN), ('\t', TAB),
    ];
    let c = c.to_ascii_lowercase();
    KEYS.iter().find(|(k, _)| *k == c).map(|(_, code)| *code)
}

/// What a mousepad packet does.
pub fn acts(b: &Value) -> Vec<Act> {
    let f = |k: &str| b[k].as_f64().unwrap_or(0.0);
    let on = |k: &str| b[k].as_bool() == Some(true);
    let click = |button| vec![Act::Button(button, true), Act::Button(button, false)];
    if on("singleclick") {
        return click(Button::Left);
    }
    if on("doubleclick") {
        return [click(Button::Left), vec![Act::Double]].concat();
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
        let flags = [("ctrl", COMMAND), ("alt", OPTION), ("super", CONTROL), ("shift", SHIFT)].into_iter().filter(|(k, _)| on(k)).fold(0, |all, (_, f)| all | f);
        if let Some(n) = b["specialKey"].as_i64() {
            return special(n).map(|code| vec![Act::Key(code, flags)]).unwrap_or_default();
        }
        // A shortcut holds what it names; plain typing (Shift alone included) is its characters.
        if flags & !SHIFT != 0 {
            return text.chars().filter_map(key_of).map(|code| Act::Key(code, flags)).collect();
        }
        let mut out = Vec::new();
        let mut run = String::new();
        for c in text.chars() {
            match c {
                '\n' | '\r' | '\t' => {
                    if !run.is_empty() {
                        out.push(Act::Text(std::mem::take(&mut run)));
                    }
                    out.push(Act::Key(if c == '\t' { TAB } else { RETURN }, 0));
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

#[cfg(target_os = "macos")]
pub use mac::Pointer;

/// Elsewhere (this crate's tests on Linux) nothing moves.
#[cfg(not(target_os = "macos"))]
#[derive(Default)]
pub struct Pointer(());

#[cfg(not(target_os = "macos"))]
impl Pointer {
    pub fn handle(&mut self, _: &[Act]) -> Result<(), NotAllowed> {
        Ok(())
    }
}

/// macOS hasn't let Vyoma control this Mac yet; said once, the first time.
#[derive(Debug)]
pub struct NotAllowed;

#[cfg(target_os = "macos")]
mod mac {
    use super::{Act, Button};
    use std::ffi::{c_ulong, c_void};

    type Ref = *mut c_void;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Point {
        x: f64,
        y: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Rect {
        origin: Point,
        size: Point,
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventCreate(source: Ref) -> Ref;
        fn CGEventGetLocation(event: Ref) -> Point;
        fn CGEventCreateMouseEvent(source: Ref, kind: u32, at: Point, button: u32) -> Ref;
        fn CGEventCreateKeyboardEvent(source: Ref, key: u16, down: bool) -> Ref;
        fn CGEventKeyboardSetUnicodeString(event: Ref, length: c_ulong, text: *const u16);
        fn CGEventSetFlags(event: Ref, flags: u64);
        fn CGEventSetIntegerValueField(event: Ref, field: u32, value: i64);
        fn CGEventCreateScrollWheelEvent(source: Ref, units: u32, count: u32, wheel1: i32, ...) -> Ref;
        fn CGEventPost(tap: u32, event: Ref);
        fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
        fn CGDisplayBounds(display: u32) -> Rect;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(object: *const c_void);
        fn CFDictionaryCreate(alloc: Ref, keys: *const *const c_void, values: *const *const c_void, count: isize, key_callbacks: *const c_void, value_callbacks: *const c_void) -> Ref;
        static kCFBooleanTrue: *const c_void;
        static kCFTypeDictionaryKeyCallBacks: u8;
        static kCFTypeDictionaryValueCallBacks: u8;
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> bool;
        fn AXIsProcessTrustedWithOptions(options: Ref) -> bool;
        static kAXTrustedCheckOptionPrompt: *const c_void;
    }

    const HID_TAP: u32 = 0;
    const LEFT_DOWN: u32 = 1;
    const LEFT_UP: u32 = 2;
    const RIGHT_DOWN: u32 = 3;
    const RIGHT_UP: u32 = 4;
    const MOVED: u32 = 5;
    const LEFT_DRAGGED: u32 = 6;
    const OTHER_DOWN: u32 = 25;
    const OTHER_UP: u32 = 26;
    const CLICK_STATE: u32 = 1;
    const PIXELS: u32 = 0;

    /// Whether macOS lets Vyoma move the pointer and type. The first time it isn't, macOS is asked
    /// to show its Allow (it opens Accessibility in System Settings with Vyoma listed).
    fn allowed(prompt: bool) -> bool {
        // SAFETY: plain CoreFoundation/ApplicationServices calls; the dictionary is released.
        unsafe {
            if AXIsProcessTrusted() {
                return true;
            }
            if !prompt {
                return false;
            }
            let keys = [kAXTrustedCheckOptionPrompt];
            let values = [kCFBooleanTrue];
            let options = CFDictionaryCreate(std::ptr::null_mut(), keys.as_ptr(), values.as_ptr(), 1, (&raw const kCFTypeDictionaryKeyCallBacks).cast(), (&raw const kCFTypeDictionaryValueCallBacks).cast());
            let yes = AXIsProcessTrustedWithOptions(options);
            if !options.is_null() {
                CFRelease(options);
            }
            yes
        }
    }

    /// Posts [event] and lets it go.
    unsafe fn post(event: Ref) {
        if !event.is_null() {
            unsafe {
                CGEventPost(HID_TAP, event);
                CFRelease(event);
            }
        }
    }

    /// The pointer's side: whether the left button is held (so moves are drags), and whether
    /// macOS's Allow was asked for already.
    #[derive(Default)]
    pub struct Pointer {
        holding: bool,
        asked: bool,
    }

    impl Pointer {
        /// Does [acts]; [super::NotAllowed] the first time macOS won't let it.
        pub fn handle(&mut self, acts: &[Act]) -> Result<(), super::NotAllowed> {
            if !allowed(!self.asked) {
                return if std::mem::replace(&mut self.asked, true) { Ok(()) } else { Err(super::NotAllowed) };
            }
            for a in acts {
                // SAFETY: CoreGraphics event calls on events made here, each released after posting.
                unsafe { self.one(a) }
            }
            Ok(())
        }

        unsafe fn here() -> Point {
            unsafe {
                let e = CGEventCreate(std::ptr::null_mut());
                if e.is_null() {
                    return Point { x: 0.0, y: 0.0 };
                }
                let p = CGEventGetLocation(e);
                CFRelease(e);
                p
            }
        }

        /// [p] kept on a screen: the union of every display's bounds.
        unsafe fn on_screen(p: Point) -> Point {
            let mut ids = [0u32; 16];
            let mut n = 0u32;
            unsafe {
                if CGGetActiveDisplayList(16, ids.as_mut_ptr(), &mut n) != 0 || n == 0 {
                    return p;
                }
                let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
                for id in &ids[..n as usize] {
                    let r = CGDisplayBounds(*id);
                    x0 = x0.min(r.origin.x);
                    y0 = y0.min(r.origin.y);
                    x1 = x1.max(r.origin.x + r.size.x - 1.0);
                    y1 = y1.max(r.origin.y + r.size.y - 1.0);
                }
                Point { x: p.x.clamp(x0, x1), y: p.y.clamp(y0, y1) }
            }
        }

        unsafe fn one(&mut self, a: &Act) {
            unsafe {
                match a {
                    Act::Move(dx, dy) => {
                        let at = Self::here();
                        let to = Self::on_screen(Point { x: at.x + dx, y: at.y + dy });
                        let kind = if self.holding { LEFT_DRAGGED } else { MOVED };
                        post(CGEventCreateMouseEvent(std::ptr::null_mut(), kind, to, 0));
                    }
                    Act::Scroll(dx, dy) => {
                        // CoreGraphics: positive is up and left.
                        post(CGEventCreateScrollWheelEvent(std::ptr::null_mut(), PIXELS, 2, -dy.round() as i32, -dx.round() as i32));
                    }
                    Act::Button(b, down) => {
                        let (kind, button) = match (b, down) {
                            (Button::Left, true) => (LEFT_DOWN, 0),
                            (Button::Left, false) => (LEFT_UP, 0),
                            (Button::Right, true) => (RIGHT_DOWN, 1),
                            (Button::Right, false) => (RIGHT_UP, 1),
                            (Button::Middle, true) => (OTHER_DOWN, 2),
                            (Button::Middle, false) => (OTHER_UP, 2),
                        };
                        if *b == Button::Left {
                            self.holding = *down;
                        }
                        post(CGEventCreateMouseEvent(std::ptr::null_mut(), kind, Self::here(), button));
                    }
                    Act::Double => {
                        let at = Self::here();
                        for kind in [LEFT_DOWN, LEFT_UP] {
                            let e = CGEventCreateMouseEvent(std::ptr::null_mut(), kind, at, 0);
                            if !e.is_null() {
                                CGEventSetIntegerValueField(e, CLICK_STATE, 2);
                            }
                            post(e);
                        }
                    }
                    Act::Key(code, flags) => {
                        for down in [true, false] {
                            let e = CGEventCreateKeyboardEvent(std::ptr::null_mut(), *code, down);
                            if !e.is_null() {
                                CGEventSetFlags(e, *flags);
                            }
                            post(e);
                        }
                    }
                    Act::Text(text) => {
                        // One character a key press: apps read a long string on one event unevenly.
                        let mut buf = [0u16; 2];
                        for c in text.chars() {
                            let units = c.encode_utf16(&mut buf);
                            for down in [true, false] {
                                let e = CGEventCreateKeyboardEvent(std::ptr::null_mut(), 0, down);
                                if !e.is_null() {
                                    CGEventSetFlags(e, 0);
                                    CGEventKeyboardSetUnicodeString(e, units.len() as c_ulong, units.as_ptr());
                                }
                                post(e);
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicks_moves_and_scrolls() {
        assert_eq!(acts(&json!({ "dx": 3.0, "dy": -2.0 })), [Act::Move(3.0, -2.0)]);
        assert_eq!(acts(&json!({ "singleclick": true })), [Act::Button(Button::Left, true), Act::Button(Button::Left, false)]);
        assert_eq!(acts(&json!({ "doubleclick": true })).last(), Some(&Act::Double), "the second click counts as a double");
        assert_eq!(acts(&json!({ "rightclick": true }))[0], Act::Button(Button::Right, true));
        assert_eq!(acts(&json!({ "singlehold": true })), [Act::Button(Button::Left, true)]);
        assert_eq!(acts(&json!({ "scroll": true, "dx": 0.0, "dy": 12.0 })), [Act::Scroll(0.0, 12.0)]);
        assert!(acts(&json!({})).is_empty());
    }

    #[test]
    fn typing_is_characters_and_shortcuts_are_keys() {
        assert_eq!(acts(&json!({ "key": "Hé!" })), [Act::Text("Hé!".into())], "any character, in any layout");
        assert_eq!(acts(&json!({ "key": "A", "shift": true })), [Act::Text("A".into())], "Shift alone is just typing");
        assert_eq!(acts(&json!({ "key": "hi\nyo" })), [Act::Text("hi".into()), Act::Key(RETURN, 0), Act::Text("yo".into())]);
        assert_eq!(acts(&json!({ "key": "c", "ctrl": true })), [Act::Key(8, COMMAND)], "the phone's Ctrl+C copies");
        assert_eq!(acts(&json!({ "key": "T", "ctrl": true, "shift": true })), [Act::Key(17, COMMAND | SHIFT)]);
        assert_eq!(acts(&json!({ "key": "x", "alt": true, "super": true })), [Act::Key(7, OPTION | CONTROL)]);
        assert_eq!(acts(&json!({ "specialKey": 12 })), [Act::Key(RETURN, 0)]);
        assert_eq!(acts(&json!({ "specialKey": 4, "shift": true })), [Act::Key(123, SHIFT)], "Shift+Left selects");
        assert!(acts(&json!({ "specialKey": 99 })).is_empty(), "unknown keys press nothing");
    }

    #[test]
    fn echoes_only_when_asked() {
        let e = echo(&json!({ "key": "a", "sendAck": true })).unwrap();
        assert_eq!(e["body"], json!({ "key": "a", "isAck": true }));
        assert!(echo(&json!({ "key": "a" })).is_none());
    }
}
