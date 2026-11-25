//! Vyoma's camera for Windows (`vyoma_camera.dll`): the phone's camera as a webcam every app sees,
//! "Vyoma Phone Camera". Windows 11's camera service (Frame Server) loads this DLL and asks it for
//! a media source ([source]); the agent adds the camera for its session with MFCreateVirtualCamera
//! and writes the phone's pictures into a shared section ([frame]) that the source reads. While no
//! phone sends, the camera shows Vyoma's orb on the night sky.
//!
//! The camera service runs in session 0 as LOCAL SERVICE: it can only load this DLL from a folder
//! every account can read, registered machine-wide. So `vyoma-windows --install-camera` (run as an
//! administrator, from This PC's Set up or the installer) copies it under Program Files and
//! registers its class in HKLM; nothing else here needs an administrator.
//! Built and tested on CI and under Wine (the source, read through Media Foundation's source
//! reader); the camera service and MFCreateVirtualCamera aren't in Wine: not yet run on Windows 11.

pub mod frame;
#[cfg(windows)]
mod source;

/// The class Windows' camera service creates (an IMFActivate for the source).
pub const CLSID: u128 = 0x5f0c9a1e_3b7d_4e62_a8f1_2d6c4b9e7a13;
/// What apps call the camera.
pub const NAME: &str = "Vyoma Phone Camera";
/// The shared section's name: global, since the camera service and the agent are in different sessions.
pub const SECTION: &str = "Global\\VyomaPhoneCamera";

/// The section's name; tests use a session-local one (`VYOMA_CAMERA_SECTION`).
pub fn section_name() -> String {
    std::env::var("VYOMA_CAMERA_SECTION").unwrap_or_else(|_| SECTION.to_owned())
}

/// The CLSID as the registry and MFCreateVirtualCamera write it: "{5F0C9A1E-...}".
pub fn clsid_text() -> String {
    let h = format!("{CLSID:032X}");
    format!("{{{}-{}-{}-{}-{}}}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}

#[cfg(windows)]
pub use source::{DllCanUnloadNow, DllGetClassObject, Section, media_source, now_ms};

#[cfg(test)]
mod tests {
    #[test]
    fn the_class_reads_as_windows_writes_it() {
        assert_eq!(super::clsid_text(), "{5F0C9A1E-3B7D-4E62-A8F1-2D6C4B9E7A13}");
    }
}
