//! Vyoma's desktop app, standing alone: finds phones, pairs with them and talks to them with no
//! kdeconnectd. This process is the daemon; the GUI will be another one, talking to it over the
//! `dev.vyoma.Desktop1` D-Bus interface below, so closing it never drops a link.
//!
//! Environment, for development beside the user's own KDE Connect:
//! - `VYOMA_DIR`: where its identity and pairings live (default `~/.config/vyoma-desktop`).
//! - `VYOMA_ALLOW`: device ids, comma-separated; when set, every other device is ignored.
//! - `VYOMA_ANNOUNCE`: addresses to announce to instead of the broadcast address.

mod clipboard;
mod notify;
mod plugins;
mod share;
mod system;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use vyoma_core::device::{Command, Event, Hub};
use vyoma_core::identity::Identity;
use vyoma_core::lan::{Lan, Ports, Rules, UDP_PORT};
use vyoma_core::link::packet;
use vyoma_core::trust::TrustStore;
use zbus::fdo;

use clipboard::{CLIPBOARD, CLIPBOARD_CONNECT, Clipboard};
use notify::{Notice, Notifier, Pressed};
use share::{SHARE, SHARE_UPDATE, Shared};
use system::{Ringer, Session};

const BUS_NAME: &str = "dev.vyoma.Desktop";
const OBJECT_PATH: &str = "/dev/vyoma/Desktop";
/// How often the PC's battery is read; it's sent on when it changed.
const BATTERY_EVERY: Duration = Duration::from_secs(30);

/// What the GUI is shown of one device, and what its plugins keep.
#[derive(Clone, Default)]
pub struct DeviceView {
    pub name: String,
    pub kind: String,
    pub paired: bool,
    pub linked: bool,
    /// The code while a pairing is on screen.
    pub key: String,
    /// The packet types it takes.
    pub incoming: Vec<String>,
    /// Charge, charging, low.
    pub battery: Option<(i64, bool, bool)>,
    /// Each SIM's network and level (0..4).
    pub signal: Option<Vec<Value>>,
    /// The certificate its links show, which its payloads must show too.
    pub cert: Vec<u8>,
}

type Devices = Arc<Mutex<HashMap<String, DeviceView>>>;

/// What the D-Bus side asks of the main loop beyond the hub's commands.
enum Ask {
    StopRinging,
}

struct Control {
    me: Arc<Identity>,
    notifier: Notifier,
    commands: mpsc::UnboundedSender<Command>,
    asks: mpsc::UnboundedSender<Ask>,
    devices: Devices,
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

    /// One device's state as JSON: battery, signal and the rest of [devices].
    fn details(&self, id: String) -> fdo::Result<String> {
        let all = self.devices.lock().unwrap();
        let d = all.get(&id).ok_or_else(|| fdo::Error::UnknownObject(id.clone()))?;
        let battery = d.battery.map(|(charge, charging, low)| json!({ "charge": charge, "charging": charging, "low": low }));
        Ok(json!({ "name": d.name, "type": d.kind, "paired": d.paired, "linked": d.linked, "key": d.key, "battery": battery, "signal": d.signal }).to_string())
    }

    /// Makes the device ring, to find it.
    fn ring(&self, id: String) -> fdo::Result<()> {
        let takes = self.devices.lock().unwrap().get(&id).is_some_and(|d| plugins::takes(d, plugins::FIND));
        if !takes {
            return Err(fdo::Error::NotSupported(format!("{id} can't be rung")));
        }
        self.command(Command::Send(id, packet(plugins::FIND, json!({}))))
    }

    /// Sends files to the device; it fetches them one by one. Fails at once if one can't be read.
    fn send_files(&self, id: String, paths: Vec<String>) -> fdo::Result<()> {
        let pinned = self.sharing(&id)?;
        let files = paths.iter().map(|p| share::outgoing(p)).collect::<Result<Vec<_>>>().map_err(|e| fdo::Error::InvalidArgs(format!("{e:#}")))?;
        if files.is_empty() {
            return Ok(());
        }
        let (me, commands, notifier) = (self.me.clone(), self.commands.clone(), self.notifier.clone());
        tokio::spawn(async move {
            if let Err(e) = share::send_files(&commands, &id, &me, &pinned, files).await {
                tracing::warn!("{id}: {e:#}");
                plugins::tell(&notifier, "Couldn't send", &format!("{e:#}")).await;
            }
        });
        Ok(())
    }

