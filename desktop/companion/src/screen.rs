//! Live PC screen on the phone. The ScreenCast portal hands us one monitor as a PipeWire stream
//! (Plasma asks which screen the first time; after that a restore token skips the dialog), and
//! GStreamer encodes it on the Intel GPU (VA-API H.264) into length-prefixed NAL units
//! (4-byte big-endian sizes; SPS/PPS before every keyframe). That goes to
//! the phone as an open-ended payload (`payloadSize` -1) over the usual pinned-TLS socket, and the
//! phone decodes it in hardware. Closing the stream on either side ends the cast.

use std::collections::HashMap;
use std::process::Stdio;

use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use serde_json::json;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tracing::{info, warn};
use zbus::zvariant::{self, OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, MatchRule, MessageStream};

use crate::identity::Identity;
use crate::link::{self, Sender};
use crate::payload;

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SCREENCAST: &str = "org.freedesktop.portal.ScreenCast";
const REMOTE: &str = "org.freedesktop.portal.RemoteDesktop";
/// Linux input button codes (BTN_LEFT, BTN_RIGHT).
const BTN_LEFT: i32 = 0x110;
const BTN_RIGHT: i32 = 0x111;
/// The phone's screen is about 1080 px tall in landscape: more than 1920 wide is wasted bits.
const MAX_WIDTH: i32 = 1920;
const BITRATE_KBPS: u32 = 8000;
/// The encoder's buffer (CPB), in kbit: about 60 ms of the peak rate. Its default allows frames of
/// hundreds of KB when the whole screen changes at once (a desktop switch, a window sliding in),
/// and each one stalls the stream for its transfer time; capped, such a frame is just blurrier.
const CPB_KBITS: u32 = 600;
/// The longest gap the encoder allows between keyframes. TCP never loses one, so they're only
/// needed to start; every one is a burst, so as few as possible.
const KEYFRAME_FRAMES: u32 = 1024;
/// The most frames a second the cast sends: the phone asks for what its screen and decoder keep up
/// with (`maxFps`), up to this. KWin's cap delays changes, not just thins them: at 60 a typed letter
/// reached the encoder ~30 ms later than uncapped (typing_reaches_the_encoder, 2026-11-04), so the
/// higher the better while the phone keeps up.
const MAX_FPS: u32 = 120;
/// For a phone that doesn't say (an older Vyoma), and the least any phone gets.
const DEFAULT_FPS: u32 = 60;
const MIN_FPS: u32 = 30;

/// The rate a phone asked for, within what the cast allows.
pub fn fps_for(asked: Option<u64>) -> u32 {
    asked.map_or(DEFAULT_FPS, |f| (f.min(MAX_FPS as u64) as u32).max(MIN_FPS))
}
const RELAY_BYTES: usize = 16 * 1024;
/// The encoder's output pipe (64 KiB by default, most of a second when the picture is still).
const PIPE_BYTES: libc::c_int = 16 * 1024;
const STAMP_MAGIC: &[u8; 4] = b"VYTS";

