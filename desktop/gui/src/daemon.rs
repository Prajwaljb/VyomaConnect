//! The desktop app (the daemon that owns the links) as the window sees it. On Linux that's its
//! D-Bus interface, `dev.vyoma.Desktop1`; macOS will reach it over a local socket instead, behind
//! the same [Daemon].

use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde_json::Value;

/// One device as the window shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PhoneData {
    pub id: String,
    pub name: String,
    pub paired: bool,
    pub linked: bool,
    pub code: String,
    pub asking: bool,
    pub via: String,
    pub battery: Option<i64>,
    pub charging: bool,
    pub can_ring: bool,
    pub can_browse: bool,
    pub can_share: bool,
    /// Each SIM's signal: its label, network ("5G") and level 0..4.
    pub sims: Vec<(String, String, i32)>,
    /// Background clipboard sync: "on", "ask" (plugged in, waiting for Allow) or "plug".
    pub clip: String,
    /// What it may do here (paired phones).
    pub permits: Vec<Permission>,
}

/// A command phones can run here.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CommandData {
    pub key: String,
    pub name: String,
    pub command: String,
    pub fingerprint: bool,
    pub every_time: bool,
}

impl CommandData {
    /// From the daemon's `Commands` JSON ({key: {name, command, fingerprint, everyTime}}), by name.
    pub fn list(v: &Value) -> Vec<CommandData> {
        let mut out: Vec<CommandData> = v
            .as_object()
            .map(|o| {
                o.iter()
                    .map(|(k, c)| CommandData { key: k.clone(), name: c["name"].as_str().unwrap_or("").to_owned(), command: c["command"].as_str().unwrap_or("").to_owned(), fingerprint: c["fingerprint"].as_bool().unwrap_or(false), every_time: c["everyTime"].as_bool().unwrap_or(false) })
                    .collect()
            })
            .unwrap_or_default();
        out.sort_by_key(|c| c.name.to_lowercase());
        out
    }
}

/// One thing a phone may do on this PC.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Permission {
    pub key: String,
    pub name: String,
    pub detail: String,
    pub on: bool,
}

impl Permission {
    /// From the daemon's `Permissions` JSON.
    pub fn list(v: &Value) -> Vec<Permission> {
        let text = |p: &Value, k: &str| p[k].as_str().unwrap_or("").to_owned();
        v.as_array().map(|a| a.iter().map(|p| Permission { key: text(p, "key"), name: text(p, "name"), detail: text(p, "detail"), on: p["on"].as_bool().unwrap_or(true) }).collect()).unwrap_or_default()
    }
}

impl PhoneData {
    /// From the daemon's `Details` JSON.
    pub fn from_details(id: &str, v: &Value) -> Self {
        let can = |k: &str| v["can"][k].as_bool().unwrap_or(false);
        PhoneData {
            id: id.to_owned(),
            name: v["name"].as_str().unwrap_or("Phone").to_owned(),
            paired: v["paired"].as_bool().unwrap_or(false),
            linked: v["linked"].as_bool().unwrap_or(false),
            code: v["key"].as_str().unwrap_or("").to_owned(),
            asking: v["asking"].as_bool().unwrap_or(false),
            via: v["via"].as_str().unwrap_or("").to_owned(),
            battery: v["battery"]["charge"].as_i64().filter(|c| (0..=100).contains(c)),
            charging: v["battery"]["charging"].as_bool().unwrap_or(false),
            can_ring: can("ring"),
            can_browse: can("browse"),
            can_share: can("share"),
            sims: sims(&v["signal"]),
            clip: v["clipboardSync"].as_str().unwrap_or("plug").to_owned(),
            permits: Vec::new(),
        }
    }
}

/// The SIMs in `signal` ({sim, network, level}), named "SIM 1", "SIM 2" in order; one alone is "Signal".
fn sims(v: &Value) -> Vec<(String, String, i32)> {
    let list = v.as_array().map(Vec::as_slice).unwrap_or_default();
    list.iter()
        .enumerate()
        .map(|(i, s)| {
            let label = if list.len() == 1 { "Signal".to_owned() } else { format!("SIM {}", i + 1) };
            (label, s["network"].as_str().unwrap_or("").to_owned(), s["level"].as_i64().unwrap_or(0).clamp(0, 4) as i32)
        })
        .collect()
}

