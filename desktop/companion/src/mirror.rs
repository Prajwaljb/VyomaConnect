//! The phone's screen in a window on this PC, driven with the mouse and keyboard. The phone sends
//! its screen as live H.264 (`vyoma.mirror`, a MediaProjection); the window's clicks, drags, wheel
//! and typing go back as `vyoma.mirror.input` (0..1 across the picture), which the phone's
//! accessibility service performs. Closing the window ends it (`vyoma.mirror.state {open:false}`).
//!
//! The window is this same binary run as a child (`--mirror-window`): GStreamer decodes stdin
//! (VA-API) into a GL window and writes what the user does in it as JSON lines on stdout. A child,
//! so a GPU driver crash can't take the companion down. It runs through XWayland: GStreamer's
//! Wayland windows have no keyboard (1.28), its X11 one does.

use std::io::Write;
use std::net::IpAddr;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use gst::prelude::*;
use gstreamer as gst;
use gstreamer_video as gst_video;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tracing::info;

use crate::identity::Identity;
use crate::link::{self, Sender};
use crate::payload;

pub const WINDOW_ARG: &str = "--mirror-window";

async fn state(sender: &Sender, body: Value) {
    let _ = sender.send(&link::packet("vyoma.mirror.state", body)).await;
}

/// Shows the phone's screen until the phone stops or the window is closed.
pub async fn show(sender: &Sender, me: &Identity, pinned: &[u8], phone: IpAddr, packet: &Value, name: &str) -> Result<()> {
    let mut input = payload::fetch_stream(me, pinned, phone, packet).await?;
    let (max_w, max_h) = bounds().await;
    let child = tokio::process::Command::new(std::env::current_exe()?)
        .args([WINDOW_ARG, name, &max_w.to_string(), &max_h.to_string()])
        .env("GST_GL_WINDOW", "x11")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            state(sender, json!({ "open": false, "error": format!("Couldn't open the window: {e}") })).await;
            return Err(e.into());
        }
    };
    let mut stdin = child.stdin.take().context("no stdin")?;
    let mut lines = BufReader::new(child.stdout.take().context("no stdout")?).lines();
    state(sender, json!({ "open": true })).await;
    info!("{name}: mirroring the phone's screen");

    let feed = async {
        let _ = tokio::io::copy(&mut input, &mut stdin).await;
        // The phone stopped: the window sees the end of the stream and closes.
        drop(stdin);
    };
    let inputs = async {
        while let Ok(Some(line)) = lines.next_line().await {
            if let Ok(body) = serde_json::from_str::<Value>(&line) {
                let _ = sender.send(&link::packet("vyoma.mirror.input", body)).await;
            }
        }
    };
    tokio::select! {
        _ = feed => {
            let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
        }
        // The window closed (its stdout ends with it).
        _ = inputs => {}
    }
    state(sender, json!({ "open": false })).await;
    info!("{name}: mirror closed");
    Ok(())
}

/// The window's largest picture: 85% of the smallest screen, so a tall phone fits on any of them.
async fn bounds() -> (u32, u32) {
    let fallback = (1000, 1000);
    let out = tokio::time::timeout(Duration::from_secs(3), tokio::process::Command::new("kscreen-doctor").arg("-j").kill_on_drop(true).output()).await;
    let Ok(Ok(out)) = out else { return fallback };
    let Ok(v) = serde_json::from_slice::<Value>(&out.stdout) else { return fallback };
    let mut best: Option<(f64, f64)> = None;
    for o in v["outputs"].as_array().into_iter().flatten().filter(|o| o["enabled"].as_bool() == Some(true)) {
        let scale = o["scale"].as_f64().unwrap_or(1.0).max(0.5);
        let (mut w, mut h) = (o["size"]["width"].as_f64().unwrap_or(0.0) / scale, o["size"]["height"].as_f64().unwrap_or(0.0) / scale);
        // Turned a quarter (KScreen: 2 left, 8 right).
        if matches!(o["rotation"].as_u64(), Some(2 | 8)) {
            std::mem::swap(&mut w, &mut h);
        }
        if w > 0.0 && h > 0.0 {
            best = Some(best.map_or((w, h), |(bw, bh)| (bw.min(w), bh.min(h))));
        }
    }
    best.map_or(fallback, |(w, h)| ((w * 0.85) as u32, (h * 0.85) as u32))
}

// ---- The window process ----