    /// Sends text to the device, or opens a link there.
    fn send_text(&self, id: String, text: String) -> fdo::Result<()> {
        self.sharing(&id)?;
        self.command(Command::Send(id, share::text_packet(&text)))
    }

    /// Stops this PC ringing for a device that's looking for it.
    fn stop_ringing(&self) -> fdo::Result<()> {
        self.asks.send(Ask::StopRinging).map_err(|_| fdo::Error::Failed("stopping".into()))
    }
}

impl Control {
    /// The device's certificate, if it takes shares.
    fn sharing(&self, id: &str) -> fdo::Result<Vec<u8>> {
        let all = self.devices.lock().unwrap();
        match all.get(id) {
            Some(d) if d.linked && plugins::takes(d, SHARE) => Ok(d.cert.clone()),
            _ => Err(fdo::Error::NotSupported(format!("{id} can't be sent anything now"))),
        }
    }

    fn command(&self, c: Command) -> fdo::Result<()> {
        self.commands.send(c).map_err(|_| fdo::Error::Failed("stopping".into()))
    }
}

/// The main loop's own state.
struct Desk {
    devices: Devices,
    commands: mpsc::UnboundedSender<Command>,
    notifier: Notifier,
    session: Option<Session>,
    ringer: Ringer,
    ring_notice: Option<u32>,
    /// Device id to the notification showing its pairing.
    prompts: HashMap<String, u32>,
    battery: Option<system::Battery>,
    locked: Option<bool>,
    me: Arc<Identity>,
    conn: zbus::Connection,
    clipboard: Clipboard,
    /// What goes onto this PC's clipboard, in order.
    paste: mpsc::UnboundedSender<String>,
    received: mpsc::UnboundedSender<Received>,
    /// Device id to the batch of files coming from it.
    batches: HashMap<String, Batch>,
    /// What a notification's buttons open.
    opens: HashMap<u32, Open>,
}

/// A file fetched from a device, or why not.
struct Received {
    id: String,
    name: String,
    count: u64,
    result: Result<PathBuf>,
}

#[derive(Default)]
struct Batch {
    done: u64,
    notice: Option<u32>,
}

enum Open {
    File(PathBuf),
    Folder(PathBuf),
    Copy(String),
}

