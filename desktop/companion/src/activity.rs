//! What phones did on this PC, for the desktop app's Activity page: screenshots, photos, the live
//! screen, files, power, the webcam, the touchpad and controller, refusals, shares and commands.
//! The newest [KEEP] entries stay, in `activity.json` beside the identity; nothing of it is logged.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use tokio::sync::watch;

pub const KEEP: usize = 500;

/// What happened, for its icon: screenshot, photo, screen, files, power, webcam, control, touchpad,
/// refused, share, command.
pub type Kind = &'static str;

/// The log. [Default] keeps it in memory only (the companion beside kdeconnectd, and tests).
pub struct Activity {
    file: Option<PathBuf>,
    /// Oldest first.
    entries: Mutex<VecDeque<Value>>,
    changed: watch::Sender<u64>,
}

impl Default for Activity {
    fn default() -> Self {
        Activity { file: None, entries: Mutex::default(), changed: watch::Sender::new(0) }
    }
}

impl Activity {
    pub fn load(file: PathBuf) -> Activity {
        let entries: VecDeque<Value> = std::fs::read_to_string(&file)
            .ok()
            .and_then(|s| serde_json::from_str::<Vec<Value>>(&s).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|e| e["text"].is_string())
            .collect();
        Activity { file: Some(file), entries: Mutex::new(entries), changed: watch::Sender::new(0) }
    }

    /// [phone] (its id, and its name as shown) did [text].
    pub fn add(&self, phone: &str, name: &str, kind: Kind, text: impl Into<String>) {
        let time = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        {
            let mut all = self.entries.lock().unwrap();
            all.push_back(json!({ "time": time, "phone": phone, "name": name, "kind": kind, "text": text.into() }));
            while all.len() > KEEP {
                all.pop_front();
            }
            self.save(&all);
        }
        self.changed.send_modify(|n| *n += 1);
    }

    /// Newest first.
    pub fn recent(&self) -> Vec<Value> {
        self.entries.lock().unwrap().iter().rev().cloned().collect()
    }

    pub fn clear(&self) {
        let mut all = self.entries.lock().unwrap();
        all.clear();
        self.save(&all);
        drop(all);
        self.changed.send_modify(|n| *n += 1);
    }

    /// Ticks on every entry.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    fn save(&self, all: &VecDeque<Value>) {
        let Some(file) = &self.file else { return };
        let tmp = file.with_extension("json.tmp");
        let written = serde_json::to_vec(all).map_err(anyhow::Error::from).and_then(|b| Ok(std::fs::write(&tmp, b)?)).and_then(|()| Ok(std::fs::rename(&tmp, file)?));
        if let Err(e) = written {
            tracing::warn!("couldn't save the activity: {e:#}");
        }
    }
}

/// A file's name from a path the phone sent ("Music/a.mp3" → "a.mp3").
pub fn file_name(rel: &str) -> String {
    rel.rsplit('/').find(|s| !s.is_empty()).unwrap_or(rel).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_newest_across_restarts() {
        let dir = std::env::temp_dir().join(format!("vyoma-activity-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("activity.json");
        let a = Activity::load(file.clone());
        let mut ticks = a.subscribe();
        for i in 0..KEEP + 3 {
            a.add("p", "Phone", "files", format!("file {i}"));
        }
        assert!(ticks.has_changed().unwrap());
        let again = Activity::load(file);
        let recent = again.recent();
        assert_eq!(recent.len(), KEEP);
        assert_eq!(recent[0]["text"], format!("file {}", KEEP + 2));
        assert_eq!(recent[0]["kind"], "files");
        again.clear();
        assert!(again.recent().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(file_name("Music/a b.mp3"), "a b.mp3");
        assert_eq!(file_name("Docs/"), "Docs");
    }
}