#[repr(C)]
struct RuleNames {
    rules: *const libc::c_char,
    model: *const libc::c_char,
    layout: *const libc::c_char,
    variant: *const libc::c_char,
    options: *const libc::c_char,
}

#[link(name = "xkbcommon")]
unsafe extern "C" {
    fn xkb_keysym_from_name(name: *const libc::c_char, flags: libc::c_int) -> u32;
    fn xkb_keysym_to_utf32(keysym: u32) -> u32;
    fn xkb_context_new(flags: libc::c_int) -> *mut libc::c_void;
    fn xkb_keymap_new_from_names(ctx: *mut libc::c_void, names: *const RuleNames, flags: libc::c_int) -> *mut libc::c_void;
    fn xkb_keymap_min_keycode(keymap: *mut libc::c_void) -> u32;
    fn xkb_keymap_max_keycode(keymap: *mut libc::c_void) -> u32;
    fn xkb_keymap_key_get_syms_by_level(keymap: *mut libc::c_void, key: u32, layout: u32, level: u32, syms: *mut *const u32) -> libc::c_int;
}

/// Each key's plain keysym → its Shift keysym, in the desktop's first layout (Plasma's kxkbrc).
/// GStreamer's X11 window names keys by their unshifted symbol ("1" for "!"), so Shift is ours to apply.
fn shifted_map() -> &'static std::collections::HashMap<u32, u32> {
    static MAP: std::sync::OnceLock<std::collections::HashMap<u32, u32>> = std::sync::OnceLock::new();
    MAP.get_or_init(|| {
        let mut map = std::collections::HashMap::new();
        let layout = std::env::var("HOME")
            .ok()
            .and_then(|h| std::fs::read_to_string(format!("{h}/.config/kxkbrc")).ok())
            .and_then(|c| c.lines().find_map(|l| l.strip_prefix("LayoutList=").map(|v| v.split(',').next().unwrap_or("us").trim().to_string())))
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| "us".into());
        let Ok(layout) = std::ffi::CString::new(layout) else { return map };
        unsafe {
            let ctx = xkb_context_new(0);
            if ctx.is_null() {
                return map;
            }
            let names = RuleNames { rules: std::ptr::null(), model: std::ptr::null(), layout: layout.as_ptr(), variant: std::ptr::null(), options: std::ptr::null() };
            let km = xkb_keymap_new_from_names(ctx, &names, 0);
            if km.is_null() {
                return map;
            }
            for kc in xkb_keymap_min_keycode(km)..=xkb_keymap_max_keycode(km) {
                let (mut plain, mut shift): (*const u32, *const u32) = (std::ptr::null(), std::ptr::null());
                if xkb_keymap_key_get_syms_by_level(km, kc, 0, 0, &mut plain) == 1
                    && xkb_keymap_key_get_syms_by_level(km, kc, 0, 1, &mut shift) == 1
                {
                    map.entry(*plain).or_insert(*shift);
                }
            }
            // The keymap and context live as long as this process: one window.
        }
        map
    })
}

/// The character an X keysym name types ("a", "comma", "eacute"), with Shift / Caps Lock applied.
fn typed(key: &str, shift: bool, caps: bool) -> Option<char> {
    let name = std::ffi::CString::new(key).ok()?;
    let mut sym = unsafe { xkb_keysym_from_name(name.as_ptr(), 0) };
    if sym == 0 {
        return None;
    }
    if shift {
        sym = shifted_map().get(&sym).copied().unwrap_or(sym);
    }
    let c = char::from_u32(unsafe { xkb_keysym_to_utf32(sym) }).filter(|c| !c.is_control())?;
    // Caps Lock flips letters only (Shift with it gives lower case).
    Some(if caps && c.is_alphabetic() {
        if c.is_uppercase() { c.to_lowercase().next().unwrap_or(c) } else { c.to_uppercase().next().unwrap_or(c) }
    } else {
        c
    })
}

/// Modifier keys held in the window, tracked from their own presses and releases (the X11 window
/// doesn't report them with other keys).
#[derive(Default)]
struct Held {
    shift: bool,
    ctrl: bool,
    alt: bool,
    caps: bool,
}

impl Held {
    /// True if [key] was a modifier (and so types nothing).
    fn track(&mut self, key: &str, down: bool) -> bool {
        match key {
            "Shift_L" | "Shift_R" => self.shift = down,
            "Control_L" | "Control_R" => self.ctrl = down,
            "Alt_L" | "Alt_R" | "Meta_L" | "Meta_R" | "Super_L" | "Super_R" | "ISO_Level3_Shift" => self.alt = down,
            "Caps_Lock" => {
                if down {
                    self.caps = !self.caps;
                }
            }
            _ => return false,
        }
        true
    }
}

