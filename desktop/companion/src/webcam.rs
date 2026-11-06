//! The phone as this PC's webcam and microphone.
//!
//! Camera: the phone sends `vyoma.webcam {w, h, rotate}` with a live payload of H.264 (Annex-B,
//! SPS/PPS before every keyframe). GStreamer decodes it on the GPU (VA-API), turns it upright
//! (`rotate`: clockwise degrees, from how the phone is held) and fits it into w x h with black
//! bars, as raw YUYV frames that this writes into a v4l2loopback device labelled [LABEL]. Every
//! app sees that as a normal webcam. The device is opened once per session with a fixed format, so
//! the phone turning (`vyoma.webcam.turn {rotate}`) only restarts the decoder and the app watching
//! never notices.
//!
//! Microphone: `vyoma.webcam.mic {rate, channels}` with a live payload of s16le PCM goes into a
//! PipeWire source "Vyoma Phone Mic" (pipewire-pulse's module-pipe-source, fed through a FIFO),
//! removed again when the phone stops.
//!
//! Both answer with `vyoma.webcam.state {camera|mic: true}` or `{error}`. v4l2loopback ships with the
//! kernel but needs loading once (desktop/companion/webcam/: sudo, once).

use std::io::Write;
use std::net::IpAddr;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::process::Stdio;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::watch;
use tracing::{info, warn};

use crate::identity::Identity;
use crate::link::{self, Sender};
use crate::payload;

pub const LABEL: &str = "Vyoma Phone Camera";
const MIC_NAME: &str = "vyoma_phone_mic";
const MIC_LABEL: &str = "Vyoma Phone Mic";
/// Both streams flow all the time (audio every 10 ms, a frame at least every 100 ms), so this long
/// without a byte means the phone is gone even if no close reached us (killed app, dropped cable).
/// The mic pipe's size: one page, about 40 ms of 48 kHz mono.
const MIC_PIPE_BYTES: libc::c_int = 4096;
const SILENCE: std::time::Duration = std::time::Duration::from_secs(4);
const OPEN_TRIES: u32 = 10;
const OPEN_WAIT: std::time::Duration = std::time::Duration::from_millis(200);

/// What the user is told when the loopback device is missing.
const SETUP: &str = "The PC needs a one-time setup for the phone camera (v4l2loopback): see desktop/companion/webcam/README";

// VIDIOC_S_FMT = _IOWR('V', 5, struct v4l2_format), 208 bytes on 64-bit.
const VIDIOC_S_FMT: libc::c_ulong = 0xc0d0_5605;
const BUF_TYPE_VIDEO_OUTPUT: u32 = 2;
const PIX_FMT_YUYV: u32 = u32::from_le_bytes(*b"YUYV");
const FIELD_NONE: u32 = 1;
const COLORSPACE_SRGB: u32 = 8;

#[repr(C)]
struct PixFormat {
    width: u32,
    height: u32,
    pixelformat: u32,
    field: u32,
    bytesperline: u32,
    sizeimage: u32,
    colorspace: u32,
    priv_: u32,
    flags: u32,
    ycbcr_enc: u32,
    quantization: u32,
    xfer_func: u32,
}

#[repr(C)]
struct Format {
    kind: u32,
    _pad: u32,
    pix: PixFormat,
    _rest: [u8; 200 - size_of::<PixFormat>()],
}

const _: () = assert!(size_of::<Format>() == 208);

/// The loopback device labelled [LABEL], if it's loaded.
pub fn device() -> Option<PathBuf> {
    std::fs::read_dir("/sys/class/video4linux").ok()?.flatten().find_map(|e| {
        let name = std::fs::read_to_string(e.path().join("name")).ok()?;
        (name.trim() == LABEL).then(|| PathBuf::from("/dev").join(e.file_name()))
    })
}

/// Opens the loopback for writing frames of w x h YUYV.
fn open_output(path: &PathBuf, w: u32, h: u32) -> Result<std::fs::File> {
    let file = std::fs::OpenOptions::new().read(true).write(true).open(path).with_context(|| format!("open {}", path.display()))?;
    let mut f = Format {
        kind: BUF_TYPE_VIDEO_OUTPUT,
        _pad: 0,
        pix: PixFormat {
            width: w,
            height: h,
            pixelformat: PIX_FMT_YUYV,
            field: FIELD_NONE,
            bytesperline: w * 2,
            sizeimage: w * h * 2,
            colorspace: COLORSPACE_SRGB,
            priv_: 0,
            flags: 0,
            ycbcr_enc: 0,
            quantization: 0,
            xfer_func: 0,
        },
        _rest: [0; 200 - size_of::<PixFormat>()],
    };
    // SAFETY: VIDIOC_S_FMT with a correctly sized, initialised v4l2_format.
    if unsafe { libc::ioctl(file.as_raw_fd(), VIDIOC_S_FMT, &mut f) } < 0 {
        bail!("set the camera format: {}", std::io::Error::last_os_error());
    }
    Ok(file)
}

