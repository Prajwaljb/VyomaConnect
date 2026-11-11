//! The desktop app (the daemon that owns the links) as the window sees it. On Linux that's its
//! D-Bus interface, `dev.vyoma.Desktop1`; on macOS a local socket (`vyoma-mac`'s `control.rs`),
//! behind the same [Daemon]: [connect] picks.

#[cfg(target_os = "linux")]
use std::sync::{Mutex, OnceLock};

use anyhow::Result;
#[cfg(target_os = "linux")]
use anyhow::Context;
#[cfg(target_os = "linux")]
use futures_util::StreamExt;
use serde_json::Value;

/// The daemon of this system: D-Bus on Linux, the socket elsewhere.
pub fn connect() -> Result<Box<dyn Daemon>> {
    #[cfg(target_os = "linux")]
    return Ok(Box::new(DbusDaemon::connect()?));
    #[cfg(not(target_os = "linux"))]
    return Ok(Box::new(crate::socket::SocketDaemon::connect()?));
}

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
    /// Its texts can be read and sent here (the Messages page).
    pub can_text: bool,
    /// How fast it answers (ms), once timed.
    pub rtt: Option<i64>,
    /// When it connected (unix ms), while connected.
    pub since: Option<i64>,
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
            can_text: can("text"),
            rtt: v["rtt"].as_i64(),
            since: v["since"].as_i64(),
            clip: v["clipboardSync"].as_str().unwrap_or("plug").to_owned(),
            permits: Vec::new(),
        }
    }
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
    /// The button that fixes it ("Open ports"), or empty.
    pub action: String,
}

/// This PC's setup.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Health {
    pub features: Vec<Feature>,
    pub installable: bool,
    pub installing: bool,
    pub checks: Vec<Check>,
    pub notes: Vec<String>,
    pub update: Update,
}

/// The PC app's version and whether a newer one is out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Update {
    pub current: String,
    /// The newer release, while there is one (and checks are on).
    pub latest: Option<String>,
    pub page: String,
    pub checks: bool,
    pub updating: bool,
    /// The clone it was installed from is still there to build from.
    pub can_update: bool,
    /// Installed by a package manager, which updates it.
    pub packaged: bool,
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
                    action: c["action"].as_str().unwrap_or("").to_owned(),
                })
                .collect(),
            notes: strings(&v["notes"]),
            update: Update {
                current: v["update"]["current"].as_str().unwrap_or("").to_owned(),
                latest: v["update"]["latest"].as_str().map(str::to_owned),
                page: v["update"]["page"].as_str().unwrap_or("").to_owned(),
                checks: v["update"]["checks"].as_bool().unwrap_or(true),
                updating: v["update"]["updating"].as_bool().unwrap_or(false),
                can_update: v["update"]["canUpdate"].as_bool().unwrap_or(false),
                packaged: v["update"]["packaged"].as_bool().unwrap_or(false),
            },
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

/// A phone's text conversation, as the Messages page lists it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Conversation {
    pub thread: i64,
    /// Who it's with, by contact name where there is one.
    pub name: String,
    /// Its newest text.
    pub snippet: String,
    /// When (unix ms).
    pub date: i64,
    /// The newest text was sent from the phone.
    pub mine: bool,
    pub unread: bool,
}

/// The phone's conversations: [state] is "loading" (asked, nothing back yet), "ready" or "silent"
/// (no answer: the phone doesn't share its texts, or has none).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Inbox {
    pub state: String,
    pub list: Vec<Conversation>,
}

impl Inbox {
    /// From the daemon's `Conversations` JSON.
    pub fn from_json(v: &Value) -> Self {
        let text = |e: &Value, k: &str| e[k].as_str().unwrap_or("").to_owned();
        let list = v["list"].as_array().into_iter().flatten().map(|c| Conversation {
            thread: c["thread"].as_i64().unwrap_or(0),
            name: text(c, "name"),
            snippet: text(c, "snippet"),
            date: c["date"].as_i64().unwrap_or(0),
            mine: c["mine"].as_bool().unwrap_or(false),
            unread: c["unread"].as_bool().unwrap_or(false),
        });
        Inbox { state: v["state"].as_str().unwrap_or("loading").to_owned(), list: list.collect() }
    }
}