/// What a key press in the window means on the phone.
fn key_input(key: &str, modifiers: gst_video::NavigationModifierType, held: &Held) -> Option<Value> {
    use gst_video::NavigationModifierType as M;
    let named = match key {
        "Escape" => return Some(json!({ "kind": "back" })),
        "BackSpace" => "backspace",
        "Delete" | "KP_Delete" => "delete",
        "Return" | "KP_Enter" => "enter",
        "Left" | "KP_Left" => "left",
        "Right" | "KP_Right" => "right",
        "Up" | "KP_Up" => "up",
        "Down" | "KP_Down" => "down",
        "Tab" => return Some(json!({ "kind": "text", "text": "\t" })),
        _ => "",
    };
    if !named.is_empty() {
        return Some(json!({ "kind": "key", "key": named }));
    }
    // Shortcuts aren't typing.
    if held.ctrl || held.alt || modifiers.intersects(M::CONTROL_MASK | M::MOD1_MASK | M::SUPER_MASK | M::META_MASK) {
        return None;
    }
    let shift = held.shift || modifiers.contains(M::SHIFT_MASK);
    let caps = held.caps || modifiers.contains(M::LOCK_MASK);
    typed(key, shift, caps).map(|c| json!({ "kind": "text", "text": c.to_string() }))
}

struct Pointer {
    held: bool,
    last_move: Instant,
    keys: Held,
}

/// A move at most this often while dragging (the phone draws at 60-120 Hz anyway).
const MOVE_EVERY: Duration = Duration::from_millis(8);

fn emit(v: Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{v}");
    let _ = out.flush();
}