/// Casts until the phone hangs up (or the stream fails). Failures before streaming starts reach the
/// phone as an error packet, so it never waits on a spinner.
pub async fn stream(sender: &Sender, me: &Identity, pinned: &[u8], name: &str, fps: u32, inputs: tokio::sync::mpsc::UnboundedReceiver<serde_json::Value>) -> Result<()> {
    // VYOMA_SCREEN_TEST=1 casts a moving test pattern instead, with no portal: for checking the
    // phone's viewer (and screenshotting it) without anything private on the screen.
    let test = std::env::var_os("VYOMA_SCREEN_TEST").is_some();
    let cast = match if test { Ok(None) } else { Cast::start().await.map(Some) } {
        Ok(c) => c,
        Err(e) => {
            sender.send(&link::packet("vyoma.screen", json!({ "error": format!("{e:#}") }))).await?;
            return Err(e);
        }
    };
    let (w, h) = cast.as_ref().and_then(|c| c.size).map(fit).unwrap_or((MAX_WIDTH, MAX_WIDTH * 9 / 16));
    let control = cast.as_ref().and_then(|c| Some(Pointer { conn: c.conn.clone(), session: c.control.clone()?, node: c.node, size: c.size? }));
    let (conn, mut gst) = match cast {
        Some(Cast { conn, node, fd, .. }) => (Some(conn), encoder(&format!("pipewiresrc fd=0 path={node} do-timestamp=true keepalive-time=1000"), Some(fd), w, h, fps)?),
        None => (None, encoder(&format!("videotestsrc is-live=true pattern=smpte horizontal-speed=4 ! capsfilter caps=video/x-raw,width={w},height={h},framerate={fps}/1"), None, w, h, fps)?),
    };
    let encoded = gst.stdout.take().context("encoder has no output")?;
    {
        use std::os::fd::AsRawFd;
        // SAFETY: resizing a pipe we own; failure just leaves the default size.
        unsafe { libc::fcntl(encoded.as_raw_fd(), libc::F_SETPIPE_SZ, PIPE_BYTES) };
    }
    // Small windows all the way from the encoder to the socket (pipe, relay, unsent bytes): if the
    // link falls behind, the encoder blocks and the leaky queue before it drops raw frames, instead
    // of video piling up on the way.
    let (video, relay) = tokio::io::duplex(RELAY_BYTES);
    let stamping = tokio::spawn(stamp(encoded, relay));
    let mut errors = gst.stderr.take().context("encoder has no error output")?;
    info!("{name}: casting the screen at {w}x{h}");
    let announce = link::packet("vyoma.screen", json!({ "codec": "h264", "framing": "avc4", "width": w, "height": h, "control": control.is_some() }));
    // The phone's touches, as the pointer on this screen, while the cast runs.
    let touching = control.map(|p| tokio::spawn(p.follow(inputs, sender.clone())));
    let result = payload::offer_stream(sender, me, pinned, announce, video).await;
    if let Some(t) = touching {
        t.abort();
    }
    let _ = gst.kill().await;
    stamping.abort();
    if let Err(e) = &result {
        let mut log = String::new();
        let _ = errors.read_to_string(&mut log).await;
        if !log.trim().is_empty() {
            warn!("encoder said: {}", log.trim());
        }
        info!("{name}: cast ended ({e:#})");
    }
    // Dropping the portal connection closes the session, and Plasma's "sharing" indicator with it.
    drop(conn);
    Ok(())
}

/// Passes the encoder's length-prefixed NAL units through, putting a timestamp unit before each
/// picture: an SEI-typed NAL with our marker and the send time (µs since the epoch), which the phone
/// reads for its latency figures and removes before decoding.
async fn stamp(mut encoded: impl AsyncRead + Unpin, mut out: impl AsyncWrite + Unpin) -> Result<()> {
    let mut nal = Vec::with_capacity(256 * 1024);
    loop {
        let size = encoded.read_u32().await? as usize;
        if size == 0 || size > 16 << 20 {
            bail!("bad NAL size {size}");
        }
        nal.resize(size, 0);
        encoded.read_exact(&mut nal).await?;
        if matches!(nal[0] & 0x1f, 1 | 5) {
            let us = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_micros() as u64).unwrap_or(0);
            let mut ts = Vec::with_capacity(17);
            ts.extend_from_slice(&13u32.to_be_bytes());
            ts.push(0x06);
            ts.extend_from_slice(STAMP_MAGIC);
            ts.extend_from_slice(&us.to_be_bytes());
            out.write_all(&ts).await?;
        }
        out.write_all(&(size as u32).to_be_bytes()).await?;
        out.write_all(&nal).await?;
    }
}

/// Scales to fit [MAX_WIDTH] keeping the shape; even sizes, as H.264 wants.
fn fit((w, h): (i32, i32)) -> (i32, i32) {
    let (w, h) = if w > MAX_WIDTH { (MAX_WIDTH, (h as i64 * MAX_WIDTH as i64 / w as i64) as i32) } else { (w, h) };
    (w & !1, h & !1)
}