fn direction(rotate: u32) -> &'static str {
    match rotate % 360 {
        90 => "90r",
        180 => "180",
        270 => "90l",
        _ => "identity",
    }
}

/// One decoder process: H.264 on stdin, upright w x h YUYV frames on stdout.
fn decoder(w: u32, h: u32, rotate: u32) -> Result<tokio::process::Child> {
    let pipeline = format!(
        "fdsrc fd=0 blocksize=65536 ! h264parse ! vah264dec ! \
         vapostproc video-direction={} add-borders=true ! video/x-raw,format=YUY2,width={w},height={h},pixel-aspect-ratio=1/1 ! \
         fdsink fd=1 sync=false",
        direction(rotate)
    );
    Command::new("gst-launch-1.0")
        .arg("-q")
        .args(pipeline.split_whitespace())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .context("start GStreamer")
}

async fn state(sender: &Sender, body: Value) {
    let _ = sender.send(&link::packet("vyoma.webcam.state", body)).await;
}

/// Runs the camera until the phone hangs up. [turn] carries the phone's latest `rotate`.
pub async fn camera(sender: &Sender, me: &Identity, pinned: &[u8], phone: IpAddr, packet: &Value, mut turn: watch::Receiver<u32>, name: &str) -> Result<()> {
    let body = &packet["body"];
    let w = body["w"].as_u64().unwrap_or(1280).clamp(320, 1920) as u32 & !1;
    let h = body["h"].as_u64().unwrap_or(720).clamp(240, 1080) as u32 & !1;
    let Some(path) = device() else {
        state(sender, json!({ "error": SETUP })).await;
        bail!("no {LABEL} device (v4l2loopback not loaded)");
    };
    // A stream replacing the last one: the old one lets go of the device a moment later.
    let mut tries = 0;
    let out = loop {
        match open_output(&path, w, h) {
            Ok(f) => break f,
            Err(_) if tries < OPEN_TRIES => {
                tries += 1;
                tokio::time::sleep(OPEN_WAIT).await;
            }
            Err(e) => {
                state(sender, json!({ "error": format!("{e:#}") })).await;
                return Err(e);
            }
        }
    };
    let mut input = payload::fetch_stream(me, pinned, phone, packet).await?;
    state(sender, json!({ "camera": true, "device": LABEL })).await;
    info!("{name}: phone camera live as {} ({w}x{h})", path.display());

    // Frames go to the device on a thread of its own (a plain blocking write); at most two wait, and
    // a newer one is dropped rather than queued when the reader is slow.
    let (frames, rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(2);
    let writer = std::thread::spawn(move || {
        let mut out = out;
        for f in rx {
            if out.write_all(&f).is_err() {
                break;
            }
        }
    });

    let frame_len = (w * h * 2) as usize;
    let mut buf = vec![0u8; 64 * 1024];
    let mut rotate = *turn.borrow_and_update();
    let result = 'session: loop {
        let mut dec = decoder(w, h, rotate)?;
        let mut stdin = dec.stdin.take().context("decoder stdin")?;
        let mut stdout = dec.stdout.take().context("decoder stdout")?;
        let frames = frames.clone();
        let reader = tokio::spawn(async move {
            let mut f = vec![0u8; frame_len];
            while stdout.read_exact(&mut f).await.is_ok() {
                // Full: the app reading the camera is behind; skip this frame.
                let _ = frames.try_send(f.clone());
            }
        });
        let ended = loop {
            tokio::select! {
                n = tokio::time::timeout(SILENCE, input.read(&mut buf)) => match n.unwrap_or_else(|_| Err(std::io::ErrorKind::TimedOut.into())) {
                    Ok(0) => break Ok(true),
                    Ok(n) => {
                        if let Err(e) = stdin.write_all(&buf[..n]).await {
                            break Err(anyhow::Error::from(e).context("the decoder stopped"));
                        }
                    }
                    Err(e) => break Err(anyhow::Error::from(e).context("the phone's stream")),
                },
                changed = turn.changed() => {
                    if changed.is_err() { break Ok(true) }
                    rotate = *turn.borrow_and_update();
                    break Ok(false);
                }
            }
        };
        drop(stdin);
        let _ = dec.kill().await;
        reader.abort();
        match ended {
            Ok(true) => break 'session Ok(()),
            Ok(false) => info!("{name}: phone turned, camera now rotated {rotate}°"),
            Err(e) => break 'session Err(e),
        }
    };
    drop(frames);
    let _ = writer.join();
    info!("{name}: phone camera stopped");
    result
}

