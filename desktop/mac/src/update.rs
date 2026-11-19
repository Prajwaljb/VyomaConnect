//! Telling when a newer Vyoma for the Mac is out: core's [Updates] for releases that carry a Mac
//! app (`Vyoma-mac…zip`), told in a notification. It doesn't replace itself yet: the app isn't
//! signed by Apple, so the person downloads it as they did the first time, and the phones stay
//! paired.

use std::path::Path;
use std::sync::Arc;

pub use vyoma_core::updates::Updates;

fn carries_mac(asset: &str) -> bool {
    asset.starts_with("Vyoma-mac") && asset.ends_with(".zip")
}

pub fn load(dir: &Path, current: &str) -> Arc<Updates> {
    Updates::load(dir, current, carries_mac)
}

/// Checks once a day while checks are on; [changed] runs when something new is known.
pub async fn run(updates: Arc<Updates>, changed: impl Fn()) {
    let tell = |version: String| {
        tokio::spawn(async move { crate::mac::notify(&format!("Vyoma {version} is out"), "Open Vyoma and choose Download. Your phones stay paired.").await });
    };
    updates.run(tell, changed).await;
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_the_mac_app_counts() {
        assert!(super::carries_mac("Vyoma-mac-0.12.0.zip"));
        assert!(!super::carries_mac("Vyoma-mac-0.12.0.zip.sha256"));
        assert!(!super::carries_mac("Vyoma-setup-0.12.0.exe"));
    }
}
