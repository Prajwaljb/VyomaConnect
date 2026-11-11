//! Running at login, as a LaunchAgent: `vyoma-mac --install` copies the binary into
//! ~/Library/Application Support/Vyoma/bin, writes ~/Library/LaunchAgents/dev.vyoma.desktop.plist
//! and starts it; `--uninstall` stops it and removes the plist (the identity and pairings stay).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

pub const LABEL: &str = "dev.vyoma.desktop";

/// Where Vyoma keeps its identity, pairings and binary on a Mac.
pub fn support_dir() -> PathBuf {
    crate::mac::home().join("Library/Application Support/Vyoma")
}

fn agent_path() -> PathBuf {
    crate::mac::home().join(format!("Library/LaunchAgents/{LABEL}.plist"))
}

fn log_path() -> PathBuf {
    crate::mac::home().join("Library/Logs/Vyoma.log")
}

/// The agent: started at login and kept running (restarted if it stops), its output in [log].
pub fn plist(program: &Path, log: &Path) -> String {
    let program = xml(&program.to_string_lossy());
    let log = xml(&log.to_string_lossy());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{LABEL}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{program}</string>
	</array>
	<key>RunAtLoad</key>
	<true/>
	<key>KeepAlive</key>
	<true/>
	<key>ProcessType</key>
	<string>Interactive</string>
	<key>StandardOutPath</key>
	<string>{log}</string>
	<key>StandardErrorPath</key>
	<string>{log}</string>
</dict>
</plist>
"#
    )
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

fn uid() -> Result<String> {
    let out = Command::new("id").arg("-u").output().context("no id command")?;
    let uid = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if uid.is_empty() || !uid.bytes().all(|b| b.is_ascii_digit()) {
        bail!("couldn't tell who you are");
    }
    Ok(uid)
}

pub fn install() -> Result<()> {
    let bin = support_dir().join("bin");
    std::fs::create_dir_all(&bin).with_context(|| format!("couldn't make {}", bin.display()))?;
    let target = bin.join("vyoma-mac");
    let me = std::env::current_exe().context("can't tell where this program is")?;
    if me.canonicalize().ok() != target.canonicalize().ok() {
        // To a new file then over the old one, so a running copy keeps its own.
        let fresh = bin.join(".vyoma-mac.new");
        std::fs::copy(&me, &fresh).with_context(|| format!("couldn't copy to {}", fresh.display()))?;
        std::fs::rename(&fresh, &target)?;
    }
    let agent = agent_path();
    std::fs::create_dir_all(agent.parent().unwrap())?;
    std::fs::create_dir_all(log_path().parent().unwrap())?;
    std::fs::write(&agent, plist(&target, &log_path())).with_context(|| format!("couldn't write {}", agent.display()))?;
    let domain = format!("gui/{}", uid()?);
    // Stops a copy that's running from an earlier install; fine if there's none.
    let _ = Command::new("launchctl").args(["bootout", &format!("{domain}/{LABEL}")]).output();
    let status = Command::new("launchctl").args(["bootstrap", &domain]).arg(&agent).status().context("no launchctl")?;
    if !status.success() {
        bail!("launchctl couldn't start it");
    }
    println!("Vyoma is running and starts with your Mac.");
    println!("If macOS asks whether vyoma-mac may find devices on your local network, or accept incoming connections, allow it: that's how your phone reaches it.");
    println!("Then on the phone, open Vyoma and pair with this Mac. Logs: {}", log_path().display());
    Ok(())
}

pub fn uninstall() -> Result<()> {
    let domain = format!("gui/{}", uid()?);
    let _ = Command::new("launchctl").args(["bootout", &format!("{domain}/{LABEL}")]).output();
    let agent = agent_path();
    if agent.exists() {
        std::fs::remove_file(&agent)?;
    }
    println!("Vyoma is stopped and won't start with your Mac. Your pairings are kept in {}.", support_dir().display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_agent_starts_at_login_and_stays_up() {
        let p = plist(Path::new("/Users/sam/Library/Application Support/Vyoma/bin/vyoma-mac"), Path::new("/Users/sam/Library/Logs/Vyoma.log"));
        assert!(p.contains("<string>dev.vyoma.desktop</string>"));
        assert!(p.contains("<string>/Users/sam/Library/Application Support/Vyoma/bin/vyoma-mac</string>"));
        assert!(p.contains("<key>RunAtLoad</key>\n\t<true/>"));
        assert!(p.contains("<key>KeepAlive</key>\n\t<true/>"));
    }

    #[test]
    fn odd_paths_stay_one_string() {
        let p = plist(Path::new("/Users/a&b/<x>/vyoma-mac"), Path::new("/tmp/log"));
        assert!(p.contains("<string>/Users/a&amp;b/&lt;x&gt;/vyoma-mac</string>"));
    }
}