/// Runs the microphone until the phone hangs up.
pub async fn mic(sender: &Sender, me: &Identity, pinned: &[u8], phone: IpAddr, packet: &Value, name: &str) -> Result<()> {
    let body = &packet["body"];
    let rate = body["rate"].as_u64().unwrap_or(48000).clamp(8000, 96000);
    let channels = body["channels"].as_u64().unwrap_or(1).clamp(1, 2);
    // Only the private runtime folder: anyone who can reach the pipe can speak into this microphone.
    let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) else {
        state(sender, json!({ "error": "couldn't add the microphone: the PC's session has no private folder" })).await;
        bail!("XDG_RUNTIME_DIR isn't set");
    };
    // A name of its own: a session being replaced removes its pipe as it goes.
    let fifo = dir.join(format!("vyoma-phone-mic-{}.fifo", uuid::Uuid::new_v4().simple()));
    remove_stale_mics().await;
    let loaded = Command::new("pactl")
        .args([
            "load-module".to_string(),
            "module-pipe-source".into(),
            format!("source_name={MIC_NAME}"),
            format!("file={}", fifo.display()),
            "format=s16le".into(),
            format!("rate={rate}"),
            format!("channels={channels}"),
            format!("source_properties='device.description=\"{MIC_LABEL}\" device.icon_name=audio-input-microphone'"),
        ])
        .output()
        .await
        .context("run pactl")?;
    if !loaded.status.success() {
        let e = String::from_utf8_lossy(&loaded.stderr).trim().to_string();
        state(sender, json!({ "error": format!("couldn't add the microphone: {e}") })).await;
        bail!("pactl load-module: {e}");
    }
    // Removed however this ends, also when the link drops and the task is aborted.
    let _module = MicModule(String::from_utf8_lossy(&loaded.stdout).trim().to_string(), fifo.clone());
    let result = async {
        let mut input = payload::fetch_stream(me, pinned, phone, packet).await?;
        // Non-blocking, with the smallest pipe: audio PipeWire hasn't taken yet is dropped rather
        // than queued. Queued, the pipe's 64 KiB plus the socket behind it held 1.5 s of audio
        // (measured), and the voice lagged the picture by that much.
        use std::os::unix::fs::OpenOptionsExt;
        let mut out = std::fs::OpenOptions::new().write(true).custom_flags(libc::O_NONBLOCK).open(&fifo).context("open the microphone pipe")?;
        // SAFETY: fcntl on a pipe we own.
        unsafe { libc::fcntl(out.as_raw_fd(), libc::F_SETPIPE_SZ, MIC_PIPE_BYTES) };
        state(sender, json!({ "mic": true, "device": MIC_LABEL })).await;
        info!("{name}: phone microphone live");
        let mut buf = vec![0u8; 16 * 1024];
        let frame = 2 * channels as usize;
        // Reads end anywhere; a sample cut in two waits here for the rest.
        let mut carry = 0usize;
        loop {
            let n = tokio::time::timeout(SILENCE, input.read(&mut buf[carry..])).await.context("the phone went quiet")??;
            if n == 0 {
                break;
            }
            let have = carry + n;
            let whole = have - have % frame;
            // 10 ms pieces: a pipe takes a write this small whole or not at all, so a dropped piece
            // never splits a sample.
            for piece in buf[..whole].chunks(rate as usize / 100 * frame) {
                match out.write(piece) {
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(e) => return Err(e.into()),
                }
            }
            buf.copy_within(whole..have, 0);
            carry = have - whole;
        }
        Ok::<_, anyhow::Error>(())
    }
    .await;
    info!("{name}: phone microphone stopped");
    if let Err(e) = &result {
        warn!("{name}: phone microphone: {e:#}");
    }
    result
}

/// Sources left by a companion that was stopped mid-stream (a kill skips the cleanup), so there's
/// only ever one "Vyoma Phone Mic". Called before adding one, and at start-up.
pub async fn remove_stale_mics() {
    let Ok(out) = Command::new("pactl").args(["list", "modules", "short"]).output().await else { return };
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if line.contains("module-pipe-source") && line.contains(&format!("source_name={MIC_NAME}"))
            && let Some(id) = line.split_whitespace().next()
        {
            let _ = Command::new("pactl").args(["unload-module", id]).status().await;
        }
    }
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR")
        && let Ok(entries) = std::fs::read_dir(dir)
    {
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().starts_with("vyoma-phone-mic") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

struct MicModule(String, PathBuf);

impl Drop for MicModule {
    fn drop(&mut self) {
        let _ = std::process::Command::new("pactl").args(["unload-module", &self.0]).status();
        let _ = std::fs::remove_file(&self.1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotations_name_gstreamer_directions() {
        assert_eq!(direction(0), "identity");
        assert_eq!(direction(90), "90r");
        assert_eq!(direction(180), "180");
        assert_eq!(direction(270), "90l");
        assert_eq!(direction(450), "90r");
    }
}
