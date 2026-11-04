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

/// How long the picture is held back so it matches the sound: while the phone's sound plays here,
/// the PC's output delay (a Bluetooth speaker: ~190 ms, user 2026-11-04) plus this side's
/// buffering; 0 without sound, so working the phone stays as quick as it can be.
static VIDEO_DELAY_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// The sound's own delay here before the output: the jitter buffer's target, pacat's 40 ms and
/// PipeWire's cycle, less what the picture takes to encode and decode.
const SOUND_PIPELINE_MS: u64 = 60;
/// When the phone last made a sound worth hearing, and when the user last worked the window (ms
/// since [clock]'s start): the picture is held back only while sound plays and nobody's using it.
static LAST_SOUND_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static LAST_INPUT_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Quieter than this (of 32767) is silence: nothing to keep in step with.
const AUDIBLE: i16 = 64;
/// Sound this recent still counts as playing (gaps between words).
const SOUND_HOLD_MS: u64 = 1_500;
/// The window used this recently: show the phone as fast as possible.
const INPUT_HOLD_MS: u64 = 2_000;

fn clock() -> u64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}

/// What the picture waits right now: the sound's delay while it plays and the window is just
/// being watched; nothing while the user works the phone, or when it's silent.
fn video_delay() -> Duration {
    use std::sync::atomic::Ordering::Relaxed;
    let now = clock();
    let sound = LAST_SOUND_MS.load(Relaxed);
    let input = LAST_INPUT_MS.load(Relaxed);
    let playing = sound != 0 && now.saturating_sub(sound) < SOUND_HOLD_MS;
    let used = input != 0 && now.saturating_sub(input) < INPUT_HOLD_MS;
    if playing && !used { Duration::from_millis(VIDEO_DELAY_MS.load(Relaxed)) } else { Duration::ZERO }
}

