//! Keeping the PC app current. Once a day (unless turned off on This PC) it asks GitHub for the
//! newest release, and when that's newer than this build a notification offers to update.
//! Updating runs `install.sh --update` from the folder Vyoma was installed from (`install.sh`
//! notes it in `source`): it moves that clone to the release, builds, installs and restarts this
//! app, pairings and settings kept. It runs as its own transient unit, as it restarts this one, and
//! tells by itself if it fails; the new build says it's done when it starts. Installed by a
//! package manager (under /usr), it only says a release is out: updating is the package's job.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::{Value, json};
use vyoma_core::release::is_newer;

pub use vyoma_core::release::REPO;
/// The release this was built from, kept in one file for the phone app and the PC app.
const VERSION: &str = include_str!("../../../VERSION");
const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// Long enough after login not to add to it.
const FIRST_CHECK: Duration = Duration::from_secs(90);
const UNIT: &str = "vyoma-update";

pub fn current() -> &'static str {
    VERSION.trim()
}

#[derive(Default)]
struct State {
    /// Checks are on unless turned off.
    off: bool,
    /// When GitHub was last asked (unix ms).
    checked: i64,
    /// The newest release seen, and its page.
    latest: String,
    page: String,
    /// The newest release a notification was shown for, to show it once.
    told: String,
    /// The version that ran last, to say when an update is done.
    ran: String,
}

pub struct Updates {
    file: PathBuf,
    source: PathBuf,
    state: Mutex<State>,
}

