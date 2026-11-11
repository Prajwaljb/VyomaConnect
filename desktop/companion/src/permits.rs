//! What each paired phone may do on this PC, beyond the basics (notifications, clipboard, media,
//! sharing). Everything is allowed until the user turns it off in the desktop app's window; only
//! what's off is kept, in `permissions.json`. A refused request is answered with `vyoma.denied`
//! (`feature`, its `name`, the `request` and the `path`/`key` it was about) so the phone says why
//! instead of waiting, and turning a feature off stops what it was doing.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::{Context, Result};
use serde_json::{Value, json};
use tokio::sync::watch;

pub const DENIED: &str = "vyoma.denied";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Permit {
    Screen,
    Control,
    Files,
    Camera,
    Webcam,
    Power,
}

impl Permit {
    pub const ALL: [Permit; 6] = [Permit::Screen, Permit::Control, Permit::Files, Permit::Camera, Permit::Webcam, Permit::Power];

    pub fn key(self) -> &'static str {
        match self {
            Permit::Screen => "screen",
            Permit::Control => "control",
            Permit::Files => "files",
            Permit::Camera => "camera",
            Permit::Webcam => "webcam",
            Permit::Power => "power",
        }
    }

    /// As the window and the phone show it.
    pub fn name(self) -> &'static str {
        match self {
            Permit::Screen => "See and control this screen",
            Permit::Control => "Touchpad, keyboard and controller",
            Permit::Files => "Use this PC's files",
            Permit::Camera => "Take photos with this PC's camera",
            Permit::Webcam => "Be this PC's webcam and mic",
            Permit::Power => "Wake, unlock, sleep and shut down",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Permit::Screen => "Watch the screen live, use it by touch, take screenshots",
            Permit::Control => "Move the pointer, type, paste, play games",
            Permit::Files => "Browse your home folder from the phone, and save to it",
            Permit::Camera => "Photos land on the phone",
            Permit::Webcam => "The phone's camera and microphone in calls here",
            Permit::Power => "Also log out and restart",
        }
    }

    pub fn from_key(key: &str) -> Option<Permit> {
        Permit::ALL.into_iter().find(|p| p.key() == key)
    }

    /// The feature a packet from the phone needs, if it needs one.
    pub fn of(packet_type: &str) -> Option<Permit> {
        Some(match packet_type {
            "vyoma.screen.request" | "vyoma.screen.touch" | "vyoma.screen.input" | "vyoma.screenshot.request" => Permit::Screen,
            "vyoma.pointer.request" | "vyoma.gamepad.state" | "vyoma.usb.accessory" | "vyoma.desktop.step" | "vyoma.desktop.switch" | "vyoma.paste" | "kdeconnect.mousepad.request" => Permit::Control,
            "vyoma.files.list" | "vyoma.files.get" | "vyoma.files.put" | "vyoma.xfer.begin" | "vyoma.xfer.data" => Permit::Files,
            "vyoma.camera.request" => Permit::Camera,
            "vyoma.webcam" | "vyoma.webcam.mic" => Permit::Webcam,
            "vyoma.pc.power" | "vyoma.pc.wake" => Permit::Power,
            _ => return None,
        })
    }
}

/// Every phone's switches. [Default] allows everything and keeps nothing (the companion beside
/// kdeconnectd, and tests).
pub struct Permits {
    file: Option<PathBuf>,
    /// Phone id → what's off.
    off: Mutex<BTreeMap<String, BTreeSet<Permit>>>,
    changed: watch::Sender<u64>,
}

impl Default for Permits {
    fn default() -> Self {
        Permits { file: None, off: Mutex::default(), changed: watch::Sender::new(0) }
    }
}

impl Permits {
    /// Reads [file] (`{phone id: [feature, ...]}` of what's off); missing or unreadable is all on.
    pub fn load(file: PathBuf) -> Permits {
        let off = std::fs::read_to_string(&file)
            .ok()
            .and_then(|s| {
                serde_json::from_str::<BTreeMap<String, Vec<String>>>(&s)
                    // Every feature is back on for every phone then: say so where it can be found.
                    .map_err(|e| tracing::warn!("{} isn't valid ({e}): every feature is on until it's fixed", file.display()))
                    .ok()
            })
            .unwrap_or_default()
            .into_iter()
            .map(|(id, keys)| (id, keys.iter().filter_map(|k| Permit::from_key(k)).collect()))
            .collect();
        Permits { file: Some(file), off: Mutex::new(off), changed: watch::Sender::new(0) }
    }

