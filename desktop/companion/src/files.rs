//! The phone's file traffic is `vyoma_core::files` and `vyoma_core::xfer`; this is what a Linux
//! desktop does around it: a notice when the phone reads files, "Received" with Open and Show in
//! folder, and sends landing where kdeconnectd puts what it receives.

use std::path::{Path, PathBuf};
use std::sync::Once;

pub use vyoma_core::files::*;

/// Gives the core this desktop's [Hooks]; once per process.
pub fn install() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| set_hooks(Box::new(Linux)));
}

struct Linux;

impl Hooks for Linux {
    fn reading(&self, phone: &str, name: &str) {
        let body = format!("{phone} is copying {name} from this PC.");
        tokio::spawn(async move { crate::notice::tell("document-send", "File read from your phone", &body).await });
    }

    fn received(&self, phone: &str, path: &Path, count: Option<u64>) {
        notify(phone, path, count);
    }

    fn incoming_dir(&self, phone_id: &str) -> PathBuf {
        incoming_dir(phone_id)
    }
}

/// Where kdeconnectd saves what this phone sends: its share plugin's `incoming_path` if set
/// (`%1` is the phone's name there; we use the folder as is), else XDG Downloads. The desktop app
/// saves its stock shares to Downloads, so these go there too, whatever kdeconnectd's old setting.
fn incoming_dir(phone_id: &str) -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
    if crate::setup::is_standalone() {
        return downloads(&home);
    }
    let config = home.join(".config/kdeconnect").join(phone_id).join("kdeconnect_share/config");
    if let Ok(text) = std::fs::read_to_string(config)
        && let Some(v) = text.lines().find_map(|l| l.strip_prefix("incoming_path="))
    {
        let v = v.trim().trim_start_matches("file://").replace("/%1", "");
        if v.starts_with('/') {
            return PathBuf::from(v);
        }
    }
    downloads(&home)
}

fn downloads(home: &Path) -> PathBuf {
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

/// "Received X from phone", like kdeconnectd's, with Open and Show in folder. Runs on its own:
/// notify-send waits for the click.
fn notify(phone_name: &str, path: &Path, count: Option<u64>) {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let body = match count {
        Some(n) => format!("{name} ({n} file{}) from {phone_name}", if n == 1 { "" } else { "s" }),
        None => format!("{name} from {phone_name}"),
    };
    let path = path.to_path_buf();
    tokio::spawn(async move {
        let out = tokio::process::Command::new("notify-send")
            .args(["--app-name=Vyoma", "--hint=string:desktop-entry:vyoma", "--icon=document-save", "--action=open=Open", "--action=folder=Show in folder", "--", "Received"])
            .arg(crate::notice::escape(&body))
            .output()
            .await;
        let Ok(out) = out else { return };
        let target = match String::from_utf8_lossy(&out.stdout).trim() {
            "open" => path,
            "folder" => path.parent().map(Path::to_path_buf).unwrap_or(path),
            _ => return,
        };
        let _ = tokio::process::Command::new("xdg-open").arg(target).spawn();
    });
}
