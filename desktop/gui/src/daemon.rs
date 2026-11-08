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
        }
    }
}

/// What the daemon says without being asked.
#[derive(Debug, PartialEq)]
pub enum Event {
    /// Something the window shows may have changed.
    Changed,
    /// A batch of files went to [id]; [error] is empty when all [count] arrived.
    Sent { id: String, count: u32, error: String },
}

/// What the window asks of the daemon.
pub trait Daemon: Send + Sync {
    fn phones(&self) -> Result<Vec<PhoneData>>;
    /// One of the per-device calls: Pair, Accept, Reject, Unpair, Ring, Browse.
    fn act(&self, method: &str, id: &str) -> Result<()>;
    fn send_files(&self, id: &str, paths: Vec<String>) -> Result<()>;
    fn search(&self) -> Result<()>;
    /// Blocks until the daemon says something (an error once it's gone away).
    fn next_event(&self) -> Result<Event>;
}

const BUS_NAME: &str = "dev.vyoma.Desktop";
const PATH: &str = "/dev/vyoma/Desktop";
const INTERFACE: &str = "dev.vyoma.Desktop1";

/// zbus is async: its calls run here, each blocked on by the window's worker threads.
fn runtime() -> &'static tokio::runtime::Runtime {
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
            out.push(PhoneData::from_details(&id, &serde_json::from_str(&details)?));
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
    fn reads_the_daemons_details() {
        let v = json!({ "name": "P", "paired": true, "linked": true, "key": "", "asking": false, "via": "cable",
            "battery": { "charge": 81, "charging": true, "low": false }, "can": { "ring": true, "browse": false, "share": true } });
        let p = PhoneData::from_details("x", &v);
        assert_eq!((p.via.as_str(), p.battery, p.charging, p.can_ring, p.can_browse), ("cable", Some(81), true, true, false));
        let away = PhoneData::from_details("y", &json!({ "name": "Q", "battery": { "charge": -1 } }));
        assert_eq!((away.paired, away.battery, away.via.as_str()), (false, None, ""));
    }
}
