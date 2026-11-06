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
    if !available() {
        bail!(unavailable_reason(behind_lid()));
    }
    let (device, format, size) = find_camera().await.context("no camera found")?;
    let out = tokio::time::timeout(
        Duration::from_secs(15),
        tokio::process::Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-f", "v4l2", "-input_format", format, "-video_size", &size, "-i", &device])
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

fn unavailable_reason(behind_lid: bool) -> &'static str {
    if behind_lid { "the laptop's lid is closed, so its camera can't see" } else { "this PC has no camera" }
}

/// When there's no camera to use, whether that's only the lid's doing. A desktop without a webcam has
/// no lid, and a closed laptop may have no camera of its own either.
pub fn behind_lid() -> bool {
    lid_closed() && camera_nodes().iter().any(|node| built_in(node))
}

/// Whether a photo can be taken now, cheaply (sysfs only, every stats tick): some camera is plugged
/// in, and it isn't only the laptop's built-in one behind a closed lid.
pub fn available() -> bool {
    let lid_closed = lid_closed();
    camera_nodes().iter().any(|node| !lid_closed || !built_in(node))
}

/// The laptop's lid is shut (ACPI). Desktops have no lid: never closed.
fn lid_closed() -> bool {
    let Ok(dir) = std::fs::read_dir("/proc/acpi/button/lid") else { return false };
    dir.flatten().any(|e| std::fs::read_to_string(e.path().join("state")).is_ok_and(|s| s.contains("closed")))
}

/// A camera wired into the machine (the lid's webcam) rather than plugged into a port: its USB
/// device says `removable=fixed`, or its port `connect_type=hardwired`.
fn built_in(node: &str) -> bool {
    let Ok(dev) = std::fs::canonicalize(format!("/sys/class/video4linux/{node}/device/..")) else { return false };
    std::fs::read_to_string(dev.join("removable")).is_ok_and(|r| r.trim() == "fixed")
        || std::fs::read_to_string(dev.join("port/connect_type")).is_ok_and(|t| t.trim() == "hardwired")
}

/// The capture node of each real camera (index 0; the others are metadata). A virtual one has no
/// device behind it: the phone-as-webcam loopback isn't something to take a photo with.
fn camera_nodes() -> Vec<String> {
    let mut nodes: Vec<String> = std::fs::read_dir("/sys/class/video4linux")
        .map(|d| {
            d.flatten()
                .filter(|e| std::fs::read_to_string(e.path().join("index")).is_ok_and(|i| i.trim() == "0"))
                .filter(|e| e.path().join("device").exists())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    nodes.sort();
    nodes
}

/// What to ask a camera for, from ffmpeg's `-list_formats all`: MJPEG at its largest if it offers it,
/// else raw colour, which is slow at big sizes, so up to 1080p. IR cameras are greyscale only: None.
fn best_format(listing: &str) -> Option<(&'static str, String)> {
    let sizes = |line: &str| -> Vec<(u32, u32)> {
        line.split_whitespace().filter_map(|s| s.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))).collect()
    };
    let largest = |sizes: Vec<(u32, u32)>| sizes.into_iter().max_by_key(|(w, h)| w * h).map(|(w, h)| format!("{w}x{h}"));
    let offering = |format: &str| listing.lines().find(|l| l.split_whitespace().any(|word| word == format));
    if let Some(size) = offering("mjpeg").and_then(|l| largest(sizes(l))) {
        return Some(("mjpeg", size));
    }
    ["yuyv422", "nv12"].into_iter().find_map(|format| {
        let fits = sizes(offering(format)?).into_iter().filter(|(w, h)| w * h <= 1920 * 1080).collect();
        Some((format, largest(fits)?))
    })
}

/// The first colour camera that can see, what to ask it for and at what size. Querying formats doesn't
/// switch the camera on.
async fn find_camera() -> Option<(String, &'static str, String)> {
    // Behind a closed lid the built-in camera sees only the keyboard; a plugged-in one still works.
    let lid_closed = lid_closed();
    for node in camera_nodes().into_iter().filter(|n| !lid_closed || !built_in(n)) {
        let device = format!("/dev/{node}");
        let Ok(out) = tokio::process::Command::new("ffmpeg")
            .args(["-hide_banner", "-f", "v4l2", "-list_formats", "all", "-i", &device])
            .output()
            .await
        else {
            continue;
        };
        if let Some((format, size)) = best_format(&String::from_utf8_lossy(&out.stderr)) {
            return Some((device, format, size));
        }
    }
    None
}

async fn notify(phone_name: &str) {
    crate::notice::tell("camera-photo", "Camera used from your phone", &format!("{phone_name} took a photo with this PC's camera.")).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAW_ONLY: &str = "[in#0 @ 0x0] Raw       :     yuyv422 :           YUYV 4:2:2 : 640x480 3840x2160 1280x720 1920x1080";

    #[test]
    fn mjpeg_wins_at_its_largest_size() {
        let listing = format!("[in#0 @ 0x0] Compressed:       mjpeg :          Motion-JPEG : 1280x720 320x180 960x540\n{RAW_ONLY}");
        assert_eq!(best_format(&listing), Some(("mjpeg", "1280x720".to_string())));
    }

    #[test]
    fn a_raw_only_webcam_is_used_up_to_1080p() {
        assert_eq!(best_format(RAW_ONLY), Some(("yuyv422", "1920x1080".to_string())));
    }

    #[test]
    fn an_ir_camera_is_not_a_camera_for_photos() {
        assert_eq!(best_format("[in#0 @ 0x0] Raw       :        gray :      8-bit Greyscale : 640x360"), None);
    }

    #[test]
    fn only_a_camera_behind_a_lid_blames_the_lid() {
        assert!(unavailable_reason(true).contains("lid"));
        assert_eq!(unavailable_reason(false), "this PC has no camera");
    }
}