/// A feature that needs something on the PC.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Feature {
    pub name: String,
    pub ready: bool,
    /// What would make it ready ("sshfs", "a udev rule"), if anything here can.
    pub needs: Vec<String>,
    pub unavailable: bool,
}

/// Something no package fixes: ok, worth knowing (info), or in the way (warn).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Check {
    pub name: String,
    pub state: String,
    pub detail: String,
}

/// This PC's setup.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Health {
    pub features: Vec<Feature>,
    pub installable: bool,
    pub installing: bool,
    pub checks: Vec<Check>,
    pub notes: Vec<String>,
}

impl Health {
    /// From the daemon's `Health` JSON.
    pub fn from_json(v: &Value) -> Self {
        let strings = |v: &Value| v.as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_owned)).collect()).unwrap_or_default();
        let list = |k: &str| v[k].as_array().cloned().unwrap_or_default();
        Health {
            features: list("features")
                .iter()
                .map(|f| Feature {
                    name: f["name"].as_str().unwrap_or("").to_owned(),
                    ready: f["ready"].as_bool().unwrap_or(true),
                    needs: strings(&f["needs"]),
                    unavailable: f["unavailable"].as_bool().unwrap_or(false),
                })
                .collect(),
            installable: v["installable"].as_bool().unwrap_or(false),
            installing: v["installing"].as_bool().unwrap_or(false),
            checks: list("checks")
                .iter()
                .map(|c| Check {
                    name: c["name"].as_str().unwrap_or("").to_owned(),
                    state: c["state"].as_str().unwrap_or("info").to_owned(),
                    detail: c["detail"].as_str().unwrap_or("").to_owned(),
                })
                .collect(),
            notes: strings(&v["notes"]),
        }
    }

    /// How many things need the user: features not ready, and checks in the way.
    pub fn attention(&self) -> usize {
        self.features.iter().filter(|f| !f.ready).count() + self.checks.iter().filter(|c| c.state == "warn").count()
    }
}

/// Something a phone did on this PC.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActivityEntry {
    /// Unix seconds.
    pub time: i64,
    /// The phone's name.
    pub name: String,
    /// For its icon: screenshot, photo, screen, files, share, power, webcam, control, refused, command.
    pub kind: String,
    pub text: String,
}

impl ActivityEntry {
    /// From the daemon's `Activity` JSON.
    pub fn list(v: &Value) -> Vec<ActivityEntry> {
        let text = |e: &Value, k: &str| e[k].as_str().unwrap_or("").to_owned();
        v.as_array().map(|a| a.iter().map(|e| ActivityEntry { time: e["time"].as_i64().unwrap_or(0), name: text(e, "name"), kind: text(e, "kind"), text: text(e, "text") }).collect()).unwrap_or_default()
    }
}

/// What the daemon says without being asked.
#[derive(Debug, PartialEq)]
pub enum Event {
    /// Something the window shows may have changed.
    Changed,
    /// A batch of files went to [id]; [error] is empty when all [count] arrived.
    Sent { id: String, count: u32, error: String },
    /// An install from the window is over.
    SetupDone { ok: bool },
    /// The activity has something new.
    Activity,
}

