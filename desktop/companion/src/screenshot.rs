//! PC screenshot to phone. Spectacle (always on Plasma) captures in the background with no
//! prompt or notification of its own, so we show one; the image travels as a payload (see payload.rs).
//! On GNOME, which lets only its own parts capture without asking, Vyoma's extension does (PNG).
//! X11 sessions (Cinnamon, MATE, Xfce) let any app read the screen: GStreamer grabs it (PNG).
//! Hyprland, Sway and the other wlroots-style compositors: grim (PNG).

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::identity::Identity;
use crate::link::{self, Sender};
use crate::payload;

/// Captures the whole desktop and offers it to the phone. On failure the phone gets an error
/// packet instead, so it never waits on a spinner.
pub async fn capture_and_send(sender: &Sender, me: &Identity, pinned: &[u8], phone_name: &str) -> Result<()> {
    let how = shooter();
    let png = !matches!(how, Shooter::Spectacle | Shooter::Nothing);
    let shot = match how {
        Shooter::X11 => capture_x11().await,
        Shooter::Gnome => crate::shell::screenshot().await,
        Shooter::Grim => capture_grim().await,
        // With neither, Spectacle's own error says what's missing.
        Shooter::Spectacle | Shooter::Nothing => capture().await,
    };
    let image = match shot {
        Ok(bytes) => bytes,
        Err(e) => {
            sender.send(&link::packet("vyoma.screenshot", json!({ "error": format!("{e:#}") }))).await?;
            return Err(e);
        }
    };
    crate::notice::tell("camera-photo", "Screenshot taken from your phone", &format!("{phone_name} took a screenshot of this PC.")).await;
    let announce: Value = link::packet("vyoma.screenshot", json!({ "format": if png { "png" } else { "webp" } }));
    payload::offer(sender, me, pinned, announce, image.len() as u64, image.as_slice()).await
}

/// How a screenshot is taken here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shooter {
    /// The X server's picture (GStreamer's ximagesrc).
    X11,
    /// Vyoma's GNOME extension: GNOME lets only its own parts capture silently.
    Gnome,
    Spectacle,
    /// wlroots-style compositors (Hyprland, Sway...) let grim copy the screen.
    Grim,
    Nothing,
}

/// The way this session takes screenshots, in the order tried.
pub fn shooter() -> Shooter {
    if crate::x11::active() {
        Shooter::X11
    } else if crate::desktop::is_gnome() {
        Shooter::Gnome
    } else if has("spectacle") {
        Shooter::Spectacle
    } else if has("grim") {
        Shooter::Grim
    } else {
        Shooter::Nothing
    }
}

/// Whether [program] is on PATH.
fn has(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(program).is_file()))
}

/// Every output, as PNG, through wlr's screencopy (grim writes it to its stdout).
async fn capture_grim() -> Result<Vec<u8>> {
    let out = tokio::time::timeout(Duration::from_secs(15), tokio::process::Command::new("grim").args(["-t", "png", "-"]).kill_on_drop(true).output())
        .await
        .context("the screenshot took too long")?
        .context("grim isn't installed")?;
    if !out.status.success() || !out.stdout.starts_with(b"\x89PNG") {
        bail!("grim couldn't capture the screen: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(out.stdout)
}

/// The whole X screen, every monitor, as PNG.
async fn capture_x11() -> Result<Vec<u8>> {
    let out = tokio::time::timeout(
        Duration::from_secs(15),
        tokio::process::Command::new("gst-launch-1.0")
            .args(["-q", "ximagesrc", "use-damage=false", "num-buffers=1", "!", "videoconvert", "!", "pngenc", "!", "fdsink", "fd=1"])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .context("the screenshot took too long")?
    .context("GStreamer (gst-launch-1.0) isn't installed")?;
    if !out.status.success() || !out.stdout.starts_with(b"\x89PNG") {
        bail!("couldn't capture the screen");
    }
    Ok(out.stdout)
}

async fn capture() -> Result<Vec<u8>> {
    // Private to this user (XDG_RUNTIME_DIR is 0700), and deleted as soon as it's read.
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    // WebP: about a quarter of the PNG, and still crisp on text at Spectacle's default quality.
    let path = dir.join(format!("vyoma-shot-{}.webp", std::process::id()));
    let status = tokio::time::timeout(
        Duration::from_secs(15),
        tokio::process::Command::new("spectacle")
            .args(["--background", "--nonotify", "--fullscreen", "--output"])
            .arg(&path)
            .kill_on_drop(true)
            .status(),
    )
    .await
    .context("Spectacle took too long")?
    .context("Spectacle isn't installed")?;
    let bytes = tokio::fs::read(&path).await;
    let _ = tokio::fs::remove_file(&path).await;
    if !status.success() {
        bail!("Spectacle couldn't capture the screen");
    }
    bytes.context("Spectacle saved no image")
}
