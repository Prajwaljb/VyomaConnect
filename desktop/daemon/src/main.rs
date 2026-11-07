//! Vyoma's desktop app, standing alone: finds phones, pairs with them and talks to them with no
//! kdeconnectd. This process is the daemon; the GUI will be another one, talking to it over the
//! `dev.vyoma.Desktop1` D-Bus interface below, so closing it never drops a link.
//!
//! Environment, for development beside the user's own KDE Connect:
//! - `VYOMA_DIR`: where its identity and pairings live (default `~/.config/vyoma-desktop`).
//! - `VYOMA_ALLOW`: device ids, comma-separated; when set, every other device is ignored.
//! - `VYOMA_ANNOUNCE`: addresses to announce to instead of the broadcast address.

mod notify;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use vyoma_core::device::{Command, Event, Hub};
use vyoma_core::identity::Identity;
use vyoma_core::lan::{Lan, Ports, Rules, UDP_PORT};
use vyoma_core::link::packet;
use vyoma_core::trust::TrustStore;
use zbus::fdo;

use notify::{Notice, Notifier, Pressed};

const BUS_NAME: &str = "dev.vyoma.Desktop";
const OBJECT_PATH: &str = "/dev/vyoma/Desktop";

/// What the GUI is shown of one device.
#[derive(Clone, Default)]
struct DeviceView {
    name: String,
    kind: String,
    paired: bool,
    linked: bool,
    /// The code while a pairing is on screen.
    key: String,
}

struct Control {
    commands: mpsc::UnboundedSender<Command>,
    devices: Arc<Mutex<HashMap<String, DeviceView>>>,
    lan: Arc<Lan>,
    targets: Vec<SocketAddr>,
}

#[zbus::interface(name = "dev.vyoma.Desktop1")]
impl Control {
    fn pair(&self, id: String) -> fdo::Result<()> {
        self.command(Command::Pair(id))
    }

    fn accept(&self, id: String) -> fdo::Result<()> {
        self.command(Command::Accept(id))
    }

    fn reject(&self, id: String) -> fdo::Result<()> {
        self.command(Command::Reject(id))
    }

    fn unpair(&self, id: String) -> fdo::Result<()> {
        self.command(Command::Unpair(id))
    }

    /// Announces this PC again, so devices that missed it link now.
    async fn refresh(&self) -> fdo::Result<()> {
        self.lan.announce(&self.targets).await.map_err(|e| fdo::Error::Failed(format!("{e:#}")))
    }

    /// Every device known: (id, name, type, paired, linked, code while pairing).
    fn devices(&self) -> Vec<(String, String, String, bool, bool, String)> {
        let mut out: Vec<_> = self.devices.lock().unwrap().iter().map(|(id, d)| (id.clone(), d.name.clone(), d.kind.clone(), d.paired, d.linked, d.key.clone())).collect();
        out.sort();
        out
    }
}

