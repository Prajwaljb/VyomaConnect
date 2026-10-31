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
use tokio::io::AsyncReadExt;
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
/// Matches the phone's 120 Hz screen; a 200 Hz monitor would otherwise send frames it can't show.
const MAX_FPS: u32 = 120;

/// Casts until the phone hangs up (or the stream fails). Failures before streaming starts reach the
/// phone as an error packet, so it never waits on a spinner.
pub async fn stream(sender: &Sender, me: &Identity, pinned: &[u8], name: &str) -> Result<()> {
    let cast = match Cast::start().await {
        Ok(c) => c,
        Err(e) => {
            sender.send(&link::packet("vyoma.screen", json!({ "error": format!("{e:#}") }))).await?;
            return Err(e);
        }
    };
    let Cast { conn, node, fd, size } = cast;
    let (w, h) = size.map(fit).unwrap_or((MAX_WIDTH, MAX_WIDTH * 9 / 16));
    let mut gst = encoder(node, fd, w, h)?;
    let video = gst.stdout.take().context("encoder has no output")?;
    let mut errors = gst.stderr.take().context("encoder has no error output")?;
    info!("{name}: casting the screen at {w}x{h}");
    let announce = link::packet("vyoma.screen", json!({ "codec": "h264", "framing": "avc4", "width": w, "height": h }));
    let result = payload::offer_stream(sender, me, pinned, announce, video).await;
    let _ = gst.kill().await;
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

/// Scales to fit [MAX_WIDTH] keeping the shape; even sizes, as H.264 wants.
fn fit((w, h): (i32, i32)) -> (i32, i32) {
    let (w, h) = if w > MAX_WIDTH { (MAX_WIDTH, (h as i64 * MAX_WIDTH as i64 / w as i64) as i32) } else { (w, h) };
    (w & !1, h & !1)
}

fn encoder(node: u32, fd: zvariant::OwnedFd, w: i32, h: i32) -> Result<tokio::process::Child> {
    // Latency first: drop (never queue) frames over MAX_FPS (KWin can deliver 200 on a 200 Hz
    // monitor), keep at most one frame waiting, no B-frames, the encoder's fastest preset, and a
    // keyframe only every 5 s at full rate (TCP never loses one; big keyframes cause stutters). Output is
    // length-prefixed ("avc"), one picture per write, so the phone shows each frame the moment it
    // lands instead of waiting for the next one to find where it ended.
    let pipeline = format!(
        "pipewiresrc fd=0 path={node} do-timestamp=true keepalive-time=1000 \
         ! videorate drop-only=true max-rate={MAX_FPS} \
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
        .stdin(Stdio::from(std::os::fd::OwnedFd::from(fd)))
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