/// A pause in the phone's picture this long is logged.
const STALL: Duration = Duration::from_millis(250);

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

    // Video waits here as long as the sound takes to be heard (see [VIDEO_DELAY_MS]); a writer
    // passes each piece to the window once its time comes.
    let (pieces, mut due) = tokio::sync::mpsc::unbounded_channel::<(Instant, Vec<u8>)>();
    let feed = async {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let read = async {
            let mut buf = vec![0u8; 64 * 1024];
            let mut last = Instant::now();
            loop {
                let n = match input.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                // A gap like this is a freeze on the PC: say where it came from (the phone or the link).
                let gap = last.elapsed();
                if gap > STALL {
                    info!("{name}: no picture from the phone for {} ms", gap.as_millis());
                }
                last = Instant::now();
                if pieces.send((last, buf[..n].to_vec())).is_err() {
                    break;
                }
            }
            drop(pieces);
        };
        let write = async {
            while let Some((at, piece)) = due.recv().await {
                // Checked again while waiting: a click mid-wait shows the picture at once.
                while at + video_delay() > Instant::now() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                if stdin.write_all(&piece).await.is_err() {
                    break;
                }
            }
            // The phone stopped: the window sees the end of the stream and closes.
            drop(stdin);
        };
        tokio::join!(read, write);
    };
    let inputs = async {
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(body) = serde_json::from_str::<Value>(&line) else { continue };
            LAST_INPUT_MS.store(clock(), std::sync::atomic::Ordering::Relaxed);
            let body = if body["kind"] == "paste" {
                match clipboard().await {
                    Some(text) => json!({ "kind": "text", "text": text }),
                    None => continue,
                }
            } else {
                body
            };
            let _ = sender.send(&link::packet("vyoma.mirror.input", body)).await;
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

/// Most text one paste types into the phone.
const PASTE_MAX: usize = 20_000;

/// The PC's clipboard as text (Wayland), if there is some.
async fn clipboard() -> Option<String> {
    let out = tokio::time::timeout(
        Duration::from_secs(2),
        tokio::process::Command::new("wl-paste").args(["--no-newline", "--type", "text/plain"]).kill_on_drop(true).output(),
    )
    .await
    .ok()?
    .ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    if text.is_empty() {
        return None;
    }
    Some(text.chars().take(PASTE_MAX).collect())
}

/// The phone copied text in the mirror (Ctrl+C / Ctrl+X in the window): onto the PC's clipboard.
pub async fn copy(text: &str) {
    use tokio::io::AsyncWriteExt;
    let Ok(mut c) = tokio::process::Command::new("wl-copy").args(["--type", "text/plain"]).stdin(Stdio::piped()).spawn() else { return };
    if let Some(mut stdin) = c.stdin.take() {
        let _ = stdin.write_all(text.as_bytes()).await;
    }
    // wl-copy forks a server to hold the clipboard and its first process exits.
    let _ = tokio::time::timeout(Duration::from_secs(2), c.wait()).await;
}

/// The phone's volume keys while its sound plays here: the PC's output, 5% a press, up to 100%.
pub async fn volume(up: bool) {
    let step = if up { "5%+" } else { "5%-" };
    let _ = tokio::process::Command::new("wpctl").args(["set-volume", "-l", "1.0", "@DEFAULT_AUDIO_SINK@", step]).status().await;
}

/// Sound waiting to play, in 10 ms pieces: aim to keep at least this many in hand (absorbs the
/// network's bursts) ...
const SOUND_TARGET: usize = 4;
/// ... judged by the fewest there were over this long (bursts make the count swing; the low point is
/// the delay that stays) ...
const SOUND_WINDOW: Duration = Duration::from_secs(2);
/// ... and never more than this (a stall that then floods in).
const SOUND_MAX: usize = 30;

/// The phone's sound waiting for the player: pieces, and the reader's end-of-stream.
struct SoundQueue {
    pieces: std::collections::VecDeque<Vec<u8>>,
    done: bool,
}

type Shared = std::sync::Arc<(Mutex<SoundQueue>, std::sync::Condvar)>;

/// Ends the feeder thread and pacat however [sound] ends, a cancel (the mirror closing) included.
struct Player {
    child: std::process::Child,
    queue: Shared,
}

impl Drop for Player {
    fn drop(&mut self) {
        let (lock, ready) = &*self.queue;
        lock.lock().unwrap().done = true;
        ready.notify_one();
        // The feeder may be blocked writing to pacat: killing it ends that write.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The phone's sound while it's mirrored (`vyoma.mirror.audio {rate, channels}` + live s16le PCM),
/// played on the PC's default output through PipeWire's pulse server until the phone stops it.
///
/// Kept in step with the picture by a small jitter buffer: sound is read off the network as it
/// comes (so it never backs up there; pacat blocking the network built a seconds-long lag), and a
/// thread feeds pacat at its own pace. Sound arrives in bursts, so the buffer is judged by its
/// lowest level over [SOUND_WINDOW]: above [SOUND_TARGET] means the phone's clock runs ahead, and a
/// piece is skipped (a hard 4 KiB pipe that dropped on every burst crackled, 2026-11-04). Running
/// dry, it waits for the target again before playing on.
pub async fn sound(me: &Identity, pinned: &[u8], phone: IpAddr, packet: &Value, name: &str) -> Result<()> {
    use tokio::io::AsyncReadExt;
    let body = &packet["body"];
    let rate = body["rate"].as_u64().unwrap_or(48_000).clamp(8_000, 96_000) as usize;
    let channels = body["channels"].as_u64().unwrap_or(2).clamp(1, 2) as usize;
    let mut input = payload::fetch_stream(me, pinned, phone, packet).await?;
    let mut player = std::process::Command::new("pacat")
        .args([
            "--playback",
            "--raw",
            "--format=s16le",
            &format!("--rate={rate}"),
            &format!("--channels={channels}"),
            "--latency-msec=40",
            "--client-name=Vyoma",
            &format!("--stream-name={name}"),
        ])
        .stdin(Stdio::piped())
        .spawn()
        .context("pacat")?;
    let mut out = player.stdin.take().context("no stdin")?;
    info!("{name}: playing the phone's sound");
    let _delay = DelayVideo;
    let delay_task = tokio::spawn(DelayVideo::follow(name.to_string()));
    struct Abort(tokio::task::JoinHandle<()>);
    impl Drop for Abort {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let _delay_task = Abort(delay_task);

    let queue: Shared = std::sync::Arc::new((Mutex::new(SoundQueue { pieces: Default::default(), done: false }), std::sync::Condvar::new()));
    let player = Player { child: player, queue: queue.clone() };
    let feeder = {
        let queue = queue.clone();
        std::thread::spawn(move || {
            let (lock, ready) = &*queue;
            let (mut played, mut skipped, mut dry) = (0u64, 0u64, 0u64);
            let mut low = usize::MAX;
            let mut since = Instant::now();
            let mut priming = true;
            loop {
                let piece = {
                    let mut q = lock.lock().unwrap();
                    // Start, and start again after running dry, with the target in hand.
                    while !q.done && (q.pieces.is_empty() || (priming && q.pieces.len() < SOUND_TARGET)) {
                        q = ready.wait(q).unwrap();
                    }
                    if q.done && q.pieces.is_empty() {
                        break;
                    }
                    priming = false;
                    low = low.min(q.pieces.len());
                    if since.elapsed() >= SOUND_WINDOW {
                        // Ahead all window long: skip what's above the target, a few pieces at most.
                        let extra = low.saturating_sub(SOUND_TARGET).min(3);
                        for _ in 0..extra {
                            q.pieces.pop_front();
                        }
                        skipped += extra as u64;
                        low = usize::MAX;
                        since = Instant::now();
                    }
                    while q.pieces.len() > SOUND_MAX {
                        q.pieces.pop_front();
                        skipped += 1;
                    }
                    let p = q.pieces.pop_front();
                    if q.pieces.is_empty() && !q.done {
                        priming = true;
                        dry += 1;
                    }
                    p
                };
                let Some(piece) = piece else { continue };
                // Blocks while pacat's own 40 ms is full: that's what paces this.
                if out.write_all(&piece).is_err() {
                    break;
                }
                played += 1;
            }
            (played, skipped, dry)
        })
    };

    let frame = 2 * channels;
    let piece = rate / 100 * frame;
    let mut buf: Vec<u8> = Vec::with_capacity(piece * 4);
    let mut chunk = vec![0u8; 16 * 1024];
    let result = loop {
        let n = match input.read(&mut chunk).await {
            Ok(0) => break Ok(()),
            Ok(n) => n,
            Err(e) => break Err(e),
        };
        buf.extend_from_slice(&chunk[..n]);
        // Whole 10 ms pieces only; a cut sample waits for the rest.
        let whole = buf.len() - buf.len() % piece;
        if whole > 0 {
            let (lock, ready) = &*queue;
            let mut q = lock.lock().unwrap();
            for p in buf[..whole].chunks(piece) {
                q.pieces.push_back(p.to_vec());
            }
            let loud = buf[..whole].chunks_exact(2).any(|b| i16::from_le_bytes([b[0], b[1]]).unsigned_abs() > AUDIBLE as u16);
            if loud {
                LAST_SOUND_MS.store(clock(), std::sync::atomic::Ordering::Relaxed);
            }
            ready.notify_one();
            drop(q);
            buf.drain(..whole);
        }
    };
    drop(player);
    let stats = tokio::task::spawn_blocking(move || feeder.join()).await.ok().and_then(|r| r.ok());
    if let Some((played, skipped, dry)) = stats {
        info!("{name}: phone's sound stopped ({played} pieces played, {skipped} skipped, ran dry {dry} times)");
    }
    result.map_err(Into::into)
}

/// The PC's default output's own delay (PipeWire's Latency on the sink node; a Bluetooth speaker
/// reports its codec's), in ms.
async fn output_delay_ms() -> u64 {
    let sink = match tokio::process::Command::new("pactl").arg("get-default-sink").output().await {
        Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        Err(_) => return 0,
    };
    let Ok(Ok(dump)) = tokio::time::timeout(Duration::from_secs(3), tokio::process::Command::new("pw-dump").kill_on_drop(true).output()).await else { return 0 };
    let Ok(Value::Array(objects)) = serde_json::from_slice::<Value>(&dump.stdout) else { return 0 };
    objects
        .iter()
        .find(|o| o["type"].as_str().is_some_and(|t| t.ends_with("Node")) && o["info"]["props"]["node.name"].as_str() == Some(sink.as_str()))
        .and_then(|o| o["info"]["params"]["Latency"].as_array()?.iter().find(|l| l["direction"] == "Input")?["maxNs"].as_u64())
        .map_or(0, |ns| ns / 1_000_000)
}

/// While the phone's sound plays: keeps [VIDEO_DELAY_MS] at the output's delay (another output may
/// be picked meanwhile); back to 0 when dropped.
struct DelayVideo;

impl DelayVideo {
    async fn follow(name: String) {
        let mut last = u64::MAX;
        loop {
            let ms = output_delay_ms().await + SOUND_PIPELINE_MS;
            if ms != last {
                info!("{name}: picture held back {ms} ms to match the sound");
                last = ms;
            }
            VIDEO_DELAY_MS.store(ms, std::sync::atomic::Ordering::Relaxed);
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    }
}

impl Drop for DelayVideo {
    fn drop(&mut self) {
        VIDEO_DELAY_MS.store(0, std::sync::atomic::Ordering::Relaxed);
    }
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
    if key == "Escape" {
        return Some(json!({ "kind": "back" }));
    }
    if key == "Tab" && !held.ctrl && !held.alt {
        return Some(json!({ "kind": "text", "text": "\t" }));
    }
    let named = match key {
        "BackSpace" => "backspace",
        "Delete" | "KP_Delete" => "delete",
        "Return" | "KP_Enter" => "enter",
        "Left" | "KP_Left" => "left",
        "Right" | "KP_Right" => "right",
        "Up" | "KP_Up" => "up",
        "Down" | "KP_Down" => "down",
        "Home" | "KP_Home" => "home",
        "End" | "KP_End" => "end",
        _ => "",
    };
    let ctrl = held.ctrl || modifiers.contains(M::CONTROL_MASK);
    let shift = held.shift || modifiers.contains(M::SHIFT_MASK);
    if !named.is_empty() && !held.alt {
        return Some(json!({ "kind": "key", "key": named, "ctrl": ctrl, "shift": shift }));
    }
    // Editing shortcuts, worked out on the phone.
    if ctrl && !held.alt {
        let short = match key {
            "a" | "A" => "selectall",
            "c" | "C" => "copy",
            "x" | "X" => "cut",
            _ => "",
        };
        if !short.is_empty() {
            return Some(json!({ "kind": "key", "key": short }));
        }
    }
    // Ctrl+V / Shift+Insert: the PC's clipboard, typed into the phone (the companion reads it).
    if (held.ctrl && !held.alt && matches!(key, "v" | "V")) || (held.shift && key == "Insert") {
        return Some(json!({ "kind": "paste" }));
    }
    // Shortcuts aren't typing.
    if held.ctrl || held.alt || modifiers.intersects(M::CONTROL_MASK | M::MOD1_MASK | M::SUPER_MASK | M::META_MASK) {
        return None;
    }
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
        let ctrl = Held { ctrl: true, ..Held::default() };
        assert_eq!(key_input("v", none, &ctrl).unwrap()["kind"], "paste");
        assert_eq!(key_input("a", none, &ctrl).unwrap()["key"], "selectall");
        let wipe = key_input("BackSpace", none, &ctrl).unwrap();
        assert_eq!((wipe["key"].as_str(), wipe["ctrl"].as_bool()), (Some("backspace"), Some(true)));
        assert_eq!(key_input("Left", none, &shift).unwrap()["shift"], true);
        assert_eq!(key_input("c", none, &ctrl).unwrap()["key"], "copy");
        assert!(key_input("q", none, &ctrl).is_none());
        let mut h = Held::default();
        assert!(h.track("Shift_L", true) && h.shift);
        assert!(h.track("Shift_L", false) && !h.shift);
        assert!(!h.track("a", true));
    }
}

#[cfg(test)]
mod live {
    /// This PC's output delay as the mirror reads it (a Bluetooth speaker reports ~190 ms).
    #[tokio::test]
    #[ignore]
    async fn reads_the_outputs_delay() {
        println!("output delay: {} ms", super::output_delay_ms().await);
    }
}
