//! Telling when a newer Vyoma for Windows is out: core's [Updates] for releases that carry the
//! installer (`Vyoma-setup-<version>.exe`), told in a notification; the window's Update opens the
//! release's page, and the installer replaces this one in place (the phones stay paired).

use std::path::Path;
use std::sync::Arc;

pub use vyoma_core::updates::Updates;

pub fn carries_installer(asset: &str) -> bool {
    asset.starts_with("Vyoma-setup-") && asset.ends_with(".exe")
}

pub fn load(dir: &Path, current: &str) -> Arc<Updates> {
    Updates::load(dir, current, carries_installer)
}

/// Checks once a day while checks are on; [changed] runs when something new is known.
pub async fn run(updates: Arc<Updates>, changed: impl Fn()) {
    let tell = |version: String| crate::tray::notify(&format!("Vyoma {version} is out"), "Open Vyoma and choose Download. Your phones stay paired.");
    updates.run(tell, changed).await;
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_the_installer_counts() {
        assert!(super::carries_installer("Vyoma-setup-0.12.0.exe"));
        assert!(!super::carries_installer("Vyoma-setup-0.12.0.exe.sha256"));
        assert!(!super::carries_installer("Vyoma-mac-0.12.0.zip"));
    }
}
