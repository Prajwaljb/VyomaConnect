//! Laptop camera photo to phone. ffmpeg grabs one frame from the colour webcam (V4L2, MJPEG) after
//! the auto-exposure has settled. Every capture also shows a desktop notification: someone at the
//! PC should always know its camera was used remotely (the camera light comes on as well).

use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::json;

use crate::identity::Identity;
use crate::link::{self, Sender};
use crate::payload;

/// Frames dropped before the one we keep: webcams start dark and need ~0.5 s to expose.
const WARMUP_FRAMES: u32 = 15;

pub async fn capture_and_send(sender: &Sender, me: &Identity, pinned: &[u8], phone_name: &str) -> Result<()> {
    let photo = match capture().await {
        Ok(bytes) => bytes,
        Err(e) => {
            sender.send(&link::packet("vyoma.camera", json!({ "error": format!("{e:#}") }))).await?;
            return Err(e);
        }
    };
    notify(phone_name).await;
    let announce = link::packet("vyoma.camera", json!({ "format": "jpg" }));
    payload::offer(sender, me, pinned, announce, photo.len() as u64, photo.as_slice()).await
}

async fn capture() -> Result<Vec<u8>> {
    let (device, size) = find_camera().await.context("no camera found")?;
    let out = tokio::time::timeout(
        Duration::from_secs(15),
        tokio::process::Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-f", "v4l2", "-input_format", "mjpeg", "-video_size", &size, "-i", &device])
            .args(["-vf", &format!("select=gte(n\\,{WARMUP_FRAMES})"), "-frames:v", "1", "-q:v", "2", "-f", "image2pipe", "-c:v", "mjpeg", "pipe:1"])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .context("the camera took too long")?
    .context("ffmpeg isn't installed")?;
    if !out.status.success() || out.stdout.is_empty() {
        let err = String::from_utf8_lossy(&out.stderr);
        if err.contains("busy") {
            bail!("the camera is in use by another app");
        }
        bail!("couldn't take a photo");
    }
    Ok(out.stdout)
}

/// The first colour camera (one that offers MJPEG; IR cameras are greyscale only) and its
/// largest MJPEG size. Querying formats doesn't switch the camera on.
async fn find_camera() -> Option<(String, String)> {
    let mut nodes: Vec<String> = std::fs::read_dir("/sys/class/video4linux")
        .ok()?
        .flatten()
        .filter(|e| std::fs::read_to_string(e.path().join("index")).is_ok_and(|i| i.trim() == "0"))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    nodes.sort();
    for node in nodes {
        let device = format!("/dev/{node}");
        let Ok(out) = tokio::process::Command::new("ffmpeg")
            .args(["-hide_banner", "-f", "v4l2", "-list_formats", "compressed", "-i", &device])
            .output()
            .await
        else {
            continue;
        };
        let listing = String::from_utf8_lossy(&out.stderr);
        let Some(line) = listing.lines().find(|l| l.contains("mjpeg")) else { continue };
        let largest = line
            .split_whitespace()
            .filter_map(|s| s.split_once('x').and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?))))
            .max_by_key(|(w, h)| w * h);
        if let Some((w, h)) = largest {
            return Some((device, format!("{w}x{h}")));
        }
    }
    None
}

async fn notify(phone_name: &str) {
    let _ = tokio::process::Command::new("notify-send")
        .args(["--app-name=Vyoma", "--icon=camera-photo", "Camera used from your phone"])
        .arg(format!("{phone_name} took a photo with this PC's camera."))
        .status()
        .await;
}
