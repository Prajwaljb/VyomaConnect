//! The companion's features as a library: the companion (beside kdeconnectd, `main.rs`) and the
//! standalone desktop app (`desktop/daemon`) both serve phones with [session::serve].

pub mod accessory;
pub mod away;
pub mod bluetooth;
pub mod camera;
pub mod desktop;
pub mod files;
pub mod gamepad;
pub mod kde;
pub mod media;
pub mod mirror;
pub mod notice;
pub mod paste;
pub mod pc;
pub mod permits;
pub mod pointer;
pub mod screen;
pub mod screenshot;
pub mod session;
pub mod setup;
pub mod specs;
pub mod stats;
pub mod tether;
pub mod usb;
pub mod wallpaper;
pub mod webcam;
pub mod xfer;

// The shared protocol core; `crate::link` and friends keep working for the modules here.
pub use vyoma_core::{identity, link, payload};
pub use link::packet;

/// The port the PC's own KDE Connect side listens on, which the cable and Bluetooth tunnels lead to:
/// kdeconnectd's 1716, or wherever the desktop app's listener got.
static DESK_PORT: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(1716);

pub fn desk_port() -> u16 {
    DESK_PORT.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_desk_port(port: u16) {
    DESK_PORT.store(port, std::sync::atomic::Ordering::Relaxed);
}
