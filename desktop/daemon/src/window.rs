//! Opens Vyoma's window (the `vyoma` binary, desktop/gui) on one of its pages, as a phone's
//! "Commands" button asks for the editor: the running window shows it, else a new one starts.

use std::path::PathBuf;

use anyhow::{Context, Result};

/// The window's own name on the session bus, and what it answers there.
pub const NAME: &str = "dev.vyoma.Window";
const PATH: &str = "/dev/vyoma/Window";
const INTERFACE: &str = "dev.vyoma.Window1";

pub async fn open(page: &'static str) {
    if let Err(e) = try_open(page).await {
        tracing::warn!("can't open the window on {page}: {e:#}");
    }
}

async fn try_open(page: &str) -> Result<()> {
    let conn = zbus::Connection::session().await?;
    if conn.call_method(Some(NAME), PATH, Some(INTERFACE), "Show", &(page,)).await.is_ok() {
        return Ok(());
    }
    let exe = binary().context("Vyoma's window isn't installed (no vyoma beside vyoma-desktop or on PATH)")?;
    // Its own transient unit, so restarting the daemon doesn't close it.
    let status = tokio::process::Command::new("systemd-run")
        .args(["--user", "--collect", "--quiet", "--"])
        .arg(&exe)
        .args(["--page", page])
        .status()
        .await?;
    anyhow::ensure!(status.success(), "systemd-run failed: {status}");
    Ok(())
}

/// `vyoma` beside this binary (both in ~/.local/bin once installed), else on PATH.
fn binary() -> Option<PathBuf> {
    let beside = std::env::current_exe().ok()?.with_file_name("vyoma");
    if beside.is_file() {
        return Some(beside);
    }
    std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join("vyoma")).find(|f| f.is_file()))
}
