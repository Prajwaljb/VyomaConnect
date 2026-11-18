//! The built-in display's brightness from the phone (`vyoma.pc.request`, `vyoma.pc.set`, answered
//! with `vyoma.pc`), through macOS's DisplayServices, the private framework its own brightness keys
//! use: loaded at run time, so a macOS without it just doesn't offer this. Offered only where
//! there is a built-in display. The phone's per-app volumes stay empty: macOS has no such control.

use serde_json::{Value, json};
use tokio::sync::mpsc;
use vyoma_core::link::packet;
use vyoma_core::device::Command;

pub const REQUEST: &str = "vyoma.pc.request";
pub const SET: &str = "vyoma.pc.set";
pub const STATE: &str = "vyoma.pc";
const INCOMING: &[&str] = &[REQUEST, SET];
const OUTGOING: &[&str] = &[STATE];

pub fn takes(kind: &str) -> bool {
    INCOMING.contains(&kind)
}

/// What to say we take: nothing without a display this can dim.
pub fn incoming() -> &'static [&'static str] {
    if display::all().is_empty() { &[] } else { INCOMING }
}

pub fn outgoing() -> &'static [&'static str] {
    if display::all().is_empty() { &[] } else { OUTGOING }
}

fn state(displays: &[(u32, f32)]) -> Value {
    let list: Vec<Value> = displays.iter().map(|&(id, value)| json!({ "id": id.to_string(), "label": "Built-in display", "internal": true, "value": value })).collect();
    json!({ "displays": list, "apps": [] })
}

/// The display and level a `vyoma.pc.set` body names, if it's one of [displays].
fn wanted(body: &Value, displays: &[(u32, f32)]) -> Option<(u32, f32)> {
    let id = body["display"].as_str()?.parse().ok()?;
    let level = body["brightness"].as_f64()?;
    displays.iter().any(|&(d, _)| d == id).then_some((id, level.clamp(0.0, 1.0) as f32))
}

/// Takes the phones' packets in order. A burst of moves of the slider comes down to its newest, and
/// each phone of the burst gets one answer.
pub fn start(commands: mpsc::UnboundedSender<Command>) -> mpsc::UnboundedSender<(String, Value)> {
    let (tx, mut rx) = mpsc::unbounded_channel::<(String, Value)>();
    tokio::spawn(async move {
        while let Some(first) = rx.recv().await {
            let mut batch = vec![first];
            while let Ok(next) = rx.try_recv() {
                batch.push(next);
            }
            let displays = display::all();
            let mut asked: Vec<String> = Vec::new();
            for (i, (id, p)) in batch.iter().enumerate() {
                if !asked.contains(id) {
                    asked.push(id.clone());
                }
                if p["type"] != SET {
                    continue;
                }
                let same = |q: &Value| q["type"] == SET && q["body"]["display"] == p["body"]["display"];
                if batch[i + 1..].iter().any(|(_, q)| same(q)) {
                    continue;
                }
                if let Some((display, level)) = wanted(&p["body"], &displays) {
                    display::set(display, level);
                }
            }
            let body = state(&display::all());
            for id in asked {
                let _ = commands.send(Command::Send(id, packet(STATE, body.clone())));
            }
        }
    });
    tx
}

#[cfg(target_os = "macos")]
mod display {
    use std::ffi::c_void;
    use std::sync::OnceLock;

    type Get = unsafe extern "C" fn(u32, *mut f32) -> i32;
    type Set = unsafe extern "C" fn(u32, f32) -> i32;

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGGetOnlineDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
        fn CGDisplayIsBuiltin(display: u32) -> i32;
    }

    fn api() -> Option<(Get, Set)> {
        static API: OnceLock<Option<(usize, usize)>> = OnceLock::new();
        let (get, set) = (*API.get_or_init(|| {
            // SAFETY: loading a system framework by its fixed path and looking up two functions.
            unsafe {
                let lib = libc::dlopen(c"/System/Library/PrivateFrameworks/DisplayServices.framework/DisplayServices".as_ptr(), libc::RTLD_LAZY);
                if lib.is_null() {
                    return None;
                }
                let get: *mut c_void = libc::dlsym(lib, c"DisplayServicesGetBrightness".as_ptr());
                let set: *mut c_void = libc::dlsym(lib, c"DisplayServicesSetBrightness".as_ptr());
                (!get.is_null() && !set.is_null()).then_some((get as usize, set as usize))
            }
        }))?;
        // SAFETY: both are the framework's functions of these signatures, loaded above.
        Some(unsafe { (std::mem::transmute::<usize, Get>(get), std::mem::transmute::<usize, Set>(set)) })
    }

    /// Built-in displays and their brightness, 0 to 1.
    pub fn all() -> Vec<(u32, f32)> {
        let Some((get, _)) = api() else { return Vec::new() };
        let mut ids = [0u32; 16];
        let mut count = 0u32;
        // SAFETY: the buffer holds the count given; the calls only read display ids it filled in.
        unsafe {
            if CGGetOnlineDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) != 0 {
                return Vec::new();
            }
            ids[..count as usize]
                .iter()
                .filter(|&&id| CGDisplayIsBuiltin(id) != 0)
                .filter_map(|&id| {
                    let mut level = 0f32;
                    (get(id, &mut level) == 0).then_some((id, level))
                })
                .collect()
        }
    }

    pub fn set(id: u32, level: f32) {
        if let Some((_, set)) = api() {
            // SAFETY: a display id from the list above.
            if unsafe { set(id, level) } != 0 {
                tracing::info!("macOS didn't take that brightness");
            }
        }
    }
}

/// Elsewhere (this crate's tests on Linux) there's no display to dim.
#[cfg(not(target_os = "macos"))]
mod display {
    pub fn all() -> Vec<(u32, f32)> {
        Vec::new()
    }
    pub fn set(_: u32, _: f32) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_what_the_phones_controls_read() {
        let v = state(&[(1, 0.5)]);
        assert_eq!(v["displays"][0], json!({ "id": "1", "label": "Built-in display", "internal": true, "value": 0.5 }));
        assert_eq!(v["apps"], json!([]));
    }

    #[test]
    fn only_a_display_it_knows_and_a_level_in_range() {
        let displays = [(1, 0.5)];
        assert_eq!(wanted(&json!({ "display": "1", "brightness": 2.0 }), &displays), Some((1, 1.0)));
        assert_eq!(wanted(&json!({ "display": "9", "brightness": 0.2 }), &displays), None);
        assert_eq!(wanted(&json!({ "app": 3, "volume": 0.2 }), &displays), None);
    }
}