/// What the window asks of the daemon.
pub trait Daemon: Send + Sync {
    fn phones(&self) -> Result<Vec<PhoneData>>;
    /// One of the per-device calls: Pair, Accept, Reject, Unpair, Ring, Browse.
    fn act(&self, method: &str, id: &str) -> Result<()>;
    fn send_files(&self, id: &str, paths: Vec<String>) -> Result<()>;
    fn search(&self) -> Result<()>;
    fn health(&self) -> Result<Health>;
    /// Turns one of a phone's [Permission]s on or off; [Event::Changed] follows.
    fn set_permission(&self, id: &str, key: &str, on: bool) -> Result<()>;
    /// Gives the plugged-in phone the one-time adb grant background clipboard sync needs.
    fn allow_clipboard(&self, id: &str) -> Result<()>;
    fn commands(&self) -> Result<Vec<CommandData>>;
    /// Changes a command, or adds one when [key] is empty; its key.
    fn save_command(&self, key: &str, name: &str, command: &str, fingerprint: bool, every_time: bool) -> Result<String>;
    fn remove_command(&self, key: &str) -> Result<()>;
    /// Installs what's missing; [Event::SetupDone] follows.
    fn install(&self) -> Result<()>;
    /// What phones did here, newest first.
    fn activity(&self) -> Result<Vec<ActivityEntry>>;
    fn clear_activity(&self) -> Result<()>;
    /// The PC's wallpaper as a colour grid, for the window's accent and canvas.
    fn wallpaper(&self) -> Result<crate::look::Wallpaper>;
    /// Blocks until the daemon says something (an error once it's gone away).
    fn next_event(&self) -> Result<Event>;
}

const BUS_NAME: &str = "dev.vyoma.Desktop";
const PATH: &str = "/dev/vyoma/Desktop";
const INTERFACE: &str = "dev.vyoma.Desktop1";

/// zbus is async: its calls run here, each blocked on by the window's worker threads.
pub fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().expect("a tokio runtime"))
}

pub struct DbusDaemon {
    proxy: zbus::Proxy<'static>,
    /// Its signals, subscribed on the first wait so none fall between two.
    signals: Mutex<Option<zbus::proxy::SignalStream<'static>>>,
}

impl DbusDaemon {
    pub fn connect() -> Result<Self> {
        runtime().block_on(async {
            let conn = zbus::Connection::session().await.context("no session bus")?;
            let proxy = zbus::Proxy::new(&conn, BUS_NAME, PATH, INTERFACE).await?;
            Ok(Self { proxy, signals: Mutex::new(None) })
        })
    }

    fn call<R>(&self, method: &str, body: &(impl serde::Serialize + zbus::zvariant::DynamicType)) -> Result<R>
    where
        R: for<'d> zbus::zvariant::DynamicDeserialize<'d>,
    {
        runtime().block_on(self.proxy.call(method, body)).map_err(|e| anyhow::anyhow!(reason(e)))
    }
}

impl Daemon for DbusDaemon {
    fn phones(&self) -> Result<Vec<PhoneData>> {
        let list: Vec<(String, String, String, bool, bool, String)> = self.call("Devices", &())?;
        let mut out = Vec::with_capacity(list.len());
        for (id, ..) in list {
            let details: String = self.call("Details", &(&id,))?;
            let mut phone = PhoneData::from_details(&id, &serde_json::from_str(&details)?);
            if phone.paired {
                // An older daemon has no permissions: the card stays hidden.
                if let Ok(json) = self.call::<String>("Permissions", &(&id,)) {
                    phone.permits = Permission::list(&serde_json::from_str(&json)?);
                }
            }
            out.push(phone);
        }
        Ok(out)
    }

    fn act(&self, method: &str, id: &str) -> Result<()> {
        self.call(method, &(id,))
    }

    fn send_files(&self, id: &str, paths: Vec<String>) -> Result<()> {
        self.call("SendFiles", &(id, paths))
    }

    fn search(&self) -> Result<()> {
        self.call("Refresh", &())
    }

    fn health(&self) -> Result<Health> {
        let json: String = self.call("Health", &())?;
        Ok(Health::from_json(&serde_json::from_str(&json)?))
    }

    fn install(&self) -> Result<()> {
        self.call("Install", &())
    }

    fn set_permission(&self, id: &str, key: &str, on: bool) -> Result<()> {
        self.call("SetPermission", &(id, key, on))
    }

    fn commands(&self) -> Result<Vec<CommandData>> {
        let json: String = self.call("Commands", &())?;
        Ok(CommandData::list(&serde_json::from_str(&json)?))
    }

    fn save_command(&self, key: &str, name: &str, command: &str, fingerprint: bool, every_time: bool) -> Result<String> {
        self.call("SaveCommand", &(key, name, command, fingerprint, every_time))
    }

    fn remove_command(&self, key: &str) -> Result<()> {
        self.call("RemoveCommand", &(key,))
    }