impl Control {
    fn command(&self, c: Command) -> fdo::Result<()> {
        self.commands.send(c).map_err(|_| fdo::Error::Failed("stopping".into()))
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "vyoma_desktop=info,vyoma_core=info".into()))
        .init();

    let dir = std::env::var_os("VYOMA_DIR").map(PathBuf::from).unwrap_or_else(default_dir);
    let me = Arc::new(Identity::load_or_create_in(&dir)?);
    let trust = TrustStore::open(&dir)?;
    let devices: Arc<Mutex<HashMap<String, DeviceView>>> = Arc::default();
    for id in trust.ids() {
        let t = trust.get(id).unwrap();
        devices.lock().unwrap().insert(id.clone(), DeviceView { name: t.name.clone(), kind: t.kind.clone(), paired: true, ..Default::default() });
    }
    let allow: Option<Vec<String>> = std::env::var("VYOMA_ALLOW").ok().map(|s| s.split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect());
    if let Some(allow) = &allow {
        tracing::info!("talking to {} allowed device(s) only", allow.len());
    }
    let targets = announce_targets()?;

    let (events_tx, mut events) = mpsc::unbounded_channel();
    let hub = Hub::new(me.cert.to_vec(), trust, events_tx);
    let linked = hub.linked();
    let rules = Rules {
        admits: Box::new(move |id| allow.as_ref().is_none_or(|a| a.iter().any(|x| x == id))),
        linked: Box::new(move |id| linked.lock().unwrap().contains(id)),
    };
    let lan = Lan::bind(me.clone(), identity(&me), rules, Ports::default()).await?;
    let (links_tx, links) = mpsc::channel(16);
    let (commands, commands_rx) = mpsc::unbounded_channel();
    tokio::spawn(lan.clone().run(links_tx));
    tokio::spawn(hub.run(links, commands_rx));

    let conn = zbus::Connection::session().await.context("no session bus")?;
    let control = Control { commands: commands.clone(), devices: devices.clone(), lan: lan.clone(), targets: targets.clone() };
    conn.object_server().at(OBJECT_PATH, control).await?;
    conn.request_name(BUS_NAME).await.context("another Vyoma desktop app is running")?;
    let notifier = Notifier::new(conn.clone());
    let (pressed_tx, mut pressed) = mpsc::unbounded_channel();
    {
        let notifier = notifier.clone();
        tokio::spawn(async move {
            if let Err(e) = notifier.presses(pressed_tx).await {
                tracing::warn!("can't hear notification buttons: {e:#}");
            }
        });
    }

    lan.announce(&targets).await?;
    tracing::info!("{} is up as {}", env!("CARGO_PKG_NAME"), me.device_id);

    // Device id to the notification showing its pairing.
    let mut prompts: HashMap<String, u32> = HashMap::new();
    loop {
        tokio::select! {
            Some(event) = events.recv() => on_event(event, &devices, &notifier, &mut prompts).await,
            Some(Pressed { notification, action }) = pressed.recv() => {
                let Some(id) = prompts.iter().find(|(_, n)| **n == notification).map(|(id, _)| id.clone()) else { continue };
                let _ = commands.send(match action.as_str() {
                    "accept" => Command::Accept(id),
                    "reject" => Command::Reject(id),
                    _ => continue,
                });
            }
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    for n in prompts.values() {
        notifier.close(*n).await;
    }
    Ok(())
}

async fn on_event(event: Event, devices: &Mutex<HashMap<String, DeviceView>>, notifier: &Notifier, prompts: &mut HashMap<String, u32>) {
    match event {
        Event::Linked { id, name, kind, paired } => {
            let mut all = devices.lock().unwrap();
            let d = all.entry(id).or_default();
            (d.name, d.kind, d.paired, d.linked) = (name, kind, paired, true);
        }
        Event::Gone { id } => {
            let mut all = devices.lock().unwrap();
            // Unpaired devices are only listed while they're here.
            if all.get(&id).is_some_and(|d| d.paired) {
                all.get_mut(&id).unwrap().linked = false;
            } else {
                all.remove(&id);
            }
        }
        Event::Prompt { id, name, key } => {
            set_key(devices, &id, &key);
            let body = format!("Code {key}. Accept only if {name} shows the same code.");
            let notice = Notice { icon: "smartphone", title: &format!("Pair with {name}?"), body: &body, actions: &[("accept", "Accept"), ("reject", "Reject")], sticky: true, replaces: prompts.get(&id).copied() };
            show_prompt(notifier, prompts, id, notice).await;
        }
        Event::Asked { id, name, key } => {
            set_key(devices, &id, &key);
            let body = format!("Code {key}. Accept on {name} if it shows the same code.");
            let notice = Notice { icon: "smartphone", title: &format!("Pairing with {name}"), body: &body, actions: &[], sticky: true, replaces: prompts.get(&id).copied() };
            show_prompt(notifier, prompts, id, notice).await;
        }
        Event::PairingOver { id } => {
            set_key(devices, &id, "");
            if let Some(n) = prompts.remove(&id) {
                notifier.close(n).await;
            }
        }
        Event::Paired { id, name } => {
            if let Some(d) = devices.lock().unwrap().get_mut(&id) {
                (d.paired, d.key) = (true, String::new());
            }
            if let Some(n) = prompts.remove(&id) {
                notifier.close(n).await;
            }
            tell(notifier, "smartphone", &format!("Paired with {name}"), "").await;
        }
        Event::Unpaired { id, name } => {
            {
                let mut all = devices.lock().unwrap();
                if all.get(&id).is_some_and(|d| d.linked) {
                    all.get_mut(&id).unwrap().paired = false;
                } else {
                    all.remove(&id);
                }
            }
            tell(notifier, "smartphone", &format!("Unpaired from {name}"), "").await;
        }
        Event::Packet { id, packet } => on_packet(&id, &packet, devices, notifier).await,
    }
}

/// The stock plugins, as they come.
async fn on_packet(id: &str, packet: &Value, devices: &Mutex<HashMap<String, DeviceView>>, notifier: &Notifier) {
    let name = devices.lock().unwrap().get(id).map(|d| d.name.clone()).unwrap_or_default();
    match packet["type"].as_str().unwrap_or("") {
        "kdeconnect.ping" => {
            let message = packet["body"]["message"].as_str().unwrap_or("Ping!");
            tell(notifier, "smartphone", &name, message).await;
        }
        other => tracing::debug!("{id}: no plugin for {other}"),
    }
}

fn set_key(devices: &Mutex<HashMap<String, DeviceView>>, id: &str, key: &str) {
    if let Some(d) = devices.lock().unwrap().get_mut(id) {
        d.key = key.to_owned();
    }
}

async fn show_prompt(notifier: &Notifier, prompts: &mut HashMap<String, u32>, id: String, notice: Notice<'_>) {
    match notifier.show(notice).await {
        Ok(n) => {
            prompts.insert(id, n);
        }
        Err(e) => tracing::warn!("couldn't show the pairing prompt: {e:#}"),
    }
}

async fn tell(notifier: &Notifier, icon: &str, title: &str, body: &str) {
    if let Err(e) = notifier.show(Notice { icon, title, body, actions: &[], sticky: false, replaces: None }).await {
        tracing::warn!("couldn't show \"{title}\": {e:#}");
    }
}

fn default_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"))
        .join("vyoma-desktop")
}

