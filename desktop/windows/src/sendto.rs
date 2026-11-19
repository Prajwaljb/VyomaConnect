//! Explorer's Send to menu: "Phone (Vyoma)" sends the right-clicked files and folders to the one
//! connected phone, or asks which in a toast when there are several (`--send-any`, the window's
//! `SendAny`). It's a shortcut in this user's SendTo folder, written by `--install` (which the
//! window runs on first open, and again when Vyoma moved) and removed by `--uninstall`. Never
//! written while testing (`VYOMA_DIR` set).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, IPersistFile};
use windows::Win32::UI::Shell::{FOLDERID_SendTo, IShellLinkW, KF_FLAG_DEFAULT, SHGetKnownFolderPath, ShellLink};
use windows::core::{HSTRING, Interface};

/// The menu's words (the shortcut's name).
const ENTRY: &str = "Phone (Vyoma).lnk";

fn folder() -> Result<PathBuf> {
    // SAFETY: the path Windows allocates is copied and freed.
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_SendTo, KF_FLAG_DEFAULT, None).context("no Send to folder")?;
        let path = p.to_string();
        CoTaskMemFree(Some(p.0.cast()));
        Ok(PathBuf::from(path?))
    }
}

fn testing() -> bool {
    std::env::var_os("VYOMA_DIR").is_some()
}

/// A shortcut at [link] that runs [exe] `--send-any` with the chosen files after it.
fn write_link(link: &Path, exe: &Path) -> Result<()> {
    // SAFETY: this thread joins an apartment for the shell's link object; everything is owned.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let shortcut: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        let exe = HSTRING::from(exe.as_os_str());
        shortcut.SetPath(&exe)?;
        shortcut.SetArguments(&HSTRING::from("--send-any"))?;
        shortcut.SetIconLocation(&exe, 0)?;
        shortcut.SetDescription(&HSTRING::from("Send to your phone with Vyoma"))?;
        shortcut.cast::<IPersistFile>()?.Save(&HSTRING::from(link.as_os_str()), true)?;
    }
    Ok(())
}

/// Puts "Phone (Vyoma)" in Send to, pointing at this copy of the agent.
pub fn add() -> Result<()> {
    if testing() {
        return Ok(());
    }
    write_link(&folder()?.join(ENTRY), &std::env::current_exe()?)
}

pub fn remove() {
    if testing() {
        return;
    }
    if let Ok(dir) = folder() {
        let _ = std::fs::remove_file(dir.join(ENTRY));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_shortcut_to_this_program() {
        let dir = std::env::temp_dir().join(format!("vyoma-sendto-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let link = dir.join(ENTRY);
        write_link(&link, &std::env::current_exe().unwrap()).unwrap();
        let bytes = std::fs::read(&link).unwrap();
        // A shell link starts with its header size (0x4C) and the link class.
        assert_eq!(&bytes[..4], &[0x4c, 0, 0, 0]);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(folder().is_ok());
    }
}
