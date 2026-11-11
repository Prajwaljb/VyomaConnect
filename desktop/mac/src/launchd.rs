//! Running at login, as a LaunchAgent: `vyoma-mac --install` writes
//! ~/Library/LaunchAgents/dev.vyoma.desktop.plist and starts it; `--uninstall` stops it and removes
//! the plist (the identity and pairings stay). From Vyoma.app the agent runs the copy inside the
//! app (the window installs it when it opens, again if the app moved), so its notifications are
//! Vyoma's and an updated app updates it; a bare binary is copied into
//! ~/Library/Application Support/Vyoma/bin first.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

pub const LABEL: &str = "dev.vyoma.desktop";
/// The phone's commands run with this: launchd's own PATH leaves out Homebrew's programs.
const PATH: &str = "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin";
/// Vyoma.app's identifier, which macOS's Login Items shows the agent under.
pub const BUNDLE_ID: &str = "dev.vyoma.Vyoma";

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

/// The app [exe] is in, if it's Vyoma.app's own helper (`….app/Contents/MacOS/vyoma-mac`).
pub fn app_of(exe: &Path) -> Option<&Path> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    (macos.file_name()? == "MacOS" && contents.file_name()? == "Contents" && app.extension()? == "app").then_some(app)
}

/// The agent: started at login and kept running (restarted if it stops), its output in [log].
pub fn plist(program: &Path, log: &Path) -> String {
    let bundled = if app_of(program).is_some() {
        format!("\n\t<key>AssociatedBundleIdentifiers</key>\n\t<array>\n\t\t<string>{BUNDLE_ID}</string>\n\t</array>")
    } else {
        String::new()
    };
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
	<key>EnvironmentVariables</key>
	<dict>
		<key>PATH</key>
		<string>{PATH}</string>
	</dict>
	<key>StandardOutPath</key>
	<string>{log}</string>
	<key>StandardErrorPath</key>
	<string>{log}</string>{bundled}
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
    let me = std::env::current_exe().context("can't tell where this program is")?;
    let me = me.canonicalize().unwrap_or(me);
    let bin = support_dir().join("bin");
    let target = if app_of(&me).is_some() { me.clone() } else { bin.join("vyoma-mac") };
    if me.canonicalize().ok() != target.canonicalize().ok() {
        std::fs::create_dir_all(&bin).with_context(|| format!("couldn't make {}", bin.display()))?;
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
    println!("If macOS asks whether Vyoma (or vyoma-mac) may find devices on your local network, or accept incoming connections, allow it: that's how your phone reaches it.");
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
        assert!(!p.contains("AssociatedBundleIdentifiers"), "a bare binary isn't the app");
        assert!(p.contains("<string>/opt/homebrew/bin:"), "commands find Homebrew's programs");
    }

    #[test]
    fn inside_the_app_it_runs_in_place_under_its_name() {
        let exe = Path::new("/Applications/Vyoma.app/Contents/MacOS/vyoma-mac");
        assert_eq!(app_of(exe), Some(Path::new("/Applications/Vyoma.app")));
        assert_eq!(app_of(Path::new("/Users/sam/Downloads/vyoma-mac")), None);
        assert_eq!(app_of(Path::new("/x/Contents/MacOS/vyoma-mac")), None, "not in an .app");
        let p = plist(exe, Path::new("/tmp/log"));
        assert!(p.contains("<string>/Applications/Vyoma.app/Contents/MacOS/vyoma-mac</string>"));
        assert!(p.contains(&format!("<key>AssociatedBundleIdentifiers</key>\n\t<array>\n\t\t<string>{BUNDLE_ID}</string>")));
    }

    #[test]
    fn odd_paths_stay_one_string() {
        let p = plist(Path::new("/Users/a&b/<x>/vyoma-mac"), Path::new("/tmp/log"));
        assert!(p.contains("<string>/Users/a&amp;b/&lt;x&gt;/vyoma-mac</string>"));
    }
}