fn announce_targets() -> Result<Vec<SocketAddr>> {
    let Ok(list) = std::env::var("VYOMA_ANNOUNCE") else {
        return Ok(vec![SocketAddr::from(([255, 255, 255, 255], UDP_PORT))]);
    };
    list.split(',').map(|a| Ok(SocketAddr::new(a.trim().parse::<IpAddr>().with_context(|| format!("VYOMA_ANNOUNCE: {a}"))?, UDP_PORT))).collect()
}

fn identity(me: &Identity) -> Value {
    packet(
        "kdeconnect.identity",
        json!({
            "deviceId": me.device_id,
            "deviceName": host_name(),
            "deviceType": if has_battery() { "laptop" } else { "desktop" },
            "protocolVersion": 8,
            "incomingCapabilities": ["kdeconnect.ping"],
            "outgoingCapabilities": ["kdeconnect.ping"],
        }),
    )
}

fn host_name() -> String {
    let name = std::fs::read_to_string("/proc/sys/kernel/hostname").unwrap_or_default();
    vyoma_core::lan::device_name(name.trim())
}

/// A system battery (not a mouse's or a phone's): this is a laptop.
fn has_battery() -> bool {
    let Ok(dir) = std::fs::read_dir("/sys/class/power_supply") else { return false };
    dir.flatten().any(|e| {
        let read = |f: &str| std::fs::read_to_string(e.path().join(f)).unwrap_or_default();
        read("type").trim() == "Battery" && read("scope").trim() != "Device"
    })
}
