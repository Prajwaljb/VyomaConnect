//! The phone's run-command list (`vyoma_core::commands`); on Linux a command that couldn't
//! start is said in a desktop notification.

pub use vyoma_core::commands::{Commands, RUNCOMMAND, RUNCOMMAND_REQUEST};

/// The phone can't be told (stock runcommand has no reply), so the PC says so.
pub fn failed(name: String, why: &'static str) {
    let _ = std::process::Command::new("notify-send")
        .args(["--app-name=Vyoma", "--hint=string:desktop-entry:vyoma", "--icon=dialog-warning", "--"])
        .arg(format!("\u{201c}{name}\u{201d} didn't run"))
        .arg(format!("{why}. Check it in Vyoma's Commands."))
        .status();
}