fn encoder(source: &str, fd: Option<zvariant::OwnedFd>, w: i32, h: i32, fps: u32) -> Result<tokio::process::Child> {
    // Latency first. The capture has a variable frame rate (KWin sends one per change, up to the
    // monitor's 200 Hz); `max-framerate` asks KWin itself to send no more than `fps` (videorate
    // can't cap it: the capture's timestamps let it through at ~100). At most one frame waits
    // before the encoder, no B-frames, the fastest preset, a small buffer and rare keyframes so
    // no frame is a burst, and length-prefixed output ("avc"), one picture per write, so
    // the phone can show each frame the moment it lands.
    // VYOMA_SCREEN_CAPS: other caps after the capture, for latency experiments.
    let caps = std::env::var("VYOMA_SCREEN_CAPS").unwrap_or_else(|_| format!("video/x-raw(ANY),max-framerate={fps}/1"));
    let pipeline = format!(
        "{source} \
         ! {caps} \
         ! queue max-size-buffers=1 leaky=downstream \
         ! vapostproc ! video/x-raw(memory:VAMemory),format=NV12,width={w},height={h} \
         ! vah264enc rate-control=vbr bitrate={BITRATE_KBPS} cpb-size={CPB_KBITS} key-int-max={KEYFRAME_FRAMES} b-frames=0 num-slices=1 target-usage=7 \
         ! h264parse config-interval=-1 ! video/x-h264,stream-format=avc,alignment=au \
         ! fdsink fd=1 sync=false",
    );
    tokio::process::Command::new("gst-launch-1.0")
        .arg("-q")
        .args(pipeline.split_whitespace())
        // The portal's PipeWire connection, handed over as the encoder's stdin (pipewiresrc fd=0).
        .stdin(fd.map_or_else(Stdio::null, |fd| Stdio::from(std::os::fd::OwnedFd::from(fd))))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("GStreamer (gst-launch-1.0) isn't installed")
}

/// Touches from the phone's view of this screen (`vyoma.screen.input`), performed through the
/// remote-desktop session: x, y are 0..1 across the picture. Kinds: "click" and "right" (a tap,
/// a long press), "down" / "move" / "up" (a drag with the left button), "scroll" (dx, dy in
/// pixels of smooth scrolling) and "scrollend".
pub struct Pointer {
    conn: Connection,
    session: OwnedObjectPath,
    node: u32,
    size: (i32, i32),
}

impl Pointer {
    async fn follow(self, mut inputs: tokio::sync::mpsc::UnboundedReceiver<serde_json::Value>, sender: Sender) {
        while let Some(mut i) = inputs.recv().await {
            // Behind? Only the newest of a run of moves matters.
            while i["kind"] == "move" {
                match inputs.try_recv() {
                    Ok(next) if next["kind"] == "move" => i = next,
                    Ok(next) => {
                        let _ = self.perform(&i).await;
                        i = next;
                        break;
                    }
                    Err(_) => break,
                }
            }
            if let Err(e) = self.perform(&i).await {
                tracing::debug!("pointer: {e:#}");
                // Text that didn't all arrive: the phone's field no longer matches, so it starts afresh.
                if i["kind"] == "text" {
                    let _ = sender.send(&link::packet("vyoma.paste.ready", json!({ "error": format!("{e:#}") }))).await;
                }
            }
        }
    }

    /// Keys and buttons wait for the portal's answer, one after another: it handles calls
    /// concurrently, so a release sent straight after its press could overtake it, be dropped, and
    /// leave the key held and repeating (letters repeated, 2026-11-04).
    async fn call<B: serde::Serialize + zvariant::DynamicType>(&self, method: &str, body: &B) -> Result<()> {
        self.conn.call_method(Some(PORTAL), PORTAL_PATH, Some(REMOTE), method, body).await?;
        Ok(())
    }

    /// Moves and scrolls don't wait (each would cost a round trip through the portal and KWin):
    /// one overtaking another changes nothing that the next one doesn't fix.
    async fn call_fast<B: serde::Serialize + zvariant::DynamicType>(&self, method: &str, body: &B) -> Result<()> {
        let msg = zbus::message::Message::method_call(PORTAL_PATH, method)?
            .destination(PORTAL)?
            .interface(REMOTE)?
            .with_flags(zbus::message::Flags::NoReplyExpected)?
            .build(body)?;
        self.conn.send(&msg).await?;
        Ok(())
    }

    /// Moves the pointer; [wait] when a click follows, so the click can't land before the move.
    async fn to(&self, i: &serde_json::Value, wait: bool) -> Result<()> {
        let x = i["x"].as_f64().unwrap_or(0.5).clamp(0.0, 1.0) * self.size.0 as f64;
        let y = i["y"].as_f64().unwrap_or(0.5).clamp(0.0, 1.0) * self.size.1 as f64;
        let body = (&self.session, opts([]), self.node, x, y);
        if wait { self.call("NotifyPointerMotionAbsolute", &body).await } else { self.call_fast("NotifyPointerMotionAbsolute", &body).await }
    }

    /// One key, by its evdev code: real key presses, which the desktop handles like a keyboard's.
    async fn key(&self, code: i32, down: bool) -> Result<()> {
        self.call("NotifyKeyboardKeycode", &(&self.session, opts([]), code, down as u32)).await
    }

