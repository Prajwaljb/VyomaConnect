//! PC screenshot to phone. Spectacle (always on Plasma) captures in the background with no
//! prompt or notification of its own, so we show one; the image travels as a payload (see payload.rs).

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
    let image = match capture().await {
        Ok(bytes) => bytes,
        Err(e) => {
            sender.send(&link::packet("vyoma.screenshot", json!({ "error": format!("{e:#}") }))).await?;
            return Err(e);
        }
    };
    crate::notice::tell("camera-photo", "Screenshot taken from your phone", &format!("{phone_name} took a screenshot of this PC.")).await;
    let announce: Value = link::packet("vyoma.screenshot", json!({ "format": "webp" }));
    payload::offer(sender, me, pinned, announce, image.len() as u64, image.as_slice()).await
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