/// One text in a conversation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextData {
    pub body: String,
    pub date: i64,
    pub mine: bool,
    pub failed: bool,
    /// Sent from here, not yet back from the phone.
    pub sending: bool,
}

/// A conversation's texts, oldest first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThreadData {
    pub name: String,
    pub number: String,
    pub texts: Vec<TextData>,
}

impl ThreadData {
    /// From the daemon's `Thread` JSON.
    pub fn from_json(v: &Value) -> Self {
        let flag = |e: &Value, k: &str| e[k].as_bool().unwrap_or(false);
        let texts = v["texts"].as_array().into_iter().flatten().map(|t| TextData {
            body: t["body"].as_str().unwrap_or("").to_owned(),
            date: t["date"].as_i64().unwrap_or(0),
            mine: flag(t, "mine"),
            failed: flag(t, "failed"),
            sending: flag(t, "sending"),
        });
        ThreadData { name: v["name"].as_str().unwrap_or("").to_owned(), number: v["number"].as_str().unwrap_or("").to_owned(), texts: texts.collect() }
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
    /// Opening the firewall from the window is over.
    FirewallDone { ok: bool },
    /// The activity has something new.
    Activity,
    /// A phone's texts have something new.
    Messages,
}

/// What the window asks of the daemon.
pub trait Daemon: Send + Sync {
    fn phones(&self) -> Result<Vec<PhoneData>>;
    /// One of the per-device calls: Pair, Accept, Reject, Unpair, Ring, Browse.
    fn act(&self, method: &str, id: &str) -> Result<()>;
    fn send_files(&self, id: &str, paths: Vec<String>) -> Result<()>;
    fn search(&self) -> Result<()>;
    /// A fresh pairing QR code's text; it replaces any before (see the daemon's PairingCode).
    fn pairing_code(&self) -> Result<String>;
    /// Nothing shows the code now: it can't pair anyone.
    fn end_pairing_code(&self) -> Result<()>;
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
    /// Opens Vyoma's ports in the firewall; [Event::FirewallDone] follows.
    fn open_firewall(&self) -> Result<()>;
    /// Updates the PC app to the newest release; the daemon restarts when it's done.
    fn update(&self) -> Result<()>;
    fn set_update_checks(&self, on: bool) -> Result<()>;
    /// What phones did here, newest first.
    fn activity(&self) -> Result<Vec<ActivityEntry>>;
    fn clear_activity(&self) -> Result<()>;
    /// The PC's wallpaper as a colour grid, for the window's accent and canvas.
    fn wallpaper(&self) -> Result<crate::look::Wallpaper>;
    /// The phone's conversations, newest first; the first look on each connection asks the phone,
    /// and [Event::Messages] follows as they come.
    fn conversations(&self, id: &str) -> Result<Inbox>;
    fn thread(&self, id: &str, thread: i64) -> Result<ThreadData>;
    /// Texts [text] to the conversation's people, from the phone.
    fn send_text(&self, id: &str, thread: i64, text: &str) -> Result<()>;
    /// Asks the phone to let its texts in (its owner confirms there), or keeps them open if they are.
    fn unlock_texts(&self, id: &str) -> Result<()>;
    /// Blocks until the daemon says something (an error once it's gone away).
    fn next_event(&self) -> Result<Event>;
}

#[cfg(target_os = "linux")]
const BUS_NAME: &str = "dev.vyoma.Desktop";
#[cfg(target_os = "linux")]
const PATH: &str = "/dev/vyoma/Desktop";
#[cfg(target_os = "linux")]
const INTERFACE: &str = "dev.vyoma.Desktop1";

/// zbus is async: its calls run here, each blocked on by the window's worker threads.
#[cfg(target_os = "linux")]
pub fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().expect("a tokio runtime"))
}

