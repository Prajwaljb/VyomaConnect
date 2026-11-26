//! A newer Vyoma for Windows: core's [Updates] for releases that carry the installer
//! (`Vyoma-setup-<version>.exe`), told in a notification. The window's Update fetches that
//! installer and its `.sha256` from the release, checks it, and runs it quietly for this user (it
//! needs no administrator): it closes Vyoma, replaces it in place and opens it again (`/RESTART`).
//! The phones stay paired. The phone camera, if it was set up, asks for an administrator to take
//! the new version (an update without that keeps the camera it had).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};

pub use vyoma_core::updates::Updates;

pub fn carries_installer(asset: &str) -> bool {
    asset.starts_with("Vyoma-setup-") && asset.ends_with(".exe")
}

pub fn load(dir: &Path, current: &str) -> Arc<Updates> {
    Updates::load(dir, current, carries_installer)
}

/// The installer and its sum can take a while on a slow line.
const FETCH_WAIT: Duration = Duration::from_secs(10 * 60);

/// Checks once a day while checks are on; [changed] runs when something new is known.
pub async fn run(updates: Arc<Updates>, changed: impl Fn()) {
    updates.installs_itself();
    let tell = |version: String| crate::tray::notify(&format!("Vyoma {version} is out"), "Open Vyoma and choose Update. Your phones stay paired.");
    updates.run(tell, changed).await;
}

/// Where release [version]'s file [name] is.
fn release_file(version: &str, name: &str) -> String {
    format!("https://github.com/{}/releases/download/v{version}/{name}", vyoma_core::release::REPO)
}

/// The window's Update: the newer release's installer, fetched, checked and started; this agent is
/// then closed by it. [changed] tells the window when it starts and if it fails.
pub async fn install(updates: Arc<Updates>, changed: impl Fn()) -> Result<()> {
    let version = updates.newer().context("No newer Vyoma for Windows yet")?;
    if !updates.set_updating(true) {
        bail!("Vyoma is updating already");
    }
    changed();
    let done = fetch_and_start(&version).await;
    if done.is_err() {
        updates.set_updating(false);
        changed();
    }
    done
}

async fn fetch_and_start(version: &str) -> Result<()> {
    let name = format!("Vyoma-setup-{version}.exe");
    let dir = crate::win::data_dir().join("update");
    // Only this release's installer is kept there.
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let setup = dir.join(&name);
    let sum = dir.join(format!("{name}.sha256"));
    fetch(&release_file(version, &name), &setup).await?;
    fetch(&release_file(version, &format!("{name}.sha256")), &sum).await?;
    let expected = expected_sum(&std::fs::read_to_string(&sum)?).context("The release's checksum couldn't be read")?;
    let bytes = std::fs::read(&setup)?;
    if !crate::gamepad::sum_matches(&bytes, &expected) {
        let _ = std::fs::remove_file(&setup);
        bail!("The download didn't match the release's checksum; nothing was installed");
    }
    tracing::info!("updating to {version}");
    // Its own process: it outlives this agent, which it closes before replacing it.
    std::process::Command::new(&setup).args(["/S", "/RESTART"]).spawn().context("Windows wouldn't start the installer")?;
    Ok(())
}

async fn fetch(url: &str, to: &PathBuf) -> Result<()> {
    let got = tokio::time::timeout(FETCH_WAIT, crate::usb::quiet("curl.exe").args(["-fsSL", "--proto", "=https", "-o"]).arg(to).arg(url).status())
        .await
        .context("The download took too long")?
        .context("Windows has no curl")?;
    if !got.success() {
        bail!("Couldn't download the update (check the internet connection)");
    }
    Ok(())
}

/// The hex sum in a `sha256sum` line ("<hex>  Vyoma-setup-0.12.0.exe").
fn expected_sum(text: &str) -> Option<String> {
    let hex = text.split_whitespace().next()?.to_lowercase();
    (hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit())).then_some(hex)
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_the_installer_counts() {
        assert!(super::carries_installer("Vyoma-setup-0.12.0.exe"));
        assert!(!super::carries_installer("Vyoma-setup-0.12.0.exe.sha256"));
        assert!(!super::carries_installer("Vyoma-mac-0.12.0.zip"));
    }

    #[test]
    fn finds_the_installer_and_its_sum() {
        assert_eq!(
            super::release_file("0.12.0", "Vyoma-setup-0.12.0.exe"),
            format!("https://github.com/{}/releases/download/v0.12.0/Vyoma-setup-0.12.0.exe", vyoma_core::release::REPO)
        );
        let hex = "ab".repeat(32);
        assert_eq!(super::expected_sum(&format!("{}  Vyoma-setup-0.12.0.exe\n", hex.to_uppercase())), Some(hex));
        assert_eq!(super::expected_sum("not a sum"), None);
        assert_eq!(super::expected_sum(""), None);
    }
}
