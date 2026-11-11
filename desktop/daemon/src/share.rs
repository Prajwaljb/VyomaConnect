//! Sharing (the protocol is vyoma-core's `share`): where files land and how they're opened, the
//! freedesktop way: XDG Downloads, xdg-open, and the file manager's ShowItems.

use std::path::{Path, PathBuf};

pub use vyoma_core::share::*;

/// XDG Downloads.
pub fn downloads() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
    if let Ok(text) = std::fs::read_to_string(home.join(".config/user-dirs.dirs"))
        && let Some(v) = text.lines().find_map(|l| l.strip_prefix("XDG_DOWNLOAD_DIR="))
    {
        let v = v.trim().trim_matches('"').replace("$HOME", &home.to_string_lossy());
        if v.starts_with('/') {
            return PathBuf::from(v);
        }
    }
    home.join("Downloads")
}

/// Opens [target] (a file, folder or web link) in the user's app for it.
pub fn open(target: &str) {
    match std::process::Command::new("xdg-open").arg(target).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn() {
        // Reaped on its own thread: xdg-open waits on nothing, it hands over.
        Ok(mut child) => drop(std::thread::spawn(move || child.wait())),
        Err(e) => tracing::warn!("can't open it: no xdg-open ({e})"),
    }
}

/// Shows [path] selected in the file manager (Dolphin, Nautilus, Nemo and others take this call),
/// else opens its folder.
pub async fn show_in_folder(conn: &zbus::Connection, path: &Path) {
    let uri = format!("file://{}", percent(&path.to_string_lossy()));
    let shown = conn
        .call_method(Some("org.freedesktop.FileManager1"), "/org/freedesktop/FileManager1", Some("org.freedesktop.FileManager1"), "ShowItems", &(vec![uri], ""))
        .await;
    if shown.is_err()
        && let Some(dir) = path.parent()
    {
        open(&dir.to_string_lossy());
    }
}