    pub fn allows(&self, phone: &str, p: Permit) -> bool {
        !self.off.lock().unwrap().get(phone).is_some_and(|s| s.contains(&p))
    }

    /// Each feature and whether [phone] has it.
    pub fn of(&self, phone: &str) -> Vec<(Permit, bool)> {
        Permit::ALL.into_iter().map(|p| (p, self.allows(phone, p))).collect()
    }

    pub fn set(&self, phone: &str, p: Permit, on: bool) -> Result<()> {
        {
            let mut all = self.off.lock().unwrap();
            let mine = all.entry(phone.to_owned()).or_default();
            let moved = if on { mine.remove(&p) } else { mine.insert(p) };
            if mine.is_empty() {
                all.remove(phone);
            }
            if !moved {
                return Ok(());
            }
            self.save(&all)?;
        }
        self.changed.send_modify(|n| *n += 1);
        Ok(())
    }

    /// An unpaired phone starts over with everything on, if it pairs again.
    pub fn forget(&self, phone: &str) {
        let mut all = self.off.lock().unwrap();
        if all.remove(phone).is_some() {
            if let Err(e) = self.save(&all) {
                tracing::warn!("couldn't save the permissions: {e:#}");
            }
        }
    }

    /// Ticks whenever a switch moves.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    fn save(&self, all: &BTreeMap<String, BTreeSet<Permit>>) -> Result<()> {
        let Some(file) = &self.file else { return Ok(()) };
        let map: BTreeMap<&String, Vec<&str>> = all.iter().map(|(id, s)| (id, s.iter().map(|p| p.key()).collect())).collect();
        let tmp = file.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(&map)?).context("writing the permissions")?;
        std::fs::rename(&tmp, file).context("saving the permissions")?;
        Ok(())
    }
}

/// The answer to [request] (a packet [p] needing [permit]) when it's off.
pub fn denied(permit: Permit, p: &Value) -> Value {
    let b = &p["body"];
    let mut body = json!({ "feature": permit.key(), "name": permit.name(), "request": p["type"] });
    for k in ["path", "key"] {
        if let Some(v) = b[k].as_str() {
            body[k] = json!(v);
        }
    }
    vyoma_core::link::packet(DENIED, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everything_is_on_until_turned_off_and_only_offs_are_kept() {
        let dir = std::env::temp_dir().join(format!("vyoma-permits-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("permissions.json");
        let p = Permits::load(file.clone());
        assert!(Permit::ALL.iter().all(|&f| p.allows("a", f)));
        let ticks = p.subscribe();
        p.set("a", Permit::Screen, false).unwrap();
        assert!(ticks.has_changed().unwrap());
        assert!(!p.allows("a", Permit::Screen) && p.allows("b", Permit::Screen) && p.allows("a", Permit::Files));
        assert_eq!(std::fs::read_to_string(&file).unwrap().replace([' ', '\n'], ""), r#"{"a":["screen"]}"#);
        assert!(!Permits::load(file.clone()).allows("a", Permit::Screen), "kept across restarts");
        p.set("a", Permit::Screen, true).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap().replace([' ', '\n'], ""), "{}");
        p.set("a", Permit::Power, false).unwrap();
        p.forget("a");
        assert!(Permits::load(file).allows("a", Permit::Power));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn requests_map_to_their_feature_and_refusals_say_which() {
        assert_eq!(Permit::of("vyoma.pc.wake"), Some(Permit::Power));
        assert_eq!(Permit::of("kdeconnect.mousepad.request"), Some(Permit::Control));
        assert_eq!(Permit::of("vyoma.stats.request"), None);
        let d = denied(Permit::Files, &json!({ "type": "vyoma.files.list", "body": { "path": "Music" } }));
        assert_eq!(d["type"], DENIED);
        assert_eq!((d["body"]["feature"].as_str(), d["body"]["path"].as_str(), d["body"]["request"].as_str()), (Some("files"), Some("Music"), Some("vyoma.files.list")));
    }
}