    async fn tap(&self, code: i32) -> Result<()> {
        self.key(code, true).await?;
        self.key(code, false).await
    }

    /// Typed text: each character by the key that types it, and what no key types (emoji, other
    /// scripts) pasted, in order.
    async fn type_text(&self, text: &str) -> Result<()> {
        let mut unkeyed = String::new();
        for c in text.chars() {
            if c != '\n' && !crate::mirror::layout().keys.contains_key(&(char_keysym(c) as u32)) {
                unkeyed.push(c);
                continue;
            }
            self.paste(&mut unkeyed).await?;
            self.type_char(c).await?;
        }
        self.paste(&mut unkeyed).await
    }

    async fn paste(&self, text: &mut String) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        crate::paste::put(text).await?;
        text.clear();
        let Some(v) = keycode("v") else { return Ok(()) };
        self.key(KEY_LEFTCTRL, true).await?;
        self.tap(v).await?;
        self.key(KEY_LEFTCTRL, false).await?;
        // The app fetches the text on its own time; a key sent now could land before it.
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        Ok(())
    }

    /// One character by the key that types it in the desktop's layout, with Shift if that's how it's reached.
    async fn type_char(&self, c: char) -> Result<()> {
        if c == '\n' {
            return self.tap(KEY_ENTER).await;
        }
        let sym = char_keysym(c);
        match crate::mirror::layout().keys.get(&(sym as u32)) {
            Some(&(code, shift)) => {
                if shift {
                    self.key(KEY_LEFTSHIFT, true).await?;
                }
                self.tap(code as i32).await?;
                if shift {
                    self.key(KEY_LEFTSHIFT, false).await?;
                }
                Ok(())
            }
            None => Ok(()),
        }
    }

    pub async fn button(&self, button: i32, down: bool) -> Result<()> {
        self.call("NotifyPointerButton", &(&self.session, opts([]), button, down as u32)).await
    }

    /// The touchpad's moves (pointer.rs): by a distance, not to a place.
    pub async fn relative(&self, dx: f64, dy: f64) -> Result<()> {
        self.call_fast("NotifyPointerMotion", &(&self.session, opts([]), dx, dy)).await
    }

    pub async fn axis(&self, dx: f64, dy: f64) -> Result<()> {
        self.call_fast("NotifyPointerAxis", &(&self.session, opts([]), dx, dy)).await
    }

    /// Returns once the portal has dealt with the moves before it.
    pub async fn settle(&self) -> Result<()> {
        self.call("NotifyPointerMotion", &(&self.session, opts([]), 0.0f64, 0.0f64)).await
    }

    async fn perform(&self, i: &serde_json::Value) -> Result<()> {
        match i["kind"].as_str().unwrap_or("") {
            "click" | "right" => {
                let b = if i["kind"] == "right" { BTN_RIGHT } else { BTN_LEFT };
                self.to(i, true).await?;
                self.button(b, true).await?;
                self.button(b, false).await
            }
            "down" => {
                self.to(i, true).await?;
                self.button(BTN_LEFT, true).await
            }
            "move" => self.to(i, false).await,
            "up" => {
                self.to(i, true).await?;
                self.button(BTN_LEFT, false).await
            }
            "scroll" => {
                self.to(i, true).await?;
                let (dx, dy) = (i["dx"].as_f64().unwrap_or(0.0), i["dy"].as_f64().unwrap_or(0.0));
                self.call_fast("NotifyPointerAxis", &(&self.session, opts([]), dx, dy)).await
            }
            "text" => {
                let text: String = i["text"].as_str().unwrap_or("").chars().take(4096).collect();
                self.type_text(&text).await
            }
            "key" => {
                let name = i["key"].as_str().unwrap_or("");
                let Some(code) = keycode(name) else { return Ok(()) };
                let mods: Vec<i32> = i["mods"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|m| match m.as_str()? {
                        "ctrl" => Some(KEY_LEFTCTRL),
                        "alt" => Some(KEY_LEFTALT),
                        "super" => Some(KEY_LEFTMETA),
                        "shift" => Some(KEY_LEFTSHIFT),
                        _ => None,
                    })
                    .collect();
                for m in &mods {
                    self.key(*m, true).await?;
                }
                self.tap(code).await?;
                for m in mods.iter().rev() {
                    self.key(*m, false).await?;
                }
                Ok(())
            }
            "scrollend" => self.call("NotifyPointerAxis", &(&self.session, opts([("finish", Value::Bool(true))]), 0.0f64, 0.0f64)).await,
            _ => Ok(()),
        }
    }
}