/// The window process: H.264 on stdin, a window, inputs on stdout. Exits when the window closes or
/// the stream ends.
pub fn window(args: &[String]) -> Result<()> {
    let title = args.first().cloned().unwrap_or_else(|| "Phone".into());
    let max_w: u32 = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(1000);
    let max_h: u32 = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(1000);
    gst::init()?;
    let has = |n: &str| gst::ElementFactory::find(n).is_some();
    let decode = if has("vah264dec") { "vah264dec" } else { "avdec_h264" };
    let scale = if has("vapostproc") && decode == "vah264dec" { "vapostproc" } else { "videoconvertscale" };
    let pipeline = gst::parse::launch(&format!(
        "fdsrc fd=0 ! h264parse ! {decode} ! {scale} ! capsfilter name=fit caps=\"video/x-raw,width=[1,{max_w}],height=[1,{max_h}],pixel-aspect-ratio=1/1\" \
         ! glimagesink name=sink sync=false force-aspect-ratio=true"
    ))?
    .downcast::<gst::Pipeline>()
    .map_err(|_| anyhow::anyhow!("not a pipeline"))?;

    let fit = pipeline.by_name("fit").context("no fit")?;
    let pad = fit.static_pad("src").context("no src pad")?;
    let pointer = Mutex::new(Pointer { held: false, last_move: Instant::now(), keys: Held::default() });
    pad.add_probe(gst::PadProbeType::EVENT_UPSTREAM, move |pad, info| {
        let Some(event) = info.event() else { return gst::PadProbeReturn::Ok };
        if event.type_() != gst::EventType::Navigation {
            return gst::PadProbeReturn::Ok;
        }
        let Ok(nav) = gst_video::NavigationEvent::parse(event) else { return gst::PadProbeReturn::Drop };
        let (w, h) = pad
            .current_caps()
            .and_then(|c| gst_video::VideoInfo::from_caps(&c).ok())
            .map_or((1.0, 1.0), |i| (i.width() as f64, i.height() as f64));
        let at = |x: f64, y: f64| ((x / w).clamp(0.0, 1.0), (y / h).clamp(0.0, 1.0));
        let mut p = pointer.lock().unwrap();
        if std::env::var_os("VYOMA_MIRROR_DEBUG").is_some() {
            eprintln!("mirror input: {nav:?}");
        }
        use gst_video::NavigationEvent as N;
        match nav {
            N::MouseButtonPress { button: 1, x, y, .. } => {
                p.held = true;
                let (x, y) = at(x, y);
                emit(json!({ "kind": "down", "x": x, "y": y }));
            }
            N::MouseMove { x, y, .. } if p.held => {
                if p.last_move.elapsed() >= MOVE_EVERY {
                    p.last_move = Instant::now();
                    let (x, y) = at(x, y);
                    emit(json!({ "kind": "move", "x": x, "y": y }));
                }
            }
            N::MouseButtonRelease { button: 1, x, y, .. } if p.held => {
                p.held = false;
                let (x, y) = at(x, y);
                emit(json!({ "kind": "up", "x": x, "y": y }));
            }
            // Right click and the mouse's back button: Back. Middle click: Home.
            N::MouseButtonPress { button: 3 | 8, .. } => emit(json!({ "kind": "back" })),
            N::MouseButtonPress { button: 2, .. } => emit(json!({ "kind": "home" })),
            N::MouseScroll { x, y, delta_x, delta_y, .. } => {
                let (x, y) = at(x, y);
                emit(json!({ "kind": "scroll", "x": x, "y": y, "dx": delta_x, "dy": delta_y }));
            }
            // X11 reports the wheel as buttons: 4 up, 5 down, 6 left, 7 right.
            N::MouseButtonPress { button: b @ 4..=7, x, y, .. } => {
                let (x, y) = at(x, y);
                let (dx, dy) = match b { 4 => (0, -1), 5 => (0, 1), 6 => (-1, 0), _ => (1, 0) };
                emit(json!({ "kind": "scroll", "x": x, "y": y, "dx": dx, "dy": dy }));
            }
            N::KeyPress { key, modifier_state } => {
                if !p.keys.track(&key, true)
                    && let Some(v) = key_input(&key, modifier_state, &p.keys)
                {
                    emit(v);
                }
            }
            N::KeyRelease { key, .. } => {
                p.keys.track(&key, false);
            }
            _ => {}
        }
        gst::PadProbeReturn::Drop
    });

    // The window's title: the phone's name.
    let mut tags = gst::TagList::new();
    tags.get_mut().unwrap().add::<gst::tags::Title>(&title.as_str(), gst::TagMergeMode::Replace);
    let sink = pipeline.by_name("sink").context("no sink")?;
    if let Some(sink_pad) = sink.static_pad("sink") {
        let _ = sink_pad.send_event(gst::event::Tag::new(tags));
    }

    pipeline.set_state(gst::State::Playing)?;
    let bus = pipeline.bus().context("no bus")?;
    let mut result = Ok(());
    for msg in bus.iter_timed(gst::ClockTime::NONE) {
        match msg.view() {
            gst::MessageView::Eos(..) => break,
            gst::MessageView::Error(e) => {
                // Closing the window is an error to GStreamer ("Output window was closed").
                let msg = e.error().to_string();
                if !msg.contains("closed") && !msg.contains("Quit requested") {
                    result = Err(anyhow::anyhow!("{} ({:?})", e.error(), e.debug()));
                }
                break;
            }
            _ => {}
        }
    }
    let _ = pipeline.set_state(gst::State::Null);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_type_what_they_say() {
        let none = gst_video::NavigationModifierType::empty();
        let plain = Held::default();
        let shift = Held { shift: true, ..Held::default() };
        let caps = Held { caps: true, ..Held::default() };
        let k = |key, held: &Held| key_input(key, none, held).map(|v| v["text"].as_str().unwrap_or("").to_string());
        assert_eq!(k("a", &plain).unwrap(), "a");
        assert_eq!(k("a", &shift).unwrap(), "A");
        assert_eq!(k("1", &shift).unwrap(), "!");
        assert_eq!(k("comma", &plain).unwrap(), ",");
        assert_eq!(k("comma", &shift).unwrap(), "<");
        assert_eq!(k("slash", &shift).unwrap(), "?");
        assert_eq!(k("a", &caps).unwrap(), "A");
        assert_eq!(k("1", &caps).unwrap(), "1");
        assert_eq!(k("space", &plain).unwrap(), " ");
        assert_eq!(key_input("Return", none, &plain).unwrap()["key"], "enter");
        assert_eq!(key_input("Escape", none, &plain).unwrap()["kind"], "back");
        assert!(key_input("c", none, &Held { ctrl: true, ..Held::default() }).is_none());
        let mut h = Held::default();
        assert!(h.track("Shift_L", true) && h.shift);
        assert!(h.track("Shift_L", false) && !h.shift);
        assert!(!h.track("a", true));
    }
}