impl Updates {
    pub fn load(dir: &Path) -> Self {
        let v: Value = std::fs::read_to_string(dir.join("updates.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let s = |k: &str| v[k].as_str().unwrap_or("").to_owned();
        let state = State { off: v["off"].as_bool().unwrap_or(false), checked: v["checked"].as_i64().unwrap_or(0), latest: s("latest"), page: s("page"), told: s("told"), ran: s("ran") };
        Self { file: dir.join("updates.json"), source: dir.join("source"), state: Mutex::new(state) }
    }

    fn save(&self) {
        let s = self.state.lock().unwrap();
        let text = json!({ "off": s.off, "checked": s.checked, "latest": s.latest, "page": s.page, "told": s.told, "ran": s.ran }).to_string();
        if let Err(e) = std::fs::write(&self.file, text) {
            tracing::warn!("couldn't save the update state: {e}");
        }
    }

    /// For This PC: this version, a newer one if there is, whether checks are on and an update runs.
    pub fn status(&self) -> Value {
        let s = self.state.lock().unwrap();
        let newer = (!s.off && is_newer(&s.latest, current())).then(|| s.latest.clone());
        json!({ "current": current(), "latest": newer, "page": s.page, "checks": !s.off, "updating": updating(), "canUpdate": !packaged() && self.clone_dir().is_some(), "packaged": packaged() })
    }

    pub fn set_checks(&self, on: bool) {
        self.state.lock().unwrap().off = !on;
        self.save();
    }

    /// The git clone Vyoma was installed from, while it's still there.
    fn clone_dir(&self) -> Option<PathBuf> {
        let dir = PathBuf::from(std::fs::read_to_string(&self.source).ok()?.trim());
        dir.join("desktop/daemon/install.sh").is_file().then_some(dir)
    }

    /// Starts the update; it runs on after this app restarts.
    pub async fn start(&self) -> Result<()> {
        if updating() {
            return Ok(());
        }
        anyhow::ensure!(!packaged(), "Vyoma came from your package manager: update it there.");
        let dir = self.clone_dir().context("Vyoma's files have moved since it was installed, so it can't update itself. Get the newest version from Vyoma's website.")?;
        let script = dir.join("desktop/daemon/install.sh");
        let status = tokio::process::Command::new("systemd-run")
            .args(["--user", "--collect", "--quiet", &format!("--unit={UNIT}"), "sh"])
            .arg(&script)
            .arg("--update")
            .status()
            .await;
        if !status.as_ref().is_ok_and(|s| s.success()) {
            tracing::warn!("couldn't start the update: {status:?}");
            anyhow::bail!("The update didn't start. Try again in a minute.");
        }
        tracing::info!("update started");
        Ok(())
    }

    /// Says when an update finished, then checks once a day while checks are on.
    pub async fn run(self: std::sync::Arc<Self>) {
        let finished = {
            let mut s = self.state.lock().unwrap();
            let was = std::mem::replace(&mut s.ran, current().to_owned());
            (!was.is_empty() && was != current()).then_some(was)
        };
        self.save();
        if finished.is_some() {
            tell_updated().await;
        }
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            let due = {
                let s = self.state.lock().unwrap();
                !s.off && now_ms() - s.checked >= DAY_MS
            };
            if due {
                match latest().await {
                    Ok((version, page)) => {
                        let tell = {
                            let mut s = self.state.lock().unwrap();
                            s.checked = now_ms();
                            (s.latest, s.page) = (version.clone(), page.clone());
                            let tell = is_newer(&version, current()) && s.told != version;
                            if tell {
                                s.told = version.clone();
                            }
                            tell
                        };
                        self.save();
                        if tell {
                            let me = self.clone();
                            tokio::spawn(async move { me.offer(&version, &page).await });
                        }
                    }
                    Err(e) => tracing::info!("couldn't check for updates: {e:#}"),
                }
            }
            tokio::time::sleep(Duration::from_secs(60 * 60)).await;
        }
    }

    /// The notification about [version], with Update (unless a package manager does that) and What's new.
    async fn offer(&self, version: &str, page: &str) {
        let (update, body) = if packaged() {
            ("", "Update it with your package manager. Your phones stay paired.")
        } else {
            ("--action=update=Update", "Update this PC now: it takes a few minutes and keeps your phones paired.")
        };
        let answer = tokio::process::Command::new("notify-send")
            .args(["--app-name=Vyoma", "--icon=vyoma", "--hint=string:desktop-entry:vyoma", "--wait", "--action=notes=What's new"])
            .args((!update.is_empty()).then_some(update))
            .arg("--")
            .arg(format!("Vyoma {version} is out"))
            .arg(body)
            .kill_on_drop(true)
            .output()
            .await;
        match answer.map(|a| String::from_utf8_lossy(&a.stdout).trim().to_owned()).as_deref() {
            Ok("update") => {
                if let Err(e) = self.start().await {
                    // start's errors are written to be read (the raw cause went to the log).
                    let _ = tokio::process::Command::new("notify-send").args(["--app-name=Vyoma", "--icon=vyoma", "--hint=string:desktop-entry:vyoma", "Couldn't update"]).arg(e.to_string()).status().await;
                }
            }
            Ok("notes") => crate::share::open(page),
            _ => {}
        }
    }
}

/// Installed by a package manager: this binary lives under /usr.
fn packaged() -> bool {
    std::env::current_exe().is_ok_and(|p| p.starts_with("/usr/"))
}

/// Whether an update is running now.
fn updating() -> bool {
    std::process::Command::new("systemctl").args(["--user", "--quiet", "is-active", UNIT]).status().is_ok_and(|s| s.success())
}

async fn tell_updated() {
    let body = "The phone app's update is on the release page, or let Obtainium keep it current.";
    let _ = tokio::process::Command::new("notify-send")
        .args(["--app-name=Vyoma", "--icon=vyoma", "--hint=string:desktop-entry:vyoma", "--"])
        .arg(format!("Vyoma is updated to {}", current()))
        .arg(body)
        .status()
        .await;
}

/// The newest release on GitHub: its version and its page.
async fn latest() -> Result<(String, String)> {
    let r = vyoma_core::release::latest().await?;
    Ok((r.version, r.page))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_as_numbers() {
        assert!(is_newer("0.4.0", "0.3.0"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("v1.0", "0.99.0"));
        assert!(!is_newer("0.3.0", "0.3.0"));
        assert!(!is_newer("0.2.9", "0.3.0"));
        assert!(!is_newer("", "0.3.0"));
        assert!(!is_newer("0.4.0-beta", "0.3.0"));
        assert!(!current().is_empty() && current().split('.').all(|p| p.parse::<u64>().is_ok()));
    }
}
