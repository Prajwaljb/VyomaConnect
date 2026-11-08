//! Vyoma's desktop app, standing alone: finds phones, pairs with them and talks to them with no
//! kdeconnectd. This process is the daemon; the GUI will be another one, talking to it over the
//! `dev.vyoma.Desktop1` D-Bus interface below, so closing it never drops a link.
//!
//! `--import-kde` takes over kdeconnectd's identity and pairings, so phones paired with this PC stay
//! paired; `--check` and `--setup` report and install what its features run (the companion's setup).
//!
//! Environment, for development beside the user's own KDE Connect:
//! - `VYOMA_DIR`: where its identity and pairings live (default `~/.config/vyoma-desktop`).
//! - `VYOMA_ALLOW`: device ids, comma-separated; when set, every other device is ignored.
//! - `VYOMA_ANNOUNCE`: addresses to announce to instead of the broadcast address.
//! - `VYOMA_PLAYERS`: MPRIS bus names (after `org.mpris.MediaPlayer2.`), comma-separated; when set,
//!   only those players are shown, so tests never reach the user's own.

mod clipboard;
mod commands;
mod input;
mod media;
mod notifications;
mod notify;
mod plugins;
mod phonefiles;
mod places;
mod presenter;
mod share;
mod system;
mod telephony;
mod volume;

use std::collections::{HashMap, HashSet};
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
use vyoma_companion::session::{self as features, Host, Phone};
use zbus::fdo;

use clipboard::{CLIPBOARD, CLIPBOARD_CONNECT, Clipboard};
use notifications::{Incoming, Note};
use notify::{Heard, Notice, Notifier};
use share::{SHARE, SHARE_UPDATE, Shared};
use system::{Ringer, Session};

const BUS_NAME: &str = "dev.vyoma.Desktop";
const OBJECT_PATH: &str = "/dev/vyoma/Desktop";
/// How often phones tethered to this PC (or whose hotspot it's on) are told it's here, while unlinked.
const TETHER_EVERY: Duration = Duration::from_secs(3);
/// How often the PC's battery is read; it's sent on when it changed.
const BATTERY_EVERY: Duration = Duration::from_secs(30);
/// How often kept phone mounts are checked, and how long a remount's ask is waited on.
/// How often what the GUI shows is checked for changes.
const CHANGES_EVERY: Duration = Duration::from_millis(400);
const MOUNTS_EVERY: Duration = Duration::from_secs(5);
const REMOUNT_WAIT: Duration = Duration::from_secs(30);

/// What the GUI is shown of one device, and what its plugins keep.
#[derive(Clone, Default)]
pub struct DeviceView {
    pub name: String,
    pub kind: String,
    pub paired: bool,
    pub linked: bool,
    /// The code while a pairing is on screen.
    pub key: String,
    /// That pairing is the device's ask (ours to accept), not ours.
    pub asking: bool,
    /// The packet types it takes.
    pub incoming: Vec<String>,
    /// Charge, charging, low.
    pub battery: Option<(i64, bool, bool)>,
    /// Each SIM's network and level (0..4).
    pub signal: Option<Vec<Value>>,
    /// The certificate its links show, which its payloads must show too.
    pub cert: Vec<u8>,
    /// Its best link's address ([best]): Wi-Fi over the cable over Bluetooth.
    pub remote: Option<IpAddr>,
    /// Every link's address.
    pub routes: Vec<IpAddr>,
}

type Devices = Arc<Mutex<HashMap<String, DeviceView>>>;

/// What the D-Bus side asks of the main loop beyond the hub's commands.
enum Ask {
    StopRinging,
    /// Opens the phone's files, mounting them first.
    Browse(String),
}

struct Control {
    me: Arc<Identity>,
    run: commands::Commands,
    notifier: Notifier,
    commands: mpsc::UnboundedSender<Command>,
    asks: mpsc::UnboundedSender<Ask>,
    devices: Devices,
    lan: Arc<Lan>,
    targets: Vec<SocketAddr>,
    /// For the signals sent from tasks.
    conn: zbus::Connection,
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

    /// One device's state as JSON: battery, signal, how it's linked (`via`: wifi, cable or
    /// bluetooth), what it can do here (`can`) and the rest of [devices].
    fn details(&self, id: String) -> fdo::Result<String> {
        let all = self.devices.lock().unwrap();
        let d = all.get(&id).ok_or_else(|| fdo::Error::UnknownObject(id.clone()))?;
        let battery = d.battery.map(|(charge, charging, low)| json!({ "charge": charge, "charging": charging, "low": low }));
        // Files need a network or the cable: Bluetooth carries packets only.
        let moves = via(d) != Some("bluetooth");
        let can = json!({
            "ring": plugins::takes(d, plugins::FIND),
            "browse": moves && plugins::takes(d, phonefiles::SFTP_REQUEST),
            "share": moves && plugins::takes(d, share::SHARE),
        });
        Ok(json!({ "name": d.name, "type": d.kind, "paired": d.paired, "linked": d.linked, "key": d.key, "asking": d.asking, "battery": battery, "signal": d.signal, "via": via(d), "can": can }).to_string())
    }

