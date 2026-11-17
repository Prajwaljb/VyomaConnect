//! Plug the phone in, and Vyoma's USB link just works (the adb tunnels and grants are
//! `vyoma_core::adb`, shared with the Mac). This side notices phones with USB debugging in sysfs
//! and asks about the clipboard grants with a notification.

use std::collections::HashSet;
use std::time::Duration;

pub use vyoma_core::adb::{ClipSync, allow_clipboard, cable_ports, clipboard, forward_unix, unforward_unix};

const POLL: Duration = Duration::from_secs(3);

pub async fn watch() {
    vyoma_core::adb::set_port(crate::desk_port());
    vyoma_core::adb::set_asker(Box::new(|q| Box::pin(ask(q))));
    let (tx, rx) = tokio::sync::watch::channel(HashSet::new());
    tokio::spawn(vyoma_core::adb::watch(rx));
    loop {
        let now = adb_interfaces();
        tx.send_if_modified(|known| {
            let changed = *known != now;
            *known = now;
            changed
        });
        tokio::time::sleep(POLL).await;
    }
}

/// USB devices exposing an adb interface (class ff, subclass 42, protocol 01). Just sysfs reads.
fn adb_interfaces() -> HashSet<String> {
    let Ok(entries) = std::fs::read_dir("/sys/bus/usb/devices") else { return HashSet::new() };
    entries
        .flatten()
        .filter(|e| {
            let p = e.path();
            let read = |f: &str| std::fs::read_to_string(p.join(f)).map(|s| s.trim().to_string()).unwrap_or_default();
            read("bInterfaceClass") == "ff" && read("bInterfaceSubClass") == "42" && read("bInterfaceProtocol") == "01"
        })
        .map(|e| e.file_name().to_string_lossy().split(':').next().unwrap_or_default().to_string())
        .collect()
}

/// A notification with Allow and Not now; notify-send waits for the click.
async fn ask(q: vyoma_core::adb::Question) -> Option<bool> {
    let out = tokio::process::Command::new("notify-send")
        .args(["--app-name=Vyoma", "--hint=string:desktop-entry:vyoma", "--icon=dialog-password", "--action=allow=Allow", "--action=no=Not now"])
        .arg(&q.title)
        .arg(&q.body)
        .kill_on_drop(true)
        .output()
        .await
        .ok()?;
    match String::from_utf8_lossy(&out.stdout).trim() {
        "allow" => Some(true),
        "no" => Some(false),
        _ => None,
    }
}