    fn allow_clipboard(&self, id: &str) -> Result<()> {
        self.call("AllowClipboardSync", &(id,))
    }

    fn activity(&self) -> Result<Vec<ActivityEntry>> {
        let json: String = self.call("Activity", &())?;
        Ok(ActivityEntry::list(&serde_json::from_str(&json)?))
    }

    fn clear_activity(&self) -> Result<()> {
        self.call("ClearActivity", &())
    }

    fn wallpaper(&self) -> Result<crate::look::Wallpaper> {
        let json: String = self.call("Wallpaper", &())?;
        let v: Value = serde_json::from_str(&json)?;
        let colors = v["colors"].as_array().context("no colours")?.iter().map(|c| {
            let b = |i: usize| c[i].as_u64().unwrap_or(0).min(255) as u8;
            [b(0), b(1), b(2)]
        });
        Ok(crate::look::Wallpaper { cols: v["cols"].as_u64().unwrap_or(0) as usize, rows: v["rows"].as_u64().unwrap_or(0) as usize, colors: colors.collect() })
    }

    fn next_event(&self) -> Result<Event> {
        let mut signals = self.signals.lock().unwrap();
        runtime().block_on(async {
            if signals.is_none() {
                *signals = Some(self.proxy.receive_all_signals().await?);
            }
            loop {
                let message = signals.as_mut().unwrap().next().await.context("the daemon went away")?;
                let header = message.header();
                match header.member().map(|m| m.as_str()) {
                    Some("Changed") => return Ok(Event::Changed),
                    Some("FilesSent") => {
                        let (id, count, error): (String, u32, String) = message.body().deserialize()?;
                        return Ok(Event::Sent { id, count, error });
                    }
                    Some("ActivityChanged") => return Ok(Event::Activity),
                    Some("SetupDone") => {
                        let (ok,): (bool,) = message.body().deserialize()?;
                        return Ok(Event::SetupDone { ok });
                    }
                    _ => {}
                }
            }
        })
    }
}

/// A D-Bus error's own message, without zbus's wrapping.
fn reason(e: zbus::Error) -> String {
    match e {
        zbus::Error::MethodError(_, Some(msg), _) => msg,
        e => e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Reads the running desktop app and waits for one change (prints no names).
    #[test]
    #[ignore]
    fn reads_the_running_daemon() {
        let d = DbusDaemon::connect().unwrap();
        let phones = d.phones().unwrap();
        for p in &phones {
            println!("paired {} linked {} via {:?} battery {} can ring/browse/share {}/{}/{}", p.paired, p.linked, p.via, p.battery.is_some(), p.can_ring, p.can_browse, p.can_share);
        }
        assert!(!phones.is_empty());
    }

    #[test]
    fn reads_the_pcs_health() {
        let v = json!({ "features": [{ "name": "Sound", "ready": true, "needs": [] }, { "name": "Browsing the phone", "ready": false, "needs": ["sshfs"] }],
            "installable": true, "checks": [{ "name": "Firewall", "state": "info", "detail": "d" }, { "name": "KDE Connect", "state": "warn", "detail": "k" }], "notes": ["n"] });
        let h = Health::from_json(&v);
        assert_eq!((h.features.len(), h.features[1].needs.clone(), h.installable, h.notes.len()), (2, vec!["sshfs".to_owned()], true, 1));
        assert_eq!(h.attention(), 2, "a missing feature and a check in the way; info isn't a fault");
    }

    #[test]
    fn reads_the_daemons_details() {
        let v = json!({ "name": "P", "paired": true, "linked": true, "key": "", "asking": false, "via": "cable",
            "battery": { "charge": 81, "charging": true, "low": false }, "can": { "ring": true, "browse": false, "share": true } });
        let p = PhoneData::from_details("x", &v);
        assert_eq!((p.via.as_str(), p.battery, p.charging, p.can_ring, p.can_browse), ("cable", Some(81), true, true, false));
        let away = PhoneData::from_details("y", &json!({ "name": "Q", "battery": { "charge": -1 } }));
        assert_eq!((away.paired, away.battery, away.via.as_str()), (false, None, ""));
    }
}