/// evdev key codes (linux/input-event-codes.h).
const KEY_LEFTSHIFT: i32 = 42;
const KEY_LEFTCTRL: i32 = 29;
const KEY_LEFTALT: i32 = 56;
const KEY_LEFTMETA: i32 = 125;
const KEY_ENTER: i32 = 28;

/// The keysym that types [c]: Latin-1 is its own code, the rest Unicode's (0x0100_0000 + code).
fn char_keysym(c: char) -> i32 {
    let u = c as u32;
    if (0x20..=0x7e).contains(&u) || (0xa0..=0xff).contains(&u) { u as i32 } else { (0x0100_0000 | u) as i32 }
}

/// The phone keyboard's named keys, and single characters for shortcuts (Ctrl+C presses C).
fn keycode(name: &str) -> Option<i32> {
    Some(match name {
        "BackSpace" => 14,
        "Tab" => 15,
        "Return" => KEY_ENTER,
        "Escape" => 1,
        "Delete" => 111,
        "Home" => 102,
        "End" => 107,
        "Left" => 105,
        "Right" => 106,
        "Up" => 103,
        "Down" => 108,
        "PageUp" => 104,
        "PageDown" => 109,
        "Super" => KEY_LEFTMETA,
        one if one.chars().count() == 1 => {
            let c = one.chars().next()?.to_ascii_lowercase();
            crate::mirror::layout().keys.get(&(char_keysym(c) as u32))?.0 as i32
        }
        _ => return None,
    })
}

struct Cast {
    conn: Connection,
    node: u32,
    fd: zvariant::OwnedFd,
    size: Option<(i32, i32)>,
    /// A remote-desktop session: the phone's touches can work the pointer on this screen.
    control: Option<OwnedObjectPath>,
}

/// A session for the pointer alone, no screen: Plasma asks once, then its own token reopens it.
pub async fn pointer_session() -> Result<Pointer> {
    let conn = Connection::session().await.context("no session bus")?;
    let portal = Portal::new(&conn)?;
    let created = portal
        .request_on(REMOTE, "CreateSession", |t| (opts([("handle_token", t.to_string().into()), ("session_handle_token", "vyomapointer".into())]),))
        .await?;
    let session: OwnedObjectPath = created
        .get("session_handle")
        .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
        .and_then(|s| OwnedObjectPath::try_from(s).ok())
        .context("the portal made no session")?;
    let token = std::fs::read_to_string(pointer_token_path()).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    let s = session.clone();
    portal
        .request_on(REMOTE, "SelectDevices", move |t| {
            let mut o = opts([
                ("handle_token", t.to_string().into()),
                ("types", Value::U32(2)), // pointer
                ("persist_mode", Value::U32(2)),
            ]);
            if let Some(tok) = &token {
                o.insert("restore_token", tok.clone().into());
            }
            (s.clone(), o)
        })
        .await?;
    let s = session.clone();
    let started = portal.request_on(REMOTE, "Start", move |t| (s.clone(), "", opts([("handle_token", t.to_string().into())]))).await?;
    if let Some(tok) = started.get("restore_token").and_then(|v| String::try_from(v.try_clone().ok()?).ok()) {
        let _ = crate::identity::write_private(&pointer_token_path(), tok.as_bytes());
    }
    let devices: u32 = started.get("devices").and_then(|v| u32::try_from(v.try_clone().ok()?).ok()).unwrap_or(0);
    anyhow::ensure!(devices & 2 != 0, "the PC didn't allow pointer control");
    Ok(Pointer { conn, session, node: 0, size: (0, 0) })
}

impl Cast {
    /// Remote desktop first (the screen plus the pointer, for touch control); plain screen sharing
    /// if the portal can't, so watching still works. Cancelling on the PC cancels both.
    async fn start() -> Result<Cast> {
        match Self::remote().await {
            Ok(c) => Ok(c),
            Err(e) if format!("{e:#}").contains("cancelled") => Err(e),
            Err(e) => {
                warn!("no remote control ({e:#}); sharing the screen to watch only");
                Self::watch().await
            }
        }
    }