#[cfg(target_os = "linux")]
pub struct DbusDaemon {
    proxy: zbus::Proxy<'static>,
    /// Its signals, subscribed on the first wait so none fall between two.
    signals: Mutex<Option<zbus::proxy::SignalStream<'static>>>,
}

#[cfg(target_os = "linux")]
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

#[cfg(target_os = "linux")]
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

    fn pairing_code(&self) -> Result<String> {
        self.call("PairingCode", &())
    }

    fn end_pairing_code(&self) -> Result<()> {
        self.call("EndPairingCode", &())
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

    fn open_firewall(&self) -> Result<()> {
        self.call("OpenFirewall", &())
    }

    fn update(&self) -> Result<()> {
        self.call("Update", &())
    }

    fn set_update_checks(&self, on: bool) -> Result<()> {
        self.call("SetUpdateChecks", &(on,))
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

    fn conversations(&self, id: &str) -> Result<Inbox> {
        let json: String = self.call("Conversations", &(id,))?;
        Ok(Inbox::from_json(&serde_json::from_str(&json)?))
    }

    fn thread(&self, id: &str, thread: i64) -> Result<ThreadData> {
        let json: String = self.call("Thread", &(id, thread))?;
        Ok(ThreadData::from_json(&serde_json::from_str(&json)?))
    }

    fn send_text(&self, id: &str, thread: i64, text: &str) -> Result<()> {
        self.call("TextThread", &(id, thread, text))
    }

    fn unlock_texts(&self, id: &str) -> Result<()> {
        self.call("UnlockTexts", &(id,))
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
                    Some("MessagesChanged") => return Ok(Event::Messages),
                    Some("SetupDone") => {
                        let (ok,): (bool,) = message.body().deserialize()?;
                        return Ok(Event::SetupDone { ok });
                    }
                    Some("FirewallDone") => {
                        let (ok,): (bool,) = message.body().deserialize()?;
                        return Ok(Event::FirewallDone { ok });
                    }
                    _ => {}
                }
            }
        })
    }
}

/// A D-Bus error's own message, without zbus's wrapping.
/// What went wrong, in words for the person at the window: bus errors name the bus, not the problem.
#[cfg(target_os = "linux")]
fn reason(e: zbus::Error) -> String {
    const NOT_RUNNING: &str = "Vyoma isn't running here right now. It restarts by itself; if it doesn't, use Start Vyoma";
    match e {
        zbus::Error::MethodError(name, msg, _) => match name.as_str() {
            "org.freedesktop.DBus.Error.ServiceUnknown" | "org.freedesktop.DBus.Error.NameHasNoOwner" => NOT_RUNNING.into(),
            "org.freedesktop.DBus.Error.NoReply" | "org.freedesktop.DBus.Error.Timeout" => "Vyoma didn't answer in time. Try again".into(),
            "org.freedesktop.DBus.Error.UnknownMethod" => "This window and Vyoma's background service are different versions. Restart Vyoma to match them".into(),
            // The daemon's own words; "stopping" is it shutting down mid-request.
            _ => match msg {
                Some(m) if m == "stopping" => "Vyoma is restarting. Try again in a moment".into(),
                Some(m) => m,
                None => "Something went wrong in Vyoma. Try again".into(),
            },
        },
        zbus::Error::InputOutput(_) => NOT_RUNNING.into(),
        e => e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Reads the running desktop app and waits for one change (prints no names).
    #[cfg(target_os = "linux")]
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
        assert_eq!((h.update.latest.clone(), h.update.checks), (None, true), "an older daemon says nothing of updates");
        let u = Health::from_json(&json!({ "update": { "current": "0.3.0", "latest": "0.4.0", "checks": true, "canUpdate": true } })).update;
        assert_eq!((u.current.as_str(), u.latest.as_deref(), u.can_update, u.updating), ("0.3.0", Some("0.4.0"), true, false));
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
