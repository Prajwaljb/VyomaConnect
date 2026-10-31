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
/// The phone's screen is about 1080 px tall in landscape: more than 1920 wide is wasted bits.
const MAX_WIDTH: i32 = 1920;
const BITRATE_KBPS: u32 = 8000;
/// Half the phone's 120 Hz: its screen always keeps up, so frames never queue on the way.
const MAX_FPS: u32 = 60;
const RELAY_BYTES: usize = 16 * 1024;
/// The encoder's output pipe (64 KiB by default, most of a second when the picture is still).
const PIPE_BYTES: libc::c_int = 16 * 1024;
const STAMP_MAGIC: &[u8; 4] = b"VYTS";

/// Casts until the phone hangs up (or the stream fails). Failures before streaming starts reach the
/// phone as an error packet, so it never waits on a spinner.
pub async fn stream(sender: &Sender, me: &Identity, pinned: &[u8], name: &str) -> Result<()> {
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
    let (conn, mut gst) = match cast {
        Some(Cast { conn, node, fd, .. }) => (Some(conn), encoder(&format!("pipewiresrc fd=0 path={node} do-timestamp=true keepalive-time=1000"), Some(fd), w, h)?),
        None => (None, encoder(&format!("videotestsrc is-live=true pattern=smpte horizontal-speed=4 ! capsfilter caps=video/x-raw,width={w},height={h},framerate={MAX_FPS}/1"), None, w, h)?),
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
    let announce = link::packet("vyoma.screen", json!({ "codec": "h264", "framing": "avc4", "width": w, "height": h }));
    let result = payload::offer_stream(sender, me, pinned, announce, video).await;
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

fn encoder(source: &str, fd: Option<zvariant::OwnedFd>, w: i32, h: i32) -> Result<tokio::process::Child> {
    // Latency first. The capture has a variable frame rate (KWin sends one per change, up to the
    // monitor's 200 Hz); `max-framerate` asks KWin itself to send no more than [MAX_FPS] (videorate
    // can't cap it: the capture's timestamps let it through at ~100). At most one frame waits
    // before the encoder, no B-frames, the fastest preset, a keyframe every 5 s (TCP never loses
    // one; big keyframes stutter), and length-prefixed output ("avc"), one picture per write, so
    // the phone can show each frame the moment it lands.
    let pipeline = format!(
        "{source} \
         ! video/x-raw(ANY),max-framerate={MAX_FPS}/1 \
         ! queue max-size-buffers=1 leaky=downstream \
         ! vapostproc ! video/x-raw(memory:VAMemory),format=NV12,width={w},height={h} \
         ! vah264enc rate-control=vbr bitrate={BITRATE_KBPS} key-int-max={keyframes} b-frames=0 num-slices=1 target-usage=7 \
         ! h264parse config-interval=-1 ! video/x-h264,stream-format=avc,alignment=au \
         ! fdsink fd=1 sync=false",
        keyframes = MAX_FPS * 5,
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

struct Cast {
    conn: Connection,
    node: u32,
    fd: zvariant::OwnedFd,
    size: Option<(i32, i32)>,
}

impl Cast {
    async fn start() -> Result<Cast> {
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
            let _ = std::fs::write(token_path(), tok);
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
        Ok(Cast { conn, node, fd, size })
    }
}

fn opts<const N: usize>(entries: [(&'static str, Value<'static>); N]) -> HashMap<&'static str, Value<'static>> {
    entries.into_iter().collect()
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
        let reply = self
            .conn
            .call_method(Some(PORTAL), PORTAL_PATH, Some("org.freedesktop.DBus.Properties"), "Get", &(SCREENCAST, name))
            .await
            .ok()?;
        let v: OwnedValue = reply.body().deserialize().ok()?;
        T::try_from(v).ok()
    }

    async fn request<B>(&self, method: &str, body: impl FnOnce(&str) -> B) -> Result<HashMap<String, OwnedValue>>
    where
        B: serde::Serialize + zvariant::DynamicType,
    {
        let token = format!("vyoma{}", std::process::id() ^ rand_u32());
        let path = format!("{PORTAL_PATH}/request/{}/{token}", self.sender);
        // Listen before calling, or a fast answer is missed.
        let rule = MatchRule::builder().msg_type(zbus::message::Type::Signal).interface("org.freedesktop.portal.Request")?.member("Response")?.path(path.as_str())?.build();
        let mut responses = MessageStream::for_match_rule(rule, self.conn, Some(1)).await?;
        self.conn.call_method(Some(PORTAL), PORTAL_PATH, Some(SCREENCAST), method, &body(&token)).await.with_context(|| format!("the screen-sharing portal refused {method}"))?;
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