    /// RemoteDesktop with its ScreenCast: Plasma asks once (screen, and allowing control), then the
    /// restore token reopens it. The pointer is moved in the stream's own coordinates.
    async fn remote() -> Result<Cast> {
        let conn = Connection::session().await.context("no session bus")?;
        let portal = Portal::new(&conn)?;
        let created = portal
            .request_on(REMOTE, "CreateSession", |t| (opts([("handle_token", t.to_string().into()), ("session_handle_token", "vyomarc".into())]),))
            .await?;
        let session: OwnedObjectPath = created
            .get("session_handle")
            .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
            .and_then(|s| OwnedObjectPath::try_from(s).ok())
            .context("the portal made no session")?;
        let token = std::fs::read_to_string(remote_token_path()).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
        let s = session.clone();
        portal
            .request_on(REMOTE, "SelectDevices", move |t| {
                let mut o = opts([
                    ("handle_token", t.to_string().into()),
                    ("types", Value::U32(1 | 2)), // keyboard, pointer
                    ("persist_mode", Value::U32(2)),
                ]);
                if let Some(tok) = &token {
                    o.insert("restore_token", tok.clone().into());
                }
                (s.clone(), o)
            })
            .await?;
        let cursors: u32 = portal.property("AvailableCursorModes").await.unwrap_or(1);
        let s = session.clone();
        portal
            .request("SelectSources", move |t| {
                (
                    s.clone(),
                    opts([
                        ("handle_token", t.to_string().into()),
                        ("types", Value::U32(1)),
                        ("multiple", Value::Bool(false)),
                        ("cursor_mode", Value::U32(if cursors & 2 != 0 { 2 } else { 1 })),
                    ]),
                )
            })
            .await?;
        let s = session.clone();
        let started = portal.request_on(REMOTE, "Start", move |t| (s.clone(), "", opts([("handle_token", t.to_string().into())]))).await?;
        if let Some(tok) = started.get("restore_token").and_then(|v| String::try_from(v.try_clone().ok()?).ok()) {
            let _ = crate::identity::write_private(&remote_token_path(), tok.as_bytes());
        }
        let devices: u32 = started.get("devices").and_then(|v| u32::try_from(v.try_clone().ok()?).ok()).unwrap_or(0);
        let streams: Vec<(u32, HashMap<String, OwnedValue>)> =
            started.get("streams").context("no screen was shared")?.try_clone()?.try_into().context("unexpected stream list")?;
        let (node, props) = streams.into_iter().next().context("no screen was shared")?;
        let size = props.get("size").and_then(|v| <(i32, i32)>::try_from(v.try_clone().ok()?).ok());
        let reply = conn
            .call_method(Some(PORTAL), PORTAL_PATH, Some(SCREENCAST), "OpenPipeWireRemote", &(&session, opts([])))
            .await
            .context("the portal wouldn't open the stream")?;
        let fd: zvariant::OwnedFd = reply.body().deserialize()?;
        let control = (devices & 2 != 0).then_some(session);
        Ok(Cast { conn, node, fd, size, control })
    }

    async fn watch() -> Result<Cast> {
        let conn = Connection::session().await.context("no session bus")?;
        let portal = Portal::new(&conn)?;

        let created = portal.request("CreateSession", |t| (opts([("handle_token", t.to_string().into()), ("session_handle_token", "vyoma".into())]),)).await?;
        let session: OwnedObjectPath = created
            .get("session_handle")
            .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
            .and_then(|s| OwnedObjectPath::try_from(s).ok())
            .context("the portal made no session")?;

        let cursors: u32 = portal.property("AvailableCursorModes").await.unwrap_or(1);
        let token = std::fs::read_to_string(token_path()).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
        let s = session.clone();
        portal
            .request("SelectSources", move |t| {
                let mut o = opts([
                    ("handle_token", t.to_string().into()),
                    ("types", Value::U32(1)), // monitors
                    ("multiple", Value::Bool(false)),
                    // Embedded (2) draws the pointer into the picture; else hidden (1).
                    ("cursor_mode", Value::U32(if cursors & 2 != 0 { 2 } else { 1 })),
                    // Until revoked: Plasma asks once, then the token reopens the same screen.
                    ("persist_mode", Value::U32(2)),
                ]);
                if let Some(tok) = &token {
                    o.insert("restore_token", tok.clone().into());
                }
                (s.clone(), o)
            })
            .await?;

        let s = session.clone();
        let started = portal.request("Start", move |t| (s.clone(), "", opts([("handle_token", t.to_string().into())]))).await?;
        if let Some(tok) = started.get("restore_token").and_then(|v| String::try_from(v.try_clone().ok()?).ok()) {
            let _ = crate::identity::write_private(&token_path(), tok.as_bytes());
        }
        let streams: Vec<(u32, HashMap<String, OwnedValue>)> =
            started.get("streams").context("no screen was shared")?.try_clone()?.try_into().context("unexpected stream list")?;
        let (node, props) = streams.into_iter().next().context("no screen was shared")?;
        let size = props.get("size").and_then(|v| <(i32, i32)>::try_from(v.try_clone().ok()?).ok());

        let reply = conn
            .call_method(Some(PORTAL), PORTAL_PATH, Some(SCREENCAST), "OpenPipeWireRemote", &(&session, opts([])))
            .await
            .context("the portal wouldn't open the stream")?;
        let fd: zvariant::OwnedFd = reply.body().deserialize()?;
        Ok(Cast { conn, node, fd, size, control: None })
    }
}