    /// Something the GUI shows changed: a device came, went, paired, or its battery moved.
    #[zbus(signal)]
    async fn changed(emitter: &zbus::object_server::SignalEmitter<'_>) -> zbus::Result<()>;

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
        let (me, commands, notifier, conn) = (self.me.clone(), self.commands.clone(), self.notifier.clone(), self.conn.clone());
        let count = files.len() as u32;
        tokio::spawn(async move {
            let error = match share::send_files(&commands, &id, &me, &pinned, files).await {
                Ok(()) => String::new(),
                Err(e) => {
                    tracing::warn!("{id}: {e:#}");
                    plugins::tell(&notifier, "Couldn't send", &format!("{e:#}")).await;
                    format!("{e:#}")
                }
            };
            if let Ok(emitter) = zbus::object_server::SignalEmitter::new(&conn, OBJECT_PATH) {
                let _ = Control::files_sent(&emitter, &id, count, &error).await;
            }
        });
        Ok(())
    }

    /// A [send_files] batch is over: [error] is empty when the device fetched all [count].
    #[zbus(signal)]
    async fn files_sent(emitter: &zbus::object_server::SignalEmitter<'_>, id: &str, count: u32, error: &str) -> zbus::Result<()>;

    /// Sends text to the device, or opens a link there.
    fn send_text(&self, id: String, text: String) -> fdo::Result<()> {
        self.sharing(&id)?;
        self.command(Command::Send(id, share::text_packet(&text)))
    }

    /// Adds a command devices can run here; its key.
    fn add_command(&self, name: String, command: String) -> fdo::Result<String> {
        let key = self.run.add(&name, &command).map_err(|e| fdo::Error::Failed(format!("{e:#}")))?;
        self.commands_changed();
        Ok(key)
    }

    fn remove_command(&self, key: String) -> fdo::Result<()> {
        match self.run.remove(&key) {
            Ok(true) => {
                self.commands_changed();
                Ok(())
            }
            Ok(false) => Err(fdo::Error::UnknownObject(key)),
            Err(e) => Err(fdo::Error::Failed(format!("{e:#}"))),
        }
    }

    /// The commands as JSON: {key: {name, command}}.
    fn commands(&self) -> String {
        Value::Object(self.run.all().into_iter().collect()).to_string()
    }

    /// Stops this PC ringing for a device that's looking for it.
    fn stop_ringing(&self) -> fdo::Result<()> {
        self.asks.send(Ask::StopRinging).map_err(|_| fdo::Error::Failed("stopping".into()))
    }

    /// Opens the device's files in the file manager (it has to allow it: Vyoma's "Browse this
    /// phone from your PC"). Mounted under $XDG_RUNTIME_DIR/vyoma until it goes.
    fn browse(&self, id: String) -> fdo::Result<()> {
        let takes = self.devices.lock().unwrap().get(&id).is_some_and(|d| d.linked && plugins::takes(d, phonefiles::SFTP_REQUEST));
        if !takes {
            return Err(fdo::Error::NotSupported(format!("{id} can't be browsed now")));
        }
        self.asks.send(Ask::Browse(id)).map_err(|_| fdo::Error::Failed("stopping".into()))
    }
}

impl Control {
    /// Every linked device that takes the list gets it again.
    fn commands_changed(&self) {
        let ids: Vec<String> = self.devices.lock().unwrap().iter().filter(|(_, d)| d.linked && plugins::takes(d, commands::RUNCOMMAND)).map(|(id, _)| id.clone()).collect();
        for id in ids {
            let _ = self.command(Command::Send(id, self.run.list_packet()));
        }
    }

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
    /// Device id to its phone call, if one's on or was missed.
    calls: HashMap<String, telephony::Line>,
    /// The presenter's dot over the screen.
    presenter: presenter::Presenter,
    /// Device id to its files, mounted.
    mounts: HashMap<String, phonefiles::Mount>,
    /// Phones whose files were opened and not unmounted here: remounted, quietly, when the link
    /// changes route or sshfs is cut off.
    kept: HashSet<String>,
    /// Remounts asked for (not to open a window for), and when.
    remounting: HashMap<String, std::time::Instant>,
    /// Finished mounts, from the tasks that make them.
    mounted: mpsc::UnboundedSender<(String, Result<phonefiles::Mount>)>,
    /// Where our identity and its key are.
    dir: PathBuf,
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
    run: commands::Commands,
    /// The phones' notifications shown here.
    mirror: notifications::Mirror,
    /// Phone notifications whose icon has just been fetched.
    notes: mpsc::UnboundedSender<(String, Note)>,
    /// Media requests from devices, for the players' task.
    media: mpsc::UnboundedSender<(String, Value)>,
    /// Touchpad and keyboard requests, for the input task.
    input: mpsc::UnboundedSender<(std::time::Instant, String, Value)>,
    /// Volume requests, for the outputs' task.
    volume: mpsc::UnboundedSender<(String, Value)>,
    /// What the companion did beside kdeconnectd (screen, files, controller…), served here.
    host: Arc<Host>,
    /// Device id to its running features session, which takes its vyoma.* packets.
    sessions: HashMap<String, mpsc::UnboundedSender<(Value, IpAddr)>>,
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

fn main() -> Result<()> {
    vyoma_companion::setup::standalone();
    let dir = std::env::var_os("VYOMA_DIR").map(PathBuf::from).unwrap_or_else(default_dir);
    match std::env::args().nth(1).as_deref() {
        // The phone's screen window runs as a child process (the companion's mirror.rs).
        Some(vyoma_companion::mirror::WINDOW_ARG) => vyoma_companion::mirror::window(&std::env::args().skip(2).collect::<Vec<_>>()),
        Some("--check") => std::process::exit(if vyoma_companion::setup::print_check() { 0 } else { 1 }),
        Some("--setup") => vyoma_companion::setup::run(std::env::args().nth(2).is_some_and(|a| a == "--dry-run")),
        Some("--import-kde") => import_kde(&dir),
        Some(phonefiles::RELAY_ARG) => phonefiles::relay(&std::env::args().nth(2).unwrap_or_default()),
        // Run by ssh with its prompt as the argument.
        Some(_) if std::env::var_os(phonefiles::ASKPASS_ENV).is_some() => phonefiles::askpass(),
        Some(other) => anyhow::bail!("unknown option {other}: try --import-kde, --check or --setup [--dry-run]"),
        None => daemon(dir),
    }
}

/// Becomes the PC kdeconnectd was: its id, certificate and pairings. Only into an empty folder, so
/// pairings made here are never lost.
fn import_kde(dir: &std::path::Path) -> Result<()> {
    if dir.join("device_id").exists() {
        anyhow::bail!("{} already has an identity: move it away first", dir.display());
    }
    let kde = vyoma_companion::kde::KdeHost::load()?;
    kde.copy_identity(dir)?;
    let mut trust = TrustStore::open(dir)?;
    let devices = kde.trusted();
    for d in &devices {
        trust.put(&d.id, vyoma_core::trust::Trusted { name: d.name.clone(), kind: d.kind.clone(), cert: d.cert.clone() })?;
    }
    println!("Now {} with its {} pairing(s). Stop kdeconnectd before starting vyoma-desktop.", kde.name, devices.len());
    Ok(())
}

#[tokio::main]
async fn daemon(dir: PathBuf) -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "vyoma_desktop=info,vyoma_core=info,vyoma_companion=info".into()))
        .init();

    // The companion's features keep their tokens and caches here too.
    vyoma_core::identity::use_dir(dir.clone());
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
    let beside_kde = kde_running(&conn).await;
    if beside_kde && vyoma_companion::kde::KdeHost::load().is_ok_and(|k| k.device_id == me.device_id) {
        anyhow::bail!("kdeconnectd is running as this same PC: stop it first");
    }
    let (asks_tx, mut asks) = mpsc::unbounded_channel();
    let (mounted_tx, mut mounts_done) = mpsc::unbounded_channel();
    let notifier = Notifier::new(conn.clone());
    let run = commands::Commands::new(&dir);
    let control = Control { me: me.clone(), run: run.clone(), notifier: notifier.clone(), commands: commands.clone(), asks: asks_tx, devices: devices.clone(), lan: lan.clone(), targets: targets.clone(), conn: conn.clone() };
    conn.object_server().at(OBJECT_PATH, control).await?;
    conn.request_name(BUS_NAME).await.context("another Vyoma desktop app is running")?;
    // Tells the GUI when what it shows changed (checked, as many places change devices).
    {
        let (conn, devices) = (conn.clone(), devices.clone());
        tokio::spawn(async move {
            let mut last = shown(&devices);
            loop {
                tokio::time::sleep(CHANGES_EVERY).await;
                let now = shown(&devices);
                if now != last {
                    last = now;
                    if let Ok(emitter) = zbus::object_server::SignalEmitter::new(&conn, OBJECT_PATH) {
                        let _ = Control::changed(&emitter).await;
                    }
                }
            }
        });
    }
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
    let (notes, mut iconed) = mpsc::unbounded_channel();
    let (input, input_requests) = mpsc::unbounded_channel();
    let (echo_tx, mut echoes) = mpsc::unbounded_channel::<(String, Value)>();
    tokio::spawn(input::run(dir.clone(), input_requests, echo_tx));
    let (volume, volume_requests) = mpsc::unbounded_channel();
    let (volume_tx, mut volume_out) = mpsc::unbounded_channel::<(Option<String>, Value)>();
    tokio::spawn(async move {
        if let Err(e) = volume::run(volume_requests, volume_tx).await {
            tracing::warn!("no volume control: {e:#}");
        }
    });
    let (media, media_requests) = mpsc::unbounded_channel();
    let (media_tx, mut media_out) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        if let Err(e) = media::run(media_requests, media_tx).await {
            tracing::warn!("no media players: {e:#}");
        }
    });

    let player = match vyoma_companion::media::Media::start().await {
        Ok(m) => Some(m),
        Err(e) => {
            tracing::warn!("no handoff or walk-away pause: {e:#}");
            None
        }
    };
    let host = Arc::new(Host { me: me.clone(), media: player, away: Arc::default() });

    let mut desk = Desk {
        host,
        sessions: HashMap::new(),
        me: me.clone(),
        conn: conn.clone(),
        clipboard: Clipboard::default(),
        paste,
        received,
        batches: HashMap::new(),
        opens: HashMap::new(),
        mirror: notifications::Mirror::default(),
        input,
        notes,
        run,
        media,
        volume,
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
        calls: HashMap::new(),
        presenter: presenter::Presenter::default(),
        mounts: HashMap::new(),
        kept: HashSet::new(),
        remounting: HashMap::new(),
        mounted: mounted_tx,
        dir: dir.clone(),
        prompts: HashMap::new(),
        battery: system::battery(),
    };

    lan.announce(&targets).await?;
    tracing::info!("{} is up as {}", env!("CARGO_PKG_NAME"), me.device_id);
    phonefiles::tidy().await;
    if beside_kde {
        tracing::info!("kdeconnectd is running: the cable, tethering and Bluetooth stay its own");
    } else {
        transports(lan.clone(), desk.devices.clone());
    }

    let mut battery_tick = tokio::time::interval(BATTERY_EVERY);
    let mut mounts_tick = tokio::time::interval(MOUNTS_EVERY);
    loop {
        tokio::select! {
            Some(event) = events.recv() => desk.on_event(event).await,
            Some(h) = pressed.recv() => desk.on_heard(h).await,
            Some((id, note)) = iconed.recv() => desk.show_note(&id, note).await,
            Some((id, p)) = echoes.recv() => desk.send(&id, p),
            Some(text) = copies.recv() => {
                if let Some(p) = desk.clipboard.changed(text) {
                    tracing::info!("clipboard copied here, {} chars", p["body"]["content"].as_str().unwrap_or("").chars().count());
                    desk.broadcast(CLIPBOARD, p);
                }
            }
            Some(r) = arrivals.recv() => desk.on_received(r).await,
            Some(o) = media_out.recv() => desk.on_media(o),
            Some((to, p)) = volume_out.recv() => match to {
                Some(id) => desk.send(&id, p),
                None => desk.broadcast(volume::VOLUME, p),
            },
            Some(ask) = asks.recv() => match ask {
                Ask::StopRinging => plugins::stop_ringing(&desk.notifier, &mut desk.ringer, &mut desk.ring_notice).await,
                Ask::Browse(id) => desk.browse(id),
            },
            Some((id, result)) = mounts_done.recv() => desk.on_mounted(id, result).await,
            _ = mounts_tick.tick() => desk.check_mounts().await,
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
    for (_, m) in desk.mounts.drain() {
        m.unmount().await;
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
    fn start(&mut self, id: &str) {
        let Some(device) = self.devices.lock().unwrap().get(id).cloned() else { return };
        for p in plugins::started(&device, self.battery, self.locked) {
            self.send(id, p);
        }
        if let Some(p) = self.clipboard.connect_packet().filter(|_| plugins::takes(&device, CLIPBOARD_CONNECT)) {
            self.send(id, p);
        }
        if plugins::takes(&device, input::MOUSEPAD_KEYBOARDSTATE) {
            self.send(id, input::keyboard_state());
        }
        self.start_features(id, &device);
    }

    /// Serves the device's vyoma.* packets, if it sends any, until it's gone or unpaired.
    fn start_features(&mut self, id: &str, device: &DeviceView) {
        if self.sessions.contains_key(id) || !device.incoming.iter().any(|t| features::OUTGOING.contains(&t.as_str())) {
            return;
        }
        let (packets, rx) = mpsc::unbounded_channel();
        let (replies, mut out) = mpsc::unbounded_channel();
        let phone = Phone { id: id.to_owned(), name: features::phone_name(&device.name), pinned: device.cert.clone() };
        let (host, commands, id) = (self.host.clone(), self.commands.clone(), id.to_owned());
        self.sessions.insert(id.clone(), packets);
        tokio::spawn(async move {
            let to = id.clone();
            tokio::spawn(async move {
                while let Some(p) = out.recv().await {
                    let _ = commands.send(Command::Send(to.clone(), p));
                }
            });
            host.away.linked(&id, host.media.as_ref()).await;
            features::serve(host.clone(), phone, vyoma_core::link::Sender::queue(replies), rx).await;
            host.away.unlinked(&id, host.media.clone());
        });
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

    fn on_media(&self, o: media::Out) {
        match o {
            media::Out::All(p) => self.broadcast(media::MPRIS, p),
            media::Out::To(id, p) => self.send(&id, p),
            media::Out::Art { id, packet, path, size } => {
                let Some(pinned) = self.devices.lock().unwrap().get(&id).map(|d| d.cert.clone()) else { return };
                let (me, commands) = (self.me.clone(), self.commands.clone());
                tokio::spawn(async move {
                    match share::offer_file(&commands, &id, &me, &pinned, packet, &path, size).await {
                        Ok(()) => tracing::info!("{id}: album art sent, {size} bytes"),
                        Err(e) => tracing::info!("{id}: album art not sent: {e:#}"),
                    }
                });
            }
        }
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

    async fn on_heard(&mut self, h: Heard) {
        if let Some((id, p)) = self.mirror.heard(&h) {
            self.send(&id, p);
        } else if let Heard::Pressed { notification, action } = h {
            self.on_pressed(notification, &action).await;
        }
    }

    /// A notification from a phone: shown once its icon is here, if it brings one we haven't.
    fn on_note(&mut self, id: String, device: &DeviceView, packet: Value, from: IpAddr) -> Option<Note> {
        let note = match notifications::parse(&packet) {
            Incoming::Show(note) => note,
            Incoming::Cancel(phone_id) => {
                if let Some(n) = self.mirror.cancelled(&id, &phone_id) {
                    let notifier = self.notifier.clone();
                    tokio::spawn(async move { notifier.close(n).await });
                }
                return None;
            }
            Incoming::Ignore => return None,
        };
        let path = note.icon.as_deref().map(notifications::icon_path);
        let Some(path) = path.filter(|p| !p.exists() && packet["payloadSize"].as_u64().is_some()) else { return Some(note) };
        let (me, pinned, notes) = (self.me.clone(), device.cert.clone(), self.notes.clone());
        tokio::spawn(async move {
            if let Err(e) = tokio::time::timeout(Duration::from_secs(5), notifications::fetch_icon(&me, &pinned, from, &packet, &path)).await.unwrap_or_else(|_| Err(anyhow::anyhow!("timed out"))) {
                tracing::debug!("{id}: icon not fetched: {e:#}");
            }
            let _ = notes.send((id, note));
        });
        None
    }

    async fn show_note(&mut self, id: &str, note: Note) {
        let heading = note.heading();
        let buttons = note.buttons();
        let actions: Vec<(&str, &str)> = buttons.iter().map(|(k, l)| (k.as_str(), l.as_str())).collect();
        let image = note.icon.as_deref().map(notifications::icon_path).filter(|p| p.exists());
        let notice = Notice { icon: "smartphone", title: &heading, body: &note.text, actions: &actions, sticky: false, replaces: self.mirror.showing(id, &note.id) };
        let reply = note.replies().then(|| format!("Reply to {}", if note.title.is_empty() { &note.app } else { &note.title }));
        match self.notifier.show_with(notice, image.as_deref(), reply.as_deref()).await {
            Ok(n) => self.mirror.shown(id, note, n),
            Err(e) => tracing::warn!("couldn't show a notification from {id}: {e:#}"),
        }
    }

    async fn on_pressed(&mut self, notification: u32, action: &str) {
        if self.ring_notice == Some(notification) && action == "found" {
            plugins::stop_ringing(&self.notifier, &mut self.ringer, &mut self.ring_notice).await;
            return;
        }
        if action == "mute"
            && let Some(id) = telephony::muting(&self.calls, notification)
        {
            tracing::info!("{id}: muting the ringer");
            self.send(&id, packet(telephony::MUTE, json!({})));
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
            Event::Linked { id, name, kind, paired, incoming, cert, remote, .. } => {
                {
                    let mut all = self.devices.lock().unwrap();
                    let d = all.entry(id.clone()).or_default();
                    (d.name, d.kind, d.paired, d.linked, d.incoming, d.cert, d.routes, d.remote) = (name, kind, paired, true, incoming, cert, vec![remote], Some(remote));
                }
                if paired {
                    self.start(&id);
                    self.route_changed(&id, remote);
                }
            }
            Event::Routes { id, remotes } => {
                let best = {
                    let mut all = self.devices.lock().unwrap();
                    let Some(d) = all.get_mut(&id) else { return };
                    d.remote = best(&remotes, !vyoma_companion::usb::cable_ports().is_empty());
                    d.routes = remotes;
                    d.paired.then_some(d.remote).flatten()
                };
                if let Some(best) = best {
                    self.route_changed(&id, best);
                }
            }
            Event::Gone { id } => {
                // Dropping its packets' channel ends the session, and the screen, camera and controller with it.
                self.sessions.remove(&id);
                // Its mount stays: sshfs has a connection of its own, which a blip of the link (the
                // cable pulled with Wi-Fi to take over) needn't end, and remounting at the same path
                // left an open file manager window looking at the old, empty one. A phone that's
                // really gone ends sshfs within ~10 s, and check_mounts remounts it once it's back.
                if let Some(mut line) = self.calls.remove(&id) {
                    let steps = line.gone();
                    self.call_steps(&id, "", &mut line, steps, None).await;
                }
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
                self.set_key(&id, &key, true);
                let body = format!("Code {key}. Accept only if {name} shows the same code.");
                let notice = Notice { icon: "smartphone", title: &format!("Pair with {name}?"), body: &body, actions: &[("accept", "Accept"), ("reject", "Reject")], sticky: true, replaces: self.prompts.get(&id).copied() };
                self.show_prompt(id, notice).await;
            }
            Event::Asked { id, name, key } => {
                self.set_key(&id, &key, false);
                let body = format!("Code {key}. Accept on {name} if it shows the same code.");
                let notice = Notice { icon: "smartphone", title: &format!("Pairing with {name}"), body: &body, actions: &[], sticky: true, replaces: self.prompts.get(&id).copied() };
                self.show_prompt(id, notice).await;
            }
            Event::PairingOver { id } => {
                self.set_key(&id, "", false);
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
                self.sessions.remove(&id);
                self.kept.remove(&id);
                if let Some(m) = self.mounts.remove(&id) {
                    m.unmount().await;
                }
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
                if packet["type"].as_str().is_some_and(|t| t.starts_with("vyoma.")) {
                    if let Some(session) = self.sessions.get(&id) {
                        let _ = session.send((packet, from));
                    }
                    return;
                }
                match packet["type"].as_str().unwrap_or("") {
                    CLIPBOARD | CLIPBOARD_CONNECT => {
                        if let Some(text) = self.clipboard.received(&packet) {
                            tracing::info!("{id}: clipboard, {} chars", text.chars().count());
                            let _ = self.paste.send(text);
                        }
                        return;
                    }
                    SHARE => return self.on_share(id, device, packet, from).await,
                    notifications::NOTIFICATION => {
                        if let Some(note) = self.on_note(id.clone(), &device, packet, from) {
                            self.show_note(&id, note).await;
                        }
                        return;
                    }
                    media::MPRIS_REQUEST => {
                        let _ = self.media.send((id, packet));
                        return;
                    }
                    input::MOUSEPAD_REQUEST => {
                        let _ = self.input.send((std::time::Instant::now(), id, packet));
                        return;
                    }
                    commands::RUNCOMMAND_REQUEST => {
                        let body = &packet["body"];
                        if body["requestCommandList"].as_bool() == Some(true) {
                            self.send(&id, self.run.list_packet());
                        }
                        if let Some(key) = body["key"].as_str() {
                            match self.run.run(key) {
                                Some(name) => tracing::info!("{id}: ran {name}"),
                                None => tracing::info!("{id}: no command {key}"),
                            }
                        }
                        return;
                    }
                    volume::VOLUME_REQUEST => {
                        let _ = self.volume.send((id, packet));
                        return;
                    }
                    phonefiles::SFTP => {
                        match phonefiles::parse(&packet) {
                            phonefiles::Reply::Ready(server) => self.mount(id, &device, server),
                            phonefiles::Reply::Failed(why) => {
                                tracing::info!("{id}: files not browsable: {why}");
                                self.kept.remove(&id);
                                self.remounting.remove(&id);
                                plugins::tell(&self.notifier, &format!("Can't open {}'s files", device.name), &why).await;
                            }
                            phonefiles::Reply::Nothing => {}
                        }
                        return;
                    }
                    presenter::PRESENTER => {
                        self.presenter.on_packet(&packet["body"]);
                        return;
                    }
                    telephony::TELEPHONY => {
                        if let Some(call) = telephony::parse(&packet) {
                            let mut line = self.calls.remove(&id).unwrap_or_default();
                            let steps = line.on_call(&call);
                            tracing::info!("{id}: call {:?}{}", call.kind, if call.over { ", over" } else { "" });
                            self.call_steps(&id, &device.name, &mut line, steps, Some(&call)).await;
                            self.calls.insert(id, line);
                        }
                        return;
                    }
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

    /// Opens [id]'s files: straight away if they're mounted, else asks the phone to serve them.
    fn browse(&mut self, id: String) {
        if let Some(m) = self.mounts.get_mut(&id) {
            if m.alive() {
                share::open(&m.path.to_string_lossy());
                return;
            }
            if let Some(m) = self.mounts.remove(&id) {
                tokio::spawn(m.unmount());
            }
        }
        tracing::info!("{id}: asking to browse its files");
        self.send(&id, phonefiles::request());
    }

    /// The phone's server is up: mount it, off the main loop.
    fn mount(&mut self, id: String, device: &DeviceView, server: phonefiles::Server) {
        let Some(remote) = device.remote else { return };
        if self.mounts.get_mut(&id).is_some_and(|m| m.alive()) {
            return;
        }
        // The ended one it replaces goes first, its folder with it.
        let old = self.mounts.remove(&id);
        let at = phonefiles::mount_point(&device.name);
        let (key, cert, dir, done) = (self.dir.join("privateKey.pem"), device.cert.clone(), self.dir.clone(), self.mounted.clone());
        tokio::spawn(async move {
            if let Some(old) = old {
                old.unmount().await;
            }
            let result = phonefiles::mount(&server, remote, &id, &at, &key, &cert, &dir).await;
            let _ = done.send((id, result));
        });
    }

    /// [id] is linked again or its best route changed: a kept mount that ended remounts. One that
    /// still works stays on its route, however good the new one: swapping it under an open file
    /// manager window leaves that window empty.
    fn route_changed(&mut self, id: &str, _remote: IpAddr) {
        if self.kept.contains(id) && !self.mounts.get_mut(id).is_some_and(|m| m.alive()) {
            self.remount(id);
        }
    }

    /// Asks [id] to serve its files again, for a mount that's kept (no window opens for it).
    fn remount(&mut self, id: &str) {
        if self.remounting.get(id).is_some_and(|at| at.elapsed() < REMOUNT_WAIT) {
            return;
        }
        tracing::info!("{id}: remounting its files");
        self.remounting.insert(id.to_owned(), std::time::Instant::now());
        self.send(id, phonefiles::request());
    }

    /// Kept mounts whose sshfs ended: cleanly, it was unmounted (the file manager's eject) and is
    /// let go; failing, it was cut off from the phone and is remounted once the phone's linked.
    async fn check_mounts(&mut self) {
        if self.kept.is_empty() && self.mounts.is_empty() {
            return;
        }
        let ended: Vec<(String, bool)> = self.mounts.iter_mut().filter_map(|(id, m)| Some((id.clone(), m.ended()?))).collect();
        for (id, ejected) in ended {
            let Some(mut m) = self.mounts.remove(&id) else { continue };
            m.release().await;
            if ejected {
                tracing::info!("{id}: files unmounted");
                self.kept.remove(&id);
            } else {
                tracing::info!("{id}: lost the connection to its files: {}", m.why().await);
            }
        }
        let all = self.devices.lock().unwrap().clone();
        let linked: Vec<String> = {
            self.kept.iter().filter(|id| !self.mounts.contains_key(*id) && all.get(*id).is_some_and(|d| d.linked)).cloned().collect()
        };
        drop(all);
        for id in linked {
            self.remount(&id);
        }
    }

    async fn on_mounted(&mut self, id: String, result: Result<phonefiles::Mount>) {
        match result {
            Ok(m) => {
                tracing::info!("{id}: files mounted");
                if self.remounting.remove(&id).is_none() {
                    share::open(&m.path.to_string_lossy());
                }
                self.kept.insert(id.clone());
                self.mounts.insert(id, m);
            }
            Err(e) => {
                tracing::warn!("{id}: couldn't mount its files: {e:#}");
                self.kept.remove(&id);
                self.remounting.remove(&id);
                plugins::tell(&self.notifier, "Couldn't open the phone's files", &format!("{e:#}")).await;
            }
        }
    }

    /// Carries out what a call on [id]'s phone (called [phone]) needs here.
    async fn call_steps(&mut self, id: &str, phone: &str, line: &mut telephony::Line, steps: telephony::Steps, call: Option<&telephony::Call>) {
        for n in steps.close {
            self.notifier.close(n).await;
        }
        if let Some(media) = &self.host.media {
            if steps.pause {
                line.paused = media.pause_all().await;
            }
            if steps.resume {
                for bus in std::mem::take(&mut line.paused) {
                    media.play_bus(&bus).await;
                }
            }
        }
        let Some(call) = call else { return };
        let body = match &call.number {
            Some(n) => format!("{n}, on {phone}"),
            None => format!("On {phone}"),
        };
        if steps.show_ringing {
            let title = format!("{} is calling", call.who);
            let notice = Notice { icon: "call-start", title: &title, body: &body, actions: &[("mute", "Mute")], sticky: true, replaces: line.ringing };
            match self.notifier.show(notice).await {
                Ok(n) => line.ringing = Some(n),
                Err(e) => tracing::warn!("{id}: couldn't show the call: {e:#}"),
            }
        }
        if steps.show_missed {
            let title = format!("Missed call from {}", call.who);
            let notice = Notice { icon: "call-stop", title: &title, body: &body, actions: &[], sticky: false, replaces: line.missed };
            line.missed = self.notifier.show(notice).await.ok();
        }
    }

    fn set_key(&self, id: &str, key: &str, asking: bool) {
        if let Some(d) = self.devices.lock().unwrap().get_mut(id) {
            d.key = key.to_owned();
            d.asking = asking;
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

async fn kde_running(conn: &zbus::Connection) -> bool {
    let reply = conn.call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "NameHasOwner", &("org.kde.kdeconnect",)).await;
    reply.is_ok_and(|r| r.body().deserialize::<bool>().unwrap_or(false))
}

/// The other ways to a phone: the USB cable (adb), USB tethering and hotspots, and Bluetooth; and
/// the offer to install what's missing. Not while kdeconnectd runs: then they're its and the
/// companion's, and a second claim would steal them.
fn transports(lan: Arc<Lan>, devices: Devices) {
    tokio::spawn(vyoma_companion::setup::offer());
    vyoma_companion::set_desk_port(lan.tcp_port());
    tokio::spawn(vyoma_companion::usb::watch());
    tokio::spawn(vyoma_companion::bluetooth::serve(None));
    tokio::spawn(vyoma_companion::gamepad::serve_wired());
    tokio::spawn(vyoma_companion::webcam::remove_stale_mics());
    // A tethered phone isn't on the network we announce to: announce to it directly, and it dials back.
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(TETHER_EVERY).await;
            let linked: Vec<IpAddr> = devices.lock().unwrap().values().filter(|d| d.linked).filter_map(|d| d.remote).collect();
            let phones: Vec<SocketAddr> = vyoma_companion::tether::phones().into_iter().filter(|(ip, _)| !linked.contains(ip)).map(|(ip, _)| SocketAddr::new(ip, UDP_PORT)).collect();
            if !phones.is_empty() {
                let _ = lan.announce(&phones).await;
            }
        }
    });
}

/// The link to use of [routes], newest first among equals: the [cable] while one's plugged in
/// (loopback; steadier than Wi-Fi), then Wi-Fi (any real address), then Bluetooth (loopback).
fn best(routes: &[IpAddr], cable: bool) -> Option<IpAddr> {
    let newest = |wired: bool| routes.iter().rev().find(|r| r.is_loopback() == wired).copied();
    if cable { newest(true).or(newest(false)) } else { newest(false).or(newest(true)) }
}

/// How a linked device is reached: loopback is the cable (while one's tunnelled) or Bluetooth.
fn via(d: &DeviceView) -> Option<&'static str> {
    let remote = d.remote.filter(|_| d.linked)?;
    Some(if !remote.is_loopback() {
        "wifi"
    } else if vyoma_companion::usb::cable_ports().is_empty() {
        "bluetooth"
    } else {
        "cable"
    })
}

/// What the GUI shows of every device, to tell when it changed.
fn shown(devices: &Devices) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let all = devices.lock().unwrap();
    let mut ids: Vec<&String> = all.keys().collect();
    ids.sort();
    for id in ids {
        let d = &all[id];
        (id, &d.name, &d.kind, d.paired, d.linked, &d.key, d.asking, d.battery, d.remote, via(d)).hash(&mut h);
    }
    h.finish()
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
            "incomingCapabilities": ([plugins::INCOMING, features::INCOMING].concat()),
            "outgoingCapabilities": ([plugins::OUTGOING, features::OUTGOING].concat()),
        }),
    )
}

fn host_name() -> String {
    let name = std::fs::read_to_string("/proc/sys/kernel/hostname").unwrap_or_default();
    vyoma_core::lan::device_name(name.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cable_is_the_best_route_then_wifi_then_bluetooth() {
        let (wifi, lo) = (IpAddr::from([192, 168, 1, 5]), IpAddr::from([127, 0, 0, 1]));
        assert_eq!(best(&[lo, wifi], true), Some(lo));
        assert_eq!(best(&[wifi, lo], true), Some(lo));
        assert_eq!(best(&[wifi], true), Some(wifi));
        assert_eq!(best(&[lo, wifi], false), Some(wifi));
        assert_eq!(best(&[lo], false), Some(lo));
        assert_eq!(best(&[], true), None);
    }
}
