//! Telling when a newer Vyoma for this PC is out, the same on every PC side that doesn't update
//! itself (the Mac, Windows): once a day, unless turned off in the window, GitHub is asked for the
//! newest release; when that's newer than this build and carries this PC's download ([carries]),
//! the side tells the person, once per version, and the window's Update opens the release's page.
//! The phones stay paired across the reinstall. State in `updates.json` in the side's folder.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use crate::release::{self, Release, is_newer};

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// Long enough after login not to add to it.
const FIRST_CHECK: Duration = Duration::from_secs(90);

#[derive(Default)]
struct State {
    off: bool,
    /// When GitHub was last asked (unix ms).
    checked: i64,
    /// The newest release with this PC's download, and its page.
    latest: String,
    page: String,
    /// The newest version the person was told of.
    told: String,
}

pub struct Updates {
    file: PathBuf,
    current: String,
    /// Whether a release file is this PC's download ("Vyoma-mac-0.12.0.zip").
    carries: fn(&str) -> bool,
    state: Mutex<State>,
}

impl Updates {
    pub fn load(dir: &Path, current: &str, carries: fn(&str) -> bool) -> Arc<Self> {
        let file = dir.join("updates.json");
        let v: Value = std::fs::read_to_string(&file).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null);
        let s = |k: &str| v[k].as_str().unwrap_or("").to_owned();
        let state = State { off: v["off"].as_bool().unwrap_or(false), checked: v["checked"].as_i64().unwrap_or(0), latest: s("latest"), page: s("page"), told: s("told") };
        Arc::new(Self { file, current: current.trim().to_owned(), carries, state: Mutex::new(state) })
    }

    fn save(&self) {
        let s = self.state.lock().unwrap();
        let text = json!({ "off": s.off, "checked": s.checked, "latest": s.latest, "page": s.page, "told": s.told }).to_string();
        drop(s);
        if let Err(e) = std::fs::write(&self.file, text) {
            tracing::warn!("couldn't save the update state: {e}");
        }
    }

    /// For the window's Updates card (the Linux app's fields): a newer release while there is one.
    pub fn status(&self) -> Value {
        let s = self.state.lock().unwrap();
        let newer = (!s.off && is_newer(&s.latest, &self.current)).then(|| s.latest.clone());
        json!({ "current": self.current, "latest": newer, "page": s.page, "checks": !s.off, "updating": false, "canUpdate": false, "packaged": false, "download": true })
    }

    pub fn set_checks(&self, on: bool) {
        self.state.lock().unwrap().off = !on;
        self.save();
    }

    /// The newer release's page, to download it from.
    pub fn page(&self) -> Option<String> {
        let s = self.state.lock().unwrap();
        (is_newer(&s.latest, &self.current) && s.page.starts_with("https://")).then(|| s.page.clone())
    }

    /// Checks once a day while checks are on: [tell] gets a version to tell the person of (once
    /// each), [changed] runs after each answer.
    pub async fn run(self: Arc<Self>, tell: impl Fn(String), changed: impl Fn()) {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            let due = {
                let s = self.state.lock().unwrap();
                !s.off && now_ms() - s.checked >= DAY_MS
            };
            if due {
                match release::latest().await {
                    Ok(r) => {
                        if let Some(version) = self.seen(&r, now_ms()) {
                            tell(version);
                        }
                        changed();
                    }
                    Err(e) => tracing::info!("couldn't check for updates: {e:#}"),
                }
            }
            tokio::time::sleep(Duration::from_secs(60 * 60)).await;
        }
    }

    /// Takes in what GitHub said; the version to tell about, once, if it's newer and has this
    /// PC's download.
    fn seen(&self, r: &Release, now: i64) -> Option<String> {
        let tell = {
            let mut s = self.state.lock().unwrap();
            s.checked = now;
            // A release without this PC's download isn't an update for it.
            if r.assets.iter().any(|a| (self.carries)(a)) {
                (s.latest, s.page) = (r.version.clone(), r.page.clone());
            }
            let tell = is_newer(&s.latest, &self.current) && s.told != s.latest;
            if tell {
                s.told = s.latest.clone();
            }
            tell.then(|| s.latest.clone())
        };
        self.save();
        tell
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_once_and_only_of_a_release_with_this_pcs_download() {
        let dir = std::env::temp_dir().join(format!("vyoma-updates-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mac = |a: &str| a.starts_with("Vyoma-mac") && a.ends_with(".zip");
        let u = Updates::load(&dir, "0.9.0\n", mac);
        let release = |v: &str, ours: bool| Release {
            version: v.into(),
            page: format!("https://github.com/x/releases/tag/v{v}"),
            assets: if ours { vec![format!("Vyoma-mac-{v}.zip"), "vyoma.apk".into()] } else { vec!["vyoma.apk".into()] },
        };
        assert_eq!(u.seen(&release("0.10.0", false), 1), None, "not this PC's download");
        assert_eq!(u.status()["latest"], Value::Null);
        assert_eq!(u.seen(&release("0.10.0", true), 2).as_deref(), Some("0.10.0"));
        assert_eq!(u.seen(&release("0.10.0", true), 3), None, "told once");
        assert_eq!(u.status()["latest"], "0.10.0");
        assert_eq!(u.page().as_deref(), Some("https://github.com/x/releases/tag/v0.10.0"));
        u.set_checks(false);
        assert_eq!(u.status()["latest"], Value::Null);
        assert_eq!(Updates::load(&dir, "0.9.0", mac).status()["checks"], false, "kept");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