fn opts<const N: usize>(entries: [(&'static str, Value<'static>); N]) -> HashMap<&'static str, Value<'static>> {
    entries.into_iter().collect()
}

fn remote_token_path() -> std::path::PathBuf {
    crate::identity::dir().join("remotedesktop-token")
}

fn pointer_token_path() -> std::path::PathBuf {
    crate::identity::dir().join("pointer-token")
}

fn token_path() -> std::path::PathBuf {
    crate::identity::dir().join("screencast-token")
}

/// xdg-desktop-portal's request/response dance: each call returns at once, and the answer
/// arrives later as a Response signal on a request object whose path we can predict.
struct Portal<'a> {
    conn: &'a Connection,
    sender: String,
}

impl<'a> Portal<'a> {
    fn new(conn: &'a Connection) -> Result<Self> {
        let unique = conn.unique_name().context("not on the bus")?;
        Ok(Portal { conn, sender: unique.trim_start_matches(':').replace('.', "_") })
    }

    async fn property<T: TryFrom<OwnedValue>>(&self, name: &str) -> Option<T> {
        self.property_of(SCREENCAST, name).await
    }

    async fn property_of<T: TryFrom<OwnedValue>>(&self, iface: &str, name: &str) -> Option<T> {
        let reply = self
            .conn
            .call_method(Some(PORTAL), PORTAL_PATH, Some("org.freedesktop.DBus.Properties"), "Get", &(iface, name))
            .await
            .ok()?;
        let v: OwnedValue = reply.body().deserialize().ok()?;
        T::try_from(v).ok()
    }

    async fn request<B>(&self, method: &str, body: impl FnOnce(&str) -> B) -> Result<HashMap<String, OwnedValue>>
    where
        B: serde::Serialize + zvariant::DynamicType,
    {
        self.request_on(SCREENCAST, method, body).await
    }

    async fn request_on<B>(&self, iface: &str, method: &str, body: impl FnOnce(&str) -> B) -> Result<HashMap<String, OwnedValue>>
    where
        B: serde::Serialize + zvariant::DynamicType,
    {
        let token = format!("vyoma{}", std::process::id() ^ rand_u32());
        let path = format!("{PORTAL_PATH}/request/{}/{token}", self.sender);
        // Listen before calling, or a fast answer is missed.
        let rule = MatchRule::builder().msg_type(zbus::message::Type::Signal).interface("org.freedesktop.portal.Request")?.member("Response")?.path(path.as_str())?.build();
        let mut responses = MessageStream::for_match_rule(rule, self.conn, Some(1)).await?;
        self.conn.call_method(Some(PORTAL), PORTAL_PATH, Some(iface), method, &body(&token)).await.with_context(|| format!("the screen-sharing portal refused {method}"))?;
        let msg = responses.next().await.context("the portal went away")??;
        let (code, results): (u32, HashMap<String, OwnedValue>) = msg.body().deserialize()?;
        match code {
            0 => Ok(results),
            1 => bail!("screen sharing was cancelled on the PC"),
            _ => bail!("the PC couldn't share its screen"),
        }
    }
}