/// Notifications whose buttons are remembered; older ones are long gone from the screen.
const OPENS_KEPT: usize = 32;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "vyoma_desktop=info,vyoma_core=info".into()))
        .init();

    let dir = std::env::var_os("VYOMA_DIR").map(PathBuf::from).unwrap_or_else(default_dir);
    let me = Arc::new(Identity::load_or_create_in(&dir)?);
    let trust = TrustStore::open(&dir)?;
    let devices: Devices = Arc::default();
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
    let (asks_tx, mut asks) = mpsc::unbounded_channel();
    let notifier = Notifier::new(conn.clone());
    let control = Control { me: me.clone(), notifier: notifier.clone(), commands: commands.clone(), asks: asks_tx, devices: devices.clone(), lan: lan.clone(), targets: targets.clone() };
    conn.object_server().at(OBJECT_PATH, control).await?;
    conn.request_name(BUS_NAME).await.context("another Vyoma desktop app is running")?;
    let (pressed_tx, mut pressed) = mpsc::unbounded_channel();
    {
        let notifier = notifier.clone();
        tokio::spawn(async move {
            if let Err(e) = notifier.presses(pressed_tx).await {
                tracing::warn!("can't hear notification buttons: {e:#}");
            }
        });
    }

    // The screen lock, if logind is there to ask.
    let session = match zbus::Connection::system().await {
        Ok(system) => Session::find(system).await.map_err(|e| tracing::warn!("no logind session, so no lock: {e:#}")).ok(),
        Err(e) => {
            tracing::warn!("no system bus, so no lock: {e:#}");
            None
        }
    };
    let (lock_tx, mut lock_changes) = mpsc::unbounded_channel();
    if let Some(session) = session.clone() {
        tokio::spawn(async move {
            if let Err(e) = session.watch(lock_tx).await {
                tracing::warn!("can't follow the screen lock: {e:#}");
            }
        });
    }

    let (copies_tx, mut copies) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        if let Err(e) = clipboard::watch(copies_tx).await {
            tracing::warn!("can't follow the clipboard: {e:#}");
        }
    });
    let (paste, mut pastes) = mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        while let Some(text) = pastes.recv().await {
            clipboard::set(&text).await;
        }
    });
    let (received, mut arrivals) = mpsc::unbounded_channel();

    let mut desk = Desk {
        me: me.clone(),
        conn: conn.clone(),
        clipboard: Clipboard::default(),
        paste,
        received,
        batches: HashMap::new(),
        opens: HashMap::new(),
        devices,
        commands,
        notifier,
        locked: match &session {
            Some(s) => s.locked().await.ok(),
            None => None,
        },
        session,
        ringer: Ringer::default(),
        ring_notice: None,
        prompts: HashMap::new(),
        battery: system::battery(),
    };

    lan.announce(&targets).await?;
    tracing::info!("{} is up as {}", env!("CARGO_PKG_NAME"), me.device_id);

    let mut battery_tick = tokio::time::interval(BATTERY_EVERY);
    loop {
        tokio::select! {
            Some(event) = events.recv() => desk.on_event(event).await,
            Some(Pressed { notification, action }) = pressed.recv() => desk.on_pressed(notification, &action).await,
            Some(text) = copies.recv() => {
                if let Some(p) = desk.clipboard.changed(text) {
                    tracing::info!("clipboard copied here, {} chars", p["body"]["content"].as_str().unwrap_or("").chars().count());
                    desk.broadcast(CLIPBOARD, p);
                }
            }
            Some(r) = arrivals.recv() => desk.on_received(r).await,
            Some(Ask::StopRinging) = asks.recv() => plugins::stop_ringing(&desk.notifier, &mut desk.ringer, &mut desk.ring_notice).await,
            Some(locked) = lock_changes.recv() => {
                if desk.locked != Some(locked) {
                    desk.locked = Some(locked);
                    desk.broadcast(plugins::LOCK, plugins::lock_packet(locked));
                }
            }
            _ = battery_tick.tick() => {
                let now = system::battery();
                if now != desk.battery {
                    desk.battery = now;
                    if let Some(b) = now {
                        desk.broadcast(plugins::BATTERY, plugins::battery_packet(b));
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    for n in desk.prompts.values() {
        desk.notifier.close(*n).await;
    }
    plugins::stop_ringing(&desk.notifier, &mut desk.ringer, &mut desk.ring_notice).await;
    Ok(())
}

impl Desk {
    fn send(&self, id: &str, packet: Value) {
        let _ = self.commands.send(Command::Send(id.to_owned(), packet));
    }

    /// [packet] to every linked, paired device that takes [kind].
    fn broadcast(&self, kind: &str, packet: Value) {
        let ids: Vec<String> = self.devices.lock().unwrap().iter().filter(|(_, d)| d.linked && plugins::takes(d, kind)).map(|(id, _)| id.clone()).collect();
        for id in ids {
            self.send(&id, packet.clone());
        }
    }

    /// The device is linked and paired: send it what it should know at once.
    fn start(&self, id: &str) {
        let Some(device) = self.devices.lock().unwrap().get(id).cloned() else { return };
        for p in plugins::started(&device, self.battery, self.locked) {
            self.send(id, p);
        }
        if let Some(p) = self.clipboard.connect_packet().filter(|_| plugins::takes(&device, CLIPBOARD_CONNECT)) {
            self.send(id, p);
        }
    }

    /// Shows a notification whose buttons [open] something.
    async fn show_open(&mut self, notice: Notice<'_>, open: Open) -> Option<u32> {
        let n = self.notifier.show(notice).await.map_err(|e| tracing::warn!("couldn't show a notification: {e:#}")).ok()?;
        self.opens.insert(n, open);
        while self.opens.len() > OPENS_KEPT {
            let oldest = *self.opens.keys().min().unwrap();
            self.opens.remove(&oldest);
        }
        Some(n)
    }

    /// A share from a device.
    async fn on_share(&mut self, id: String, device: DeviceView, packet: Value, from: IpAddr) {
        match share::classify(&packet) {
            Shared::File { name, count } => {
                let (me, received) = (self.me.clone(), self.received.clone());
                tokio::spawn(async move {
                    let result = share::receive(&me, &device.cert, from, &packet, &share::downloads()).await;
                    let _ = received.send(Received { id, name, count, result });
                });
            }
            Shared::Text(text) => {
                let title = format!("Text from {}", device.name);
                let shown: String = text.chars().take(400).collect();
                let notice = Notice { icon: "edit-paste", title: &title, body: &shown, actions: &[("copy", "Copy")], sticky: false, replaces: None };
                self.show_open(notice, Open::Copy(text)).await;
            }
            Shared::Url(url) => share::open(&url),
            Shared::Nothing => {}
        }
    }

    async fn on_received(&mut self, r: Received) {
        let device = self.devices.lock().unwrap().get(&r.id).map(|d| d.name.clone()).unwrap_or_default();
        let path = match r.result {
            Ok(path) => path,
            Err(e) => {
                tracing::warn!("{}: {} not received: {e:#}", r.id, r.name);
                plugins::tell(&self.notifier, &format!("Couldn't receive {}", r.name), &format!("{e:#}")).await;
                return;
            }
        };
        tracing::info!("{}: received {}", r.id, path.display());
        let batch = self.batches.entry(r.id.clone()).or_default();
        batch.done += 1;
        let (done, replaces) = (batch.done, batch.notice);
        if done >= r.count {
            self.batches.remove(&r.id);
        }
        let n = if r.count <= 1 {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let body = format!("{name} from {device}");
            let notice = Notice { icon: "document-save", title: "Received", body: &body, actions: &[("open", "Open"), ("folder", "Show in folder")], sticky: false, replaces };
            self.show_open(notice, Open::File(path)).await
        } else {
            let title = if done >= r.count { format!("Received {} files", r.count) } else { format!("Received {done} of {} files", r.count) };
            let body = format!("from {device}");
            let notice = Notice { icon: "document-save", title: &title, body: &body, actions: &[("folder", "Show in folder")], sticky: false, replaces };
            self.show_open(notice, Open::Folder(path)).await
        };
        if let Some(b) = self.batches.get_mut(&r.id) {
            b.notice = n;
        }
    }

    async fn on_pressed(&mut self, notification: u32, action: &str) {
        if self.ring_notice == Some(notification) && action == "found" {
            plugins::stop_ringing(&self.notifier, &mut self.ringer, &mut self.ring_notice).await;
            return;
        }
        if let Some(open) = self.opens.get(&notification) {
            match (open, action) {
                (Open::File(p), "open") => share::open(&p.to_string_lossy()),
                (Open::File(p) | Open::Folder(p), "folder") => share::show_in_folder(&self.conn, p).await,
                (Open::Copy(text), "copy") => {
                    let _ = self.paste.send(text.clone());
                }
                _ => {}
            }
            return;
        }
        let Some(id) = self.prompts.iter().find(|(_, n)| **n == notification).map(|(id, _)| id.clone()) else { return };
        let _ = self.commands.send(match action {
            "accept" => Command::Accept(id),
            "reject" => Command::Reject(id),
            _ => return,
        });
    }

    async fn on_event(&mut self, event: Event) {
        match event {
            Event::Linked { id, name, kind, paired, incoming, cert, .. } => {
                {
                    let mut all = self.devices.lock().unwrap();
                    let d = all.entry(id.clone()).or_default();
                    (d.name, d.kind, d.paired, d.linked, d.incoming, d.cert) = (name, kind, paired, true, incoming, cert);
                }
                if paired {
                    self.start(&id);
                }
            }
            Event::Gone { id } => {
                let mut all = self.devices.lock().unwrap();
                // Unpaired devices are only listed while they're here.
                if all.get(&id).is_some_and(|d| d.paired) {
                    let d = all.get_mut(&id).unwrap();
                    (d.linked, d.battery, d.signal) = (false, None, None);
                } else {
                    all.remove(&id);
                }
            }
            Event::Prompt { id, name, key } => {
                self.set_key(&id, &key);
                let body = format!("Code {key}. Accept only if {name} shows the same code.");
                let notice = Notice { icon: "smartphone", title: &format!("Pair with {name}?"), body: &body, actions: &[("accept", "Accept"), ("reject", "Reject")], sticky: true, replaces: self.prompts.get(&id).copied() };
                self.show_prompt(id, notice).await;
            }
            Event::Asked { id, name, key } => {
                self.set_key(&id, &key);
                let body = format!("Code {key}. Accept on {name} if it shows the same code.");
                let notice = Notice { icon: "smartphone", title: &format!("Pairing with {name}"), body: &body, actions: &[], sticky: true, replaces: self.prompts.get(&id).copied() };
                self.show_prompt(id, notice).await;
            }
            Event::PairingOver { id } => {
                self.set_key(&id, "");
                if let Some(n) = self.prompts.remove(&id) {
                    self.notifier.close(n).await;
                }
            }
            Event::Paired { id, name } => {
                if let Some(d) = self.devices.lock().unwrap().get_mut(&id) {
                    (d.paired, d.key) = (true, String::new());
                }
                if let Some(n) = self.prompts.remove(&id) {
                    self.notifier.close(n).await;
                }
                plugins::tell(&self.notifier, &format!("Paired with {name}"), "").await;
                self.start(&id);
            }
            Event::Unpaired { id, name } => {
                {
                    let mut all = self.devices.lock().unwrap();
                    if all.get(&id).is_some_and(|d| d.linked) {
                        let d = all.get_mut(&id).unwrap();
                        (d.paired, d.battery, d.signal) = (false, None, None);
                    } else {
                        all.remove(&id);
                    }
                }
                plugins::tell(&self.notifier, &format!("Unpaired from {name}"), "").await;
            }
            Event::Packet { id, packet, from } => {
                let Some(mut device) = self.devices.lock().unwrap().get(&id).cloned() else { return };
                match packet["type"].as_str().unwrap_or("") {
                    CLIPBOARD | CLIPBOARD_CONNECT => {
                        if let Some(text) = self.clipboard.received(&packet) {
                            tracing::info!("{id}: clipboard, {} chars", text.chars().count());
                            let _ = self.paste.send(text);
                        }
                        return;
                    }
                    SHARE => return self.on_share(id, device, packet, from).await,
                    // Each file says how many there are too.
                    SHARE_UPDATE => return,
                    _ => {}
                }
                let cx = plugins::Context { notifier: &self.notifier, session: self.session.as_ref(), ringer: &mut self.ringer, ring_notice: &mut self.ring_notice };
                let replies = plugins::on_packet(cx, &mut device, &packet).await;
                // Only what the plugin keeps; the rest may have changed meanwhile.
                if let Some(d) = self.devices.lock().unwrap().get_mut(&id) {
                    (d.battery, d.signal) = (device.battery, device.signal);
                }
                for p in replies {
                    self.send(&id, p);
                }
            }
        }
    }

    fn set_key(&self, id: &str, key: &str) {
        if let Some(d) = self.devices.lock().unwrap().get_mut(id) {
            d.key = key.to_owned();
        }
    }

    async fn show_prompt(&mut self, id: String, notice: Notice<'_>) {
        match self.notifier.show(notice).await {
            Ok(n) => {
                self.prompts.insert(id, n);
            }
            Err(e) => tracing::warn!("couldn't show the pairing prompt: {e:#}"),
        }
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
            "deviceType": if system::battery().is_some() { "laptop" } else { "desktop" },
            "protocolVersion": 8,
            "incomingCapabilities": plugins::INCOMING,
            "outgoingCapabilities": plugins::OUTGOING,
        }),
    )
}

fn host_name() -> String {
    let name = std::fs::read_to_string("/proc/sys/kernel/hostname").unwrap_or_default();
    vyoma_core::lan::device_name(name.trim())
}