fn rand_u32() -> u32 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How long a typed letter takes to come out of the encoder as a frame: types single letters
    /// into KWrite (as [types_into_a_test_window]) while casting, and prints each delay.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn typing_reaches_the_encoder() {
        let file = std::env::var("VYOMA_TYPE_FILE").expect("VYOMA_TYPE_FILE");
        let focus = std::env::var("VYOMA_FOCUS").expect("VYOMA_FOCUS");
        std::fs::write(&file, "").unwrap();
        let cast = Cast::remote().await.expect("remote desktop session");
        let p = Pointer { conn: cast.conn.clone(), session: cast.control.clone().expect("keyboard allowed"), node: cast.node, size: cast.size.unwrap() };
        let (w, h) = fit(cast.size.unwrap());
        let mut editor = std::process::Command::new("kwrite").arg(&file).spawn().unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        let said = String::from_utf8_lossy(&std::process::Command::new(&focus).output().unwrap().stdout).to_string();
        if !said.contains("VYOMA_ACTIVE:test") {
            let _ = editor.kill();
            panic!("the test window didn't get focus; typed nothing");
        }
        let mut gst = encoder(&format!("pipewiresrc fd=0 path={} do-timestamp=true keepalive-time=1000", cast.node), Some(cast.fd), w, h, MAX_FPS).unwrap();
        let mut out = gst.stdout.take().unwrap();
        let frames = std::sync::Arc::new(std::sync::Mutex::new(Vec::<(std::time::Instant, usize)>::new()));
        let seen = frames.clone();
        tokio::spawn(async move {
            let mut nal = Vec::new();
            loop {
                let Ok(size) = out.read_u32().await else { return };
                nal.resize(size as usize, 0);
                if out.read_exact(&mut nal).await.is_err() { return }
                if matches!(nal[0] & 0x1f, 1 | 5) { seen.lock().unwrap().push((std::time::Instant::now(), nal.len())); }
            }
        });
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let mut delays = Vec::new();
        for i in 0..16u64 {
            tokio::time::sleep(std::time::Duration::from_millis(600 + i * 37)).await;
            let t0 = std::time::Instant::now();
            p.type_char((b'a' + i as u8) as char).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(700)).await;
            let around: Vec<(i64, usize)> = frames.lock().unwrap().iter()
                .map(|(t, n)| (if *t >= *&t0 { t.duration_since(t0).as_millis() as i64 } else { -(t0.duration_since(*t).as_millis() as i64) }, *n))
                .filter(|(d, _)| (-60..300).contains(d)).collect();
            delays.push(around);
        }
        let _ = editor.kill();
        let _ = gst.kill().await;
        for d in &delays { println!("frames around a letter, ms: {d:?}"); }
    }

    #[test]
    fn characters_press_the_keys_that_type_them() {
        let keys = &crate::mirror::layout().keys;
        let key = |c: char| keys.get(&(char_keysym(c) as u32)).copied();
        assert_eq!(key('h'), Some((35, false)));
        assert_eq!(key('A'), Some((30, true)));
        assert_eq!(key('!'), Some((2, true)));
        assert_eq!(key(' '), Some((57, false)));
        assert_eq!(keycode("c"), Some(46));
        assert_eq!(keycode("Return"), Some(KEY_ENTER));
    }
}

#[cfg(test)]
mod live {
    use super::*;
    use std::time::Duration;

    /// Types into a KWrite window on this PC through a real remote-desktop session and reads the
    /// file back. Only types once KWin says the test window has focus (VYOMA_FOCUS: a script that
    /// prints VYOMA_ACTIVE:test). Run by hand, with the PC left alone.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn types_into_a_test_window() {
        let file = std::env::var("VYOMA_TYPE_FILE").expect("VYOMA_TYPE_FILE");
        let focus = std::env::var("VYOMA_FOCUS").expect("VYOMA_FOCUS");
        std::fs::write(&file, "").unwrap();
        let cast = Cast::remote().await.expect("remote desktop session");
        let p = Pointer { conn: cast.conn.clone(), session: cast.control.clone().expect("keyboard allowed"), node: cast.node, size: cast.size.unwrap() };
        let mut editor = std::process::Command::new("kwrite").arg(&file).spawn().unwrap();
        tokio::time::sleep(Duration::from_secs(3)).await;
        let said = String::from_utf8_lossy(&std::process::Command::new(&focus).output().unwrap().stdout).to_string();
        if !said.contains("VYOMA_ACTIVE:test") {
            let _ = editor.kill();
            panic!("the test window didn't get focus; typed nothing");
        }
        let text = std::env::var("VYOMA_TYPE_TEXT").unwrap_or_else(|_| "Hello, World! 123 abc".into());
        let t0 = std::time::Instant::now();
        p.perform(&json!({ "kind": "text", "text": text })).await.unwrap();
        println!("{} characters in {:?}", text.chars().count(), t0.elapsed());
        p.perform(&json!({ "kind": "key", "key": "s", "mods": ["ctrl"] })).await.unwrap();
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let _ = editor.kill();
        println!("typed: {:?}", std::fs::read_to_string(&file).unwrap());
    }
}
