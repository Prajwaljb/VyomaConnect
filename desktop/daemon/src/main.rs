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
mod menus;
mod messages;
mod notifications;
mod notify;
mod plugins;
mod gvfs;
mod hfp;
mod phonefiles;
mod places;
mod plain;
mod presenter;
mod presenter_x11;
mod share;
mod system;
mod telephony;
mod update;
mod volume;
mod window;

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
use vyoma_core::invite::Invite;
use vyoma_core::lan::{Lan, Ports, Rules, UDP_PORT};
use vyoma_core::link::packet;
use vyoma_core::trust::TrustStore;
use vyoma_companion::activity::Activity;
use vyoma_companion::permits::{Permit, Permits};
use vyoma_companion::session::{self as features, Host, Phone};
use zbus::fdo;

use clipboard::{CLIPBOARD, CLIPBOARD_CONNECT, Clipboard};
use notifications::{Incoming, Note};
use notify::{Heard, Notice, Notifier};
use share::{SHARE, SHARE_UPDATE, Shared};
use system::{Ringer, Session};

const BUS_NAME: &str = "dev.vyoma.Desktop";
/// The window's response time: `vyoma.echo.request {n}`, which Vyoma on the phone answers with `vyoma.echo {n}`.
const ECHO_REQUEST: &str = "vyoma.echo.request";
const ECHO: &str = "vyoma.echo";
const ECHO_EVERY: Duration = Duration::from_secs(5);
const ECHOES_KEPT: usize = 5;
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
    /// The certificate its links show, which its payloads must show too.
    pub cert: Vec<u8>,
    /// Its best link's address ([best]): Wi-Fi over the cable over Bluetooth.
    pub remote: Option<IpAddr>,
    /// Every link's address.
    pub routes: Vec<IpAddr>,
    /// When it linked (unix ms), while linked.
    pub linked_at: Option<i64>,
    /// Its last few echo round trips (ms), newest last; the window shows their median.
    pub trips: Vec<u32>,
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
    /// kdeconnectd runs beside us (it keeps the cable, tethering and Bluetooth).
    beside_kde: bool,
    permits: Arc<Permits>,
    activity: Arc<Activity>,
    updates: Arc<update::Updates>,
    texts: Arc<messages::Store>,
}

/// A phone has reached this PC over the network since start: a firewall lets it through.
static WIFI_SEEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Vyoma's ports were opened from the window since start (ufw's rules can't be read without root).
static FIREWALL_OPENED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// An install from the window is running.
static INSTALLING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn note_routes(routes: &[IpAddr]) {
    if routes.iter().any(|r| !r.is_loopback()) {
        WIFI_SEEN.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

#[zbus::interface(name = "dev.vyoma.Desktop1")]
impl Control {
    /// A fresh QR pairing code for the window to show, see vyoma_core::invite: this PC's id, name,
    /// certificate fingerprint, addresses and port, and a new one-time secret, after the `#` of the
    /// release page's link. A phone camera without Vyoma opens that page, to get the app; the part
    /// after `#` never leaves the phone (browsers don't send it), and Vyoma's own scanner reads it.
    /// It replaces any code shown before, and lasts ten minutes or until a phone pairs with it.
    fn pairing_code(&self) -> fdo::Result<String> {
        // No address, no code: a phone couldn't reach this PC. The window says so (the error's name).
        let addresses: Vec<String> = vyoma_core::invite::local_addresses().iter().map(|a| a.to_string()).collect();
        if addresses.is_empty() {
            return Err(fdo::Error::Failed("offline".into()));
        }
        let invite = Invite::new(std::time::Instant::now()).ok_or_else(|| fdo::Error::Failed("no randomness".into()))?;
        let secret = vyoma_core::invite::hex(invite.secret());
        self.command(Command::Invite(Some(invite)))?;
        Ok(format!(
            "https://github.com/{}/releases/latest#pair?v=1&id={}&name={}&fp={}&s={secret}&a={}&p={}",
            update::REPO,
            self.me.device_id,
            url_encode(&host_name()),
            vyoma_core::invite::fingerprint(&self.me.cert),
            addresses.join(","),
            self.lan.tcp_port()
        ))
    }

    /// The window stopped showing the code: it can't pair anyone now.
    fn end_pairing_code(&self) -> fdo::Result<()> {
        self.command(Command::Invite(None))
    }

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

    /// One device's state as JSON: battery, how it's linked (`via`: wifi, cable or
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
            "text": plugins::takes(d, messages::REQUEST),
        });
        let linked_paired = all.values().filter(|d| d.paired && d.linked).count();
        let clip = clipboard_sync(&id, d, linked_paired);
        // The typical round trip, not the latest: one slow answer shouldn't make the link look bad.
        // `since`: when it linked (unix ms); `rtt`: how fast it answers (ms).
        let rtt = (!d.trips.is_empty()).then(|| {
            let mut t = d.trips.clone();
            t.sort_unstable();
            t[t.len() / 2]
        });
        Ok(json!({ "name": d.name, "type": d.kind, "paired": d.paired, "linked": d.linked, "key": d.key, "asking": d.asking, "battery": battery, "via": via(d), "can": can, "clipboardSync": clip, "rtt": rtt, "since": d.linked_at }).to_string())
    }

    /// Lets the plugged-in phone's clipboard sync work with Vyoma closed (the USB notification's
    /// Allow, from the window). Needs the phone on the cable with USB debugging on.
    async fn allow_clipboard_sync(&self, id: String, #[zbus(signal_emitter)] emitter: zbus::object_server::SignalEmitter<'_>) -> fdo::Result<()> {
        vyoma_companion::usb::allow_clipboard().await.map_err(|e| fdo::Error::Failed(format!("{e:#}")))?;
        clip_remember(&id);
        let _ = Control::changed(&emitter).await;
        Ok(())
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

    /// Sends files and folders to the device; it fetches them one by one. Fails at once if one
    /// can't be read.
    fn send_files(&self, id: String, paths: Vec<String>) -> fdo::Result<()> {
        let pinned = self.sharing(&id)?;
        let files = share::expand(&paths).map_err(|e| fdo::Error::InvalidArgs(format!("{e:#}")))?;
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
                    let why = plain::send(&e);
                    plugins::tell(&notifier, "Couldn't send", &why).await;
                    why
                }
            };
            if let Ok(emitter) = zbus::object_server::SignalEmitter::new(&conn, OBJECT_PATH) {
                let _ = Control::files_sent(&emitter, &id, count, &error).await;
            }
        });
        Ok(())
    }

    /// [send_files] from the file manager's "Send to" menu, where no window shows how it went:
    /// notifications say it's sending, then that it's sent, or what stopped it.
    fn send(&self, id: String, paths: Vec<String>) {
        let (name, problem) = match self.devices.lock().unwrap().get(&id) {
            None => ("That phone".to_owned(), Some("It isn't paired with this PC anymore.".to_owned())),
            Some(d) => {
                let problem = if !d.paired {
                    Some(format!("{} isn't paired with this PC anymore.", d.name))
                } else if !d.linked {
                    Some(format!("{} isn't connected. Open Vyoma on it, then try again.", d.name))
                } else if via(d) == Some("bluetooth") {
                    Some(format!("{} is only on Bluetooth, which can't carry files. Connect it by Wi-Fi or USB, then try again.", d.name))
                } else if !plugins::takes(d, SHARE) {
                    Some(format!("{} doesn't take files from this PC.", d.name))
                } else {
                    None
                };
                (d.name.clone(), problem)
            }
        };
        let notifier = self.notifier.clone();
        let failed = format!("Couldn't send to {name}");
        let files = match (problem, self.sharing(&id), share::expand(&paths)) {
            (Some(why), _, _) => Err(why),
            (None, Err(e), _) => Err(e.to_string()),
            (None, _, Err(e)) => {
                tracing::warn!("{id}: {e:#}");
                Err(plain::send(&e))
            }
            (None, Ok(pinned), Ok(files)) => Ok((pinned, files)),
        };
        let (pinned, files) = match files {
            Ok(f) if !f.1.is_empty() => f,
            Ok(_) => {
                tokio::spawn(async move { plugins::tell(&notifier, &failed, "That folder is empty.").await });
                return;
            }
            Err(why) => {
                tokio::spawn(async move { plugins::tell(&notifier, &failed, &why).await });
                return;
            }
        };
        let what = share::summary(&files);
        let (me, commands, conn) = (self.me.clone(), self.commands.clone(), self.conn.clone());
        let count = files.len() as u32;
        tokio::spawn(async move {
            let started = sending(&notifier, &format!("Sending to {name}"), &what, None).await;
            let error = match share::send_files(&commands, &id, &me, &pinned, files).await {
                Ok(()) => {
                    sending(&notifier, &format!("Sent to {name}"), &what, started).await;
                    String::new()
                }
                Err(e) => {
                    tracing::warn!("{id}: {e:#}");
                    let why = plain::send(&e);
                    sending(&notifier, &failed, &why, started).await;
                    why
                }
            };
            if let Ok(emitter) = zbus::object_server::SignalEmitter::new(&conn, OBJECT_PATH) {
                let _ = Control::files_sent(&emitter, &id, count, &error).await;
            }
        });
    }

    /// A [send_files] batch is over: [error] is empty when the device fetched all [count].
    #[zbus(signal)]
    async fn files_sent(emitter: &zbus::object_server::SignalEmitter<'_>, id: &str, count: u32, error: &str) -> zbus::Result<()>;

    /// This PC's setup as JSON: each feature (`ready`, or what it `needs`), whether Install can
    /// help (`installable`, `installing`), checks no package fixes (`checks`: name, `state` ok,
    /// info or warn, detail) and advice (`notes`).
    async fn health(&self) -> fdo::Result<String> {
        let (beside_kde, updates) = (self.beside_kde, self.updates.clone());
        let shell = if vyoma_companion::desktop::is_gnome() { Some(vyoma_companion::shell::version().await) } else { None };
        let lock = system::cannot_lock().await;
        let report = tokio::task::spawn_blocking(move || {
            let mut report = health(beside_kde, shell);
            if let (Some(why), Some(checks)) = (lock, report["checks"].as_array_mut()) {
                checks.push(json!({ "name": "Screen lock", "state": "info", "detail": why }));
            }
            report["update"] = updates.status();
            report
        })
        .await
        .map_err(|e| fdo::Error::Failed(e.to_string()))?;
        Ok(report.to_string())
    }

    /// Updates the PC app to the newest release (This PC's Update); it restarts when done.
    async fn update(&self) -> fdo::Result<()> {
        self.updates.start().await.map_err(|e| fdo::Error::Failed(format!("{e:#}")))
    }

    /// Turns the daily check for a new release on or off.
    async fn set_update_checks(&self, on: bool, #[zbus(signal_emitter)] emitter: zbus::object_server::SignalEmitter<'_>) -> fdo::Result<()> {
        self.updates.set_checks(on);
        let _ = Control::changed(&emitter).await;
        Ok(())
    }

    /// Installs what's missing (asks for the password once); [setup_done] says how it went.
    fn install(&self) -> fdo::Result<()> {
        if INSTALLING.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return Ok(());
        }
        let conn = self.conn.clone();
        tokio::spawn(async move {
            let ok = vyoma_companion::setup::install().await;
            INSTALLING.store(false, std::sync::atomic::Ordering::SeqCst);
            tracing::info!("install from the window: {}", if ok { "done" } else { "didn't finish" });
            if let Ok(emitter) = zbus::object_server::SignalEmitter::new(&conn, OBJECT_PATH) {
                let _ = Control::setup_done(&emitter, ok).await;
            }
        });
        Ok(())
    }

    /// Opens Vyoma's ports in the firewall that's up (asks for the password once); [firewall_done] says how it went.
    fn open_firewall(&self) -> fdo::Result<()> {
        let conn = self.conn.clone();
        tokio::spawn(async move {
            let ok = open_firewall().await;
            if ok {
                FIREWALL_OPENED.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            tracing::info!("firewall from the window: {}", if ok { "opened" } else { "not opened" });
            if let Ok(emitter) = zbus::object_server::SignalEmitter::new(&conn, OBJECT_PATH) {
                let _ = Control::firewall_done(&emitter, ok).await;
                let _ = Control::changed(&emitter).await;
            }
        });
        Ok(())
    }

    /// An [open_firewall] is over.
    #[zbus(signal)]
    async fn firewall_done(emitter: &zbus::object_server::SignalEmitter<'_>, ok: bool) -> zbus::Result<()>;

    /// An [install] is over.
    #[zbus(signal)]
    async fn setup_done(emitter: &zbus::object_server::SignalEmitter<'_>, ok: bool) -> zbus::Result<()>;

    /// Sends text to the device, or opens a link there.
    fn send_text(&self, id: String, text: String) -> fdo::Result<()> {
        self.sharing(&id)?;
        self.command(Command::Send(id, share::text_packet(&text)))
    }

    /// Adds a command devices can run here; its key.
    async fn add_command(&self, name: String, command: String, #[zbus(signal_emitter)] emitter: zbus::object_server::SignalEmitter<'_>) -> fdo::Result<String> {
        let key = self.run.add(&name, &command).map_err(|e| fdo::Error::Failed(format!("{e:#}")))?;
        self.commands_changed();
        let _ = Control::changed(&emitter).await;
        Ok(key)
    }

    /// Changes a command, or adds one when [key] is empty ([fingerprint]: the phone asks for its
    /// owner's fingerprint before running it); its key.
    async fn save_command(&self, key: String, name: String, command: String, fingerprint: bool, every_time: bool, #[zbus(signal_emitter)] emitter: zbus::object_server::SignalEmitter<'_>) -> fdo::Result<String> {
        let key = self.run.save_one(&key, &name, &command, fingerprint, every_time).map_err(|e| fdo::Error::Failed(format!("{e:#}")))?;
        self.commands_changed();
        let _ = Control::changed(&emitter).await;
        Ok(key)
    }

    async fn remove_command(&self, key: String, #[zbus(signal_emitter)] emitter: zbus::object_server::SignalEmitter<'_>) -> fdo::Result<()> {
        match self.run.remove(&key) {
            Ok(true) => {
                self.commands_changed();
                let _ = Control::changed(&emitter).await;
                Ok(())
            }
            Ok(false) => Err(fdo::Error::UnknownObject(key)),
            Err(e) => Err(fdo::Error::Failed(format!("{e:#}"))),
        }
    }

    /// The commands as JSON: {key: {name, command, fingerprint}}.
    fn commands(&self) -> String {
        Value::Object(self.run.all().into_iter().collect()).to_string()
    }

    /// Stops this PC ringing for a device that's looking for it.
    fn stop_ringing(&self) -> fdo::Result<()> {
        self.asks.send(Ask::StopRinging).map_err(|_| fdo::Error::Failed("stopping".into()))
    }

    /// The desktop's wallpaper as a grid of colours, for the window's canvas and accent (the phone
    /// takes the same from the companion): {cols, rows, colors: [[r, g, b], ...]}, row-major.
    async fn wallpaper(&self) -> fdo::Result<String> {
        let colors = vyoma_companion::wallpaper::colors().await.map_err(|e| fdo::Error::Failed(format!("{e:#}")))?;
        Ok(json!({ "cols": vyoma_companion::wallpaper::COLS, "rows": vyoma_companion::wallpaper::ROWS, "colors": colors }).to_string())
    }

    /// What phones did here, newest first, as JSON: [{time (Unix seconds), phone (id), name, kind, text}].
    fn activity(&self) -> String {
        Value::Array(self.activity.recent()).to_string()
    }

    /// The phone's text conversations as JSON (`messages.rs`), newest first; asks the phone the
    /// first time on each link, and [messages_changed] follows as they come.
    fn conversations(&self, id: String) -> fdo::Result<String> {
        let takes = self.devices.lock().unwrap().get(&id).is_some_and(|d| plugins::takes(d, messages::REQUEST));
        if !takes {
            return Err(fdo::Error::NotSupported("This phone doesn't share its texts".into()));
        }
        let guarded = self.texts_guarded(&id);
        let (json, ask) = self.texts.conversations(&id, guarded);
        for p in ask {
            self.command(Command::Send(id.clone(), p))?;
        }
        Ok(json)
    }

    /// One conversation's texts as JSON, oldest first.
    fn thread(&self, id: String, thread: i64) -> fdo::Result<String> {
        let (json, ask) = self.texts.thread(&id, thread, self.texts_guarded(&id));
        for p in ask {
            self.command(Command::Send(id.clone(), p))?;
        }
        Ok(json)
    }

    /// Texts [text] to [thread]'s people from the phone (D-Bus `TextThread`).
    fn text_thread(&self, id: String, thread: i64, text: String) -> fdo::Result<()> {
        let linked = self.devices.lock().unwrap().get(&id).is_some_and(|d| d.linked && plugins::takes(d, messages::REQUEST));
        if !linked {
            return Err(fdo::Error::Failed("The phone isn't connected".into()));
        }
        let p = self.texts.send(&id, thread, &text, self.texts_guarded(&id)).map_err(fdo::Error::Failed)?;
        tracing::info!("{id}: sending a text");
        self.command(Command::Send(id, p))
    }

    /// The Messages page was opened, a conversation picked or a text sent: a phone with the texts lock is asked to
    /// let them in (its owner confirms there), or, already open, stays open 10 more minutes.
    fn unlock_texts(&self, id: String) -> fdo::Result<()> {
        if !self.texts_guarded(&id) {
            return Ok(());
        }
        let linked = self.devices.lock().unwrap().get(&id).is_some_and(|d| d.linked);
        if !linked {
            return Err(fdo::Error::Failed("The phone isn't connected".into()));
        }
        if let Some(p) = self.texts.unlock(&id) {
            tracing::info!("{id}: asking to show the texts");
            self.command(Command::Send(id, p))?;
        }
        Ok(())
    }

    /// The texts have something new.
    #[zbus(signal)]
    async fn messages_changed(emitter: &zbus::object_server::SignalEmitter<'_>) -> zbus::Result<()>;

    fn clear_activity(&self) {
        self.activity.clear();
    }

    /// [activity] has something new (or was cleared).
    #[zbus(signal)]
    async fn activity_changed(emitter: &zbus::object_server::SignalEmitter<'_>) -> zbus::Result<()>;

    /// What the device may do here, as JSON: [{key, name, detail, on}], in the window's order.
    fn permissions(&self, id: String) -> String {
        let list: Vec<Value> = self.permits.of(&id).into_iter().map(|(p, on)| json!({ "key": p.key(), "name": p.name(), "detail": p.detail(), "on": on })).collect();
        Value::Array(list).to_string()
    }

    /// Turns one of [permissions] on or off; off stops what it's doing at once.
    async fn set_permission(&self, id: String, key: String, on: bool, #[zbus(signal_emitter)] emitter: zbus::object_server::SignalEmitter<'_>) -> fdo::Result<()> {
        let p = Permit::from_key(&key).ok_or_else(|| fdo::Error::InvalidArgs(format!("no permission {key}")))?;
        self.permits.set(&id, p, on).map_err(|e| fdo::Error::Failed(format!("{e:#}")))?;
        tracing::info!("{id}: {key} {}", if on { "allowed" } else { "turned off" });
        let _ = Control::changed(&emitter).await;
        Ok(())
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
    /// The phone holds its texts behind Vyoma's texts lock.
    fn texts_guarded(&self, id: &str) -> bool {
        self.devices.lock().unwrap().get(id).is_some_and(|d| plugins::takes(d, messages::UNLOCK))
    }

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
    /// The phones' texts, for the window's Messages page.
    texts: Arc<messages::Store>,
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
    /// When each phone's remote input was last refused (see [Permit::Control]).
    refused_input: HashMap<String, std::time::Instant>,
    /// Device id to its running features session, which takes its vyoma.* packets.
    sessions: HashMap<String, mpsc::UnboundedSender<(Value, IpAddr)>>,
    /// Device id to the echo it was last sent and when, for the window's response time.
    echoes: HashMap<String, (u64, std::time::Instant)>,
    echo_n: u64,
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
        Some(menus::SEND_ARG) => menus::send(&std::env::args().skip(2).collect::<Vec<_>>()),
        Some("--remove-menus") => {
            menus::remove_all();
            Ok(())
        }
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
    let permits = Arc::new(Permits::load(dir.join("permissions.json")));
    let activity = Arc::new(Activity::load(dir.join("activity.json")));
    let texts = Arc::new(messages::Store::default());
    clip_load(&dir);
    let updates = Arc::new(update::Updates::load(&dir));
    if std::env::var_os("VYOMA_DIR").is_none() {
        tokio::spawn(updates.clone().run());
    }
    let control = Control { me: me.clone(), run: run.clone(), notifier: notifier.clone(), commands: commands.clone(), asks: asks_tx, devices: devices.clone(), lan: lan.clone(), targets: targets.clone(), conn: conn.clone(), beside_kde, permits: permits.clone(), activity: activity.clone(), updates: updates.clone(), texts: texts.clone() };
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
    // "Send to <phone>" in the file managers' menus, for every paired phone. Not from a test
    // run in another folder: its phones aren't the user's.
    if std::env::var_os("VYOMA_DIR").is_none() {
        let devices = devices.clone();
        tokio::spawn(async move {
            let mut last = None;
            loop {
                let mut phones: Vec<menus::Phone> = devices.lock().unwrap().iter().filter(|(_, d)| d.paired).map(|(id, d)| menus::Phone { id: id.clone(), name: d.name.clone() }).collect();
                phones.sort_by(|a, b| (&a.name, &a.id).cmp(&(&b.name, &b.id)));
                // Also when a file manager, or Nautilus's Python extensions, comes or goes.
                let now = (phones.clone(), menus::installed());
                if last.as_ref() != Some(&now) {
                    last = Some(now);
                    let _ = tokio::task::spawn_blocking(move || menus::update(&phones)).await;
                }
                tokio::time::sleep(CHANGES_EVERY).await;
            }
        });
    }
    // And when the texts did.
    {
        let (conn, mut ticks) = (conn.clone(), texts.subscribe());
        tokio::spawn(async move {
            while ticks.changed().await.is_ok() {
                if let Ok(emitter) = zbus::object_server::SignalEmitter::new(&conn, OBJECT_PATH) {
                    let _ = Control::messages_changed(&emitter).await;
                }
            }
        });
    }
    // And when the activity grew.
    {
        let (conn, mut ticks) = (conn.clone(), activity.subscribe());
        tokio::spawn(async move {
            while ticks.changed().await.is_ok() {
                if let Ok(emitter) = zbus::object_server::SignalEmitter::new(&conn, OBJECT_PATH) {
                    let _ = Control::activity_changed(&emitter).await;
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
    // Again after a while if it stops: GNOME's extension can come after us, or come back after
    // the shell restarts.
    tokio::spawn(async move {
        let mut said = false;
        loop {
            if let Err(e) = clipboard::watch(copies_tx.clone()).await {
                if !said {
                    tracing::warn!("can't follow the clipboard: {e:#}");
                    said = true;
                }
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
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
    let host = Arc::new(Host { me: me.clone(), media: player, away: Arc::default(), permits, activity });

    let mut desk = Desk {
        host,
        texts,
        refused_input: HashMap::new(),
        sessions: HashMap::new(),
        echoes: HashMap::new(),
        echo_n: 0,
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
    phonefiles::tidy(&dir).await;
    if beside_kde {
        tracing::info!("kdeconnectd is running: the cable, tethering and Bluetooth stay its own");
    } else {
        transports(lan.clone(), desk.devices.clone());
    }

    let mut battery_tick = tokio::time::interval(BATTERY_EVERY);
    let mut texts_tick = tokio::time::interval(Duration::from_secs(30));
    let mut mounts_tick = tokio::time::interval(MOUNTS_EVERY);
    let mut echo_tick = tokio::time::interval(ECHO_EVERY);
    loop {
        tokio::select! {
            _ = echo_tick.tick() => desk.send_echoes(),
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
                if locked {
                    for id in desk.texts.lock_all() {
                        tracing::info!("{id}: texts locked with the screen");
                        desk.send(&id, vyoma_core::link::packet(messages::LOCK, json!({})));
                    }
                }
                if desk.locked != Some(locked) {
                    desk.locked = Some(locked);
                    desk.broadcast(plugins::LOCK, plugins::lock_packet(locked));
                }
            }
            _ = texts_tick.tick() => {
                for id in desk.texts.expire() {
                    tracing::info!("{id}: texts locked, unused for a while");
                    desk.send(&id, vyoma_core::link::packet(messages::LOCK, json!({})));
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

    /// Times each linked phone's link that answers echoes: one at a time, every [ECHO_EVERY].
    fn send_echoes(&mut self) {
        let due: Vec<String> = self.devices.lock().unwrap().iter().filter(|(_, d)| d.linked && plugins::takes(d, ECHO_REQUEST)).map(|(id, _)| id.clone()).collect();
        for id in due {
            self.echo_n += 1;
            self.echoes.insert(id.clone(), (self.echo_n, std::time::Instant::now()));
            self.send(&id, packet(ECHO_REQUEST, json!({ "n": self.echo_n })));
        }
    }

    fn on_echo(&mut self, id: &str, packet: &Value) {
        let Some((n, sent)) = self.echoes.get(id).copied() else { return };
        if packet["body"]["n"].as_u64() != Some(n) {
            return;
        }
        self.echoes.remove(id);
        let ms = sent.elapsed().as_millis().min(60_000) as u32;
        if let Some(d) = self.devices.lock().unwrap().get_mut(id) {
            d.trips.push(ms);
            let extra = d.trips.len().saturating_sub(ECHOES_KEPT);
            d.trips.drain(..extra);
        }
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
                plugins::tell(&self.notifier, &format!("Couldn't receive {}", r.name), &plain::receive(&e)).await;
                return;
            }
        };
        tracing::info!("{}: received {}", r.id, path.display());
        let batch = self.batches.entry(r.id.clone()).or_default();
        batch.done += 1;
        let (done, replaces) = (batch.done, batch.notice);
        if done >= r.count {
            self.batches.remove(&r.id);
            let text = if r.count <= 1 {
                format!("Shared \u{201c}{}\u{201d} with this PC", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
            } else {
                format!("Shared {} files with this PC", r.count)
            };
            self.host.activity.add(&r.id, &features::phone_name(&device), "share", text);
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
        if let Heard::Pressed { notification, action } = &h
            && action == notify::INLINE_REPLY
        {
            if let Some(ask) = self.mirror.take_reply(*notification) {
                let (commands, notifier) = (self.commands.clone(), self.notifier.clone());
                tokio::spawn(async move {
                    match notifications::ask_reply(&ask).await {
                        Ok(Some(text)) => {
                            let _ = commands.send(Command::Send(ask.device.clone(), ask.packet(&text)));
                        }
                        Ok(None) => {}
                        Err(e) => {
                            tracing::warn!("couldn't ask for a reply: {e:#}");
                            plugins::tell(&notifier, "Can't reply from this PC", &plain::reply(&e)).await;
                        }
                    }
                });
            }
            return;
        }
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
        if let Some(id) = telephony::line_of(&self.calls, notification) {
            return self.on_call_pressed(id, action).await;
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
                    if !d.linked {
                        (d.linked_at, d.trips) = (Some(vyoma_core::link::now_ms()), Vec::new());
                    }
                    (d.name, d.kind, d.paired, d.linked, d.incoming, d.cert, d.routes, d.remote) = (name, kind, paired, true, incoming, cert, vec![remote], Some(remote));
                    note_routes(&d.routes);
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
                    note_routes(&remotes);
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
                self.texts.gone(&id);
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
                self.echoes.remove(&id);
                if all.get(&id).is_some_and(|d| d.paired) {
                    let d = all.get_mut(&id).unwrap();
                    (d.linked, d.battery, d.linked_at, d.trips) = (false, None, None, Vec::new());
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
            Event::Paired { id, name, by_code, watched } => {
                if let Some(d) = self.devices.lock().unwrap().get_mut(&id) {
                    (d.paired, d.key) = (true, String::new());
                }
                if let Some(n) = self.prompts.remove(&id) {
                    self.notifier.close(n).await;
                }
                // Nobody here said yes to a code pairing: say how to undo one that wasn't wanted. With the
                // code still on screen the window showed it happen, its tick saying the same thing.
                if !(by_code && watched) {
                    let body = if by_code { "It scanned the code in Vyoma's window. Not you? Unpair it there." } else { "" };
                    plugins::tell(&self.notifier, &format!("Paired with {name}"), body).await;
                }
                self.start(&id);
            }
            Event::Unpaired { id, name } => {
                self.sessions.remove(&id);
                self.texts.forget(&id);
                self.host.permits.forget(&id);
                self.kept.remove(&id);
                if let Some(m) = self.mounts.remove(&id) {
                    m.unmount().await;
                }
                {
                    let mut all = self.devices.lock().unwrap();
                    if all.get(&id).is_some_and(|d| d.linked) {
                        let d = all.get_mut(&id).unwrap();
                        (d.paired, d.battery) = (false, None);
                    } else {
                        all.remove(&id);
                    }
                }
                plugins::tell(&self.notifier, &format!("Unpaired from {name}"), "").await;
            }
            Event::Packet { id, packet, from } => {
                let Some(mut device) = self.devices.lock().unwrap().get(&id).cloned() else { return };
                if packet["type"] == ECHO {
                    return self.on_echo(&id, &packet);
                }
                if packet["type"] == messages::UNLOCKED || packet["type"] == messages::LOCKED {
                    tracing::info!("{id}: {}", packet["type"].as_str().unwrap_or(""));
                    self.texts.on_packet(&id, &packet);
                    return;
                }
                if packet["type"] == telephony::DONE {
                    return self.on_call_done(&id, &packet).await;
                }
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
                        if !self.host.permits.allows(&id, Permit::Control) {
                            // Said once a while, not for every move of the finger.
                            let now = std::time::Instant::now();
                            if self.refused_input.get(&id).is_none_or(|t| now.duration_since(*t) > Duration::from_secs(5)) {
                                self.refused_input.insert(id.clone(), now);
                                tracing::info!("{id}: remote input refused, control is off for this phone");
                                if plugins::takes(&device, vyoma_companion::permits::DENIED) {
                                    self.send(&id, vyoma_companion::permits::denied(Permit::Control, &packet));
                                }
                            }
                            return;
                        }
                        let _ = self.input.send((std::time::Instant::now(), id, packet));
                        return;
                    }
                    commands::RUNCOMMAND_REQUEST => {
                        let body = &packet["body"];
                        if body["requestCommandList"].as_bool() == Some(true) {
                            self.send(&id, self.run.list_packet());
                        }
                        // The phone's "add command": the editor is Vyoma's window.
                        if body["setup"].as_bool() == Some(true) {
                            tracing::info!("{id}: opening the command editor");
                            tokio::spawn(window::open("commands"));
                        }
                        if let Some(key) = body["key"].as_str() {
                            match self.run.run(key, commands::failed) {
                                Some(name) => {
                                    tracing::info!("{id}: ran {name}");
                                    self.host.activity.add(&id, &features::phone_name(&device.name), "command", format!("Ran \u{201c}{name}\u{201d}"));
                                }
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
                    messages::MESSAGES | messages::CONTACTS_UIDS_REPLY | messages::CONTACTS_VCARDS_REPLY => {
                        for p in self.texts.on_packet(&id, &packet) {
                            self.send(&id, p);
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
                    d.battery = device.battery;
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
                show_files(m, true);
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
        let (at, name) = (phonefiles::mount_point(&device.name), device.name.clone());
        let (key, cert, dir, done) = (self.dir.join("privateKey.pem"), device.cert.clone(), self.dir.clone(), self.mounted.clone());
        tokio::spawn(async move {
            if let Some(old) = old {
                old.unmount().await;
            }
            let result = phonefiles::mount(&server, remote, &id, &name, &at, &key, &cert, &dir).await;
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
                show_files(&m, self.remounting.remove(&id).is_none());
                self.kept.insert(id.clone());
                self.mounts.insert(id, m);
            }
            Err(e) => {
                tracing::warn!("{id}: couldn't mount its files: {e:#}");
                self.kept.remove(&id);
                self.remounting.remove(&id);
                plugins::tell(&self.notifier, "Couldn't open the phone's files", &plain::mount(&e)).await;
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
        if steps.release
            && let Some(audio) = line.audio.take()
        {
            give_back(audio);
        }
        let Some(call) = call else { return };
        let body = call.body(phone);
        if steps.show_talking {
            line.talking_title = call.talking_title();
            self.show_talking(id, phone, line).await;
        }
        if steps.show_ringing {
            let title = call.ringing_title();
            let actions = telephony::ringing_actions(call.answerable);
            let notice = Notice { icon: "call-start", title: &title, body: &body, actions, sticky: true, replaces: line.ringing };
            match self.notifier.show(notice).await {
                Ok(n) => line.ringing = Some(n),
                Err(e) => tracing::warn!("{id}: couldn't show the call: {e:#}"),
            }
        }
        if steps.show_missed {
            let title = call.missed_title();
            let notice = Notice { icon: "call-stop", title: &title, body: &body, actions: &[], sticky: false, replaces: line.missed };
            line.missed = self.notifier.show(notice).await.ok();
        }
    }

    /// A button on [id]'s ringing or talking notification.
    async fn on_call_pressed(&mut self, id: String, action: &str) {
        let Some(mut line) = self.calls.remove(&id) else { return };
        let phone = self.devices.lock().unwrap().get(&id).map(|d| d.name.clone()).unwrap_or_default();
        match action {
            "mute" => {
                tracing::info!("{id}: muting the ringer");
                self.send(&id, packet(telephony::MUTE, json!({})));
            }
            "answer" | "decline" | "hangup" => {
                tracing::info!("{id}: asking the phone to {action}");
                self.send(&id, packet(telephony::REQUEST, json!({ "action": action })));
                match action {
                    "answer" => {
                        line.answered = true;
                        // Its own headset keeps the sound; else it comes here, connecting while it's answered.
                        if !line.headset {
                            self.take_audio(&mut line);
                        }
                    }
                    "decline" => line.declined = true,
                    _ => {}
                }
            }
            "phone-audio" | "pc-audio" => {
                match line.audio.take() {
                    Some(audio) => give_back(audio),
                    None => self.take_audio(&mut line),
                }
                tracing::info!("{id}: call sound on {}", if line.audio.is_some() { "this PC" } else { "the phone" });
                self.show_talking(&id, &phone, &mut line).await;
            }
            _ => {}
        }
        self.calls.insert(id, line);
    }

    /// Connects this PC as the phone's headset for the call, in the background.
    fn take_audio(&self, line: &mut telephony::Line) {
        if line.audio.is_some() {
            return;
        }
        let (notifier, name) = (self.notifier.clone(), line.bluetooth.clone());
        line.audio = Some(tokio::spawn(async move {
            match hfp::take(name.as_deref()).await {
                Ok(audio) => Some(audio),
                Err(e) => {
                    tracing::warn!("call audio: {e:#}");
                    plugins::tell(&notifier, "The call's sound stays on the phone", &plain::call_audio(&e)).await;
                    None
                }
            }
        }));
    }

    /// A call answered here: who, where its sound is, Hang up.
    async fn show_talking(&self, id: &str, phone: &str, line: &mut telephony::Line) {
        let (body, actions) = telephony::talking_notice(line.audio.is_some(), phone);
        let notice = Notice { icon: "call-start", title: &line.talking_title, body: &body, actions, sticky: true, replaces: line.talking };
        match self.notifier.show(notice).await {
            Ok(n) => line.talking = Some(n),
            Err(e) => tracing::warn!("{id}: couldn't show the call: {e:#}"),
        }
    }

    /// The phone did (or couldn't do) what a call button asked.
    async fn on_call_done(&mut self, id: &str, packet: &Value) {
        let b = &packet["body"];
        let action = b["action"].as_str().unwrap_or("");
        let Some(error) = b["error"].as_str() else { return };
        tracing::info!("{id}: the phone couldn't {action} the call");
        if action == "answer"
            && let Some(line) = self.calls.get_mut(id)
        {
            line.answered = false;
            if let Some(audio) = line.audio.take() {
                give_back(audio);
            }
        }
        plugins::tell(&self.notifier, telephony::failed_title(action), error).await;
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

/// Gives a call's sound back to the phone once it's been taken (or failed to be).
fn give_back(audio: tokio::task::JoinHandle<Option<hfp::Audio>>) {
    tokio::spawn(async move {
        if let Ok(Some(audio)) = audio.await {
            hfp::release(audio).await;
        }
    });
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
/// Phones whose background clipboard sync was seen granted: it lasts until the app's uninstalled,
/// and the cable (the only place it shows) is usually out.
static CLIP_ON: std::sync::OnceLock<(PathBuf, Mutex<std::collections::BTreeSet<String>>)> = std::sync::OnceLock::new();

fn clip_load(dir: &std::path::Path) {
    let file = dir.join("clipboard-sync.json");
    let ids = std::fs::read_to_string(&file).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let _ = CLIP_ON.set((file, Mutex::new(ids)));
}

fn clip_remember(id: &str) {
    let Some((file, ids)) = CLIP_ON.get() else { return };
    let mut ids = ids.lock().unwrap();
    if ids.insert(id.to_owned()) {
        let _ = std::fs::write(file, serde_json::to_string(&*ids).unwrap_or_default());
    }
}

/// "on", "ask" (on the cable, waiting for Allow) or "plug" (unknown until it's plugged in). A phone
/// on the cable is the one linked by it, or the only one linked at all.
fn clipboard_sync(id: &str, d: &DeviceView, linked_paired: usize) -> &'static str {
    let plugged = vyoma_companion::usb::clipboard();
    let mine = d.paired && d.linked && !plugged.is_empty() && (via(d) == Some("cable") || linked_paired == 1);
    if mine {
        if plugged.iter().any(|c| c.granted()) {
            clip_remember(id);
            return "on";
        }
        return "ask";
    }
    if CLIP_ON.get().is_some_and(|(_, ids)| ids.lock().unwrap().contains(id)) { "on" } else { "plug" }
}

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

/// [Control::health]'s report.
/// [shell]: on GNOME, the running extension's version, if it runs.
fn health(beside_kde: bool, shell: Option<Option<u32>>) -> serde_json::Value {
    let report = vyoma_companion::setup::report();
    let features: Vec<_> = report
        .features
        .iter()
        .map(|f| json!({ "name": capitalised(f.feature), "ready": f.ready, "needs": f.needs, "unavailable": f.unavailable }))
        .collect();
    let mut checks = vec![firewall()];
    if beside_kde {
        checks.push(json!({ "name": "KDE Connect", "state": "warn",
            "detail": "Also running, so the USB cable, tethering and Bluetooth stay with it; the rest works here. To use them with Vyoma, turn KDE Connect off in your startup apps, then log out and back in." }));
    }
    if let Some(running) = shell {
        checks.push(gnome_extension(running));
    }
    json!({
        "features": features,
        "installable": report.installable,
        "installing": INSTALLING.load(std::sync::atomic::Ordering::Relaxed),
        "checks": checks,
        "notes": report.notes,
    })
}

/// Whether a firewall is up, and whether phones get through it. firewalld says whether a port is
/// open to anyone who asks; ufw's rules need root to read, so a phone that reached us over the
/// network, or the ports opened from here, says enough. Up with none seen yet may be nothing (a
/// day on the cable proves nothing either way), but phones on Wi-Fi would stop there: it warns, with the fix.
fn firewall() -> serde_json::Value {
    let active = ["ufw", "firewalld"].into_iter().find(|unit| {
        std::process::Command::new("systemctl").args(["is-active", "--quiet", unit]).status().is_ok_and(|s| s.success())
    });
    let quiet = |args: &[&str]| {
        std::process::Command::new("firewall-cmd")
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    };
    let open = |name: &str| {
        WIFI_SEEN.load(std::sync::atomic::Ordering::Relaxed)
            || FIREWALL_OPENED.load(std::sync::atomic::Ordering::Relaxed)
            || (name == "firewalld" && (quiet(&["--query-port=1716/tcp"]) || quiet(&["--query-service=kdeconnect"])))
            || (name == "ufw" && ufw_lets_vyoma_in())
    };
    let (state, detail, action) = match active {
        None => ("ok", "Off".to_owned(), ""),
        Some(name) if open(name) => ("ok", "On, and phones get through".to_owned(), ""),
        Some(name) => {
            tracing::debug!("{name} is up and Vyoma's ports (1714 to 1764) aren't seen open");
            ("warn", "This PC's firewall may be blocking phones on Wi-Fi.".to_owned(), "Let phones through")
        }
    };
    json!({ "name": "Firewall", "state": state, "detail": detail, "action": action })
}

/// ufw's own record of its rules, where a distro leaves it readable: Vyoma's TCP port allowed in.
fn ufw_lets_vyoma_in() -> bool {
    std::fs::read_to_string("/etc/ufw/user.rules").is_ok_and(|rules| ufw_allows(&rules))
}

/// Whether ufw's rules (its `### tuple ###` lines) let TCP in on port 1716.
fn ufw_allows(rules: &str) -> bool {
    rules.lines().filter_map(|l| l.strip_prefix("### tuple ### allow ")).any(|rule| {
        let mut parts = rule.split_whitespace();
        let (Some(proto), Some(port)) = (parts.next(), parts.next()) else { return false };
        (proto == "tcp" || proto == "any")
            && port.split(',').any(|p| match p.split_once(':') {
                Some((lo, hi)) => lo.parse::<u16>().is_ok_and(|lo| lo <= 1716) && hi.parse::<u16>().is_ok_and(|hi| hi >= 1716),
                None => p == "1716",
            })
    })
}

/// Lets phones through the firewall that's up: Vyoma's (KDE Connect's) ports, TCP and UDP, kept
/// across restarts. Run as root by pkexec, outside this service's sandbox, from constants only.
const OPEN_FIREWALL: &str = "set -e
if systemctl is-active --quiet firewalld; then
  firewall-cmd --permanent --add-port=1714-1764/tcp --add-port=1714-1764/udp
  firewall-cmd --reload
fi
if systemctl is-active --quiet ufw; then
  ufw allow 1714:1764/tcp
  ufw allow 1714:1764/udp
fi
";

/// Runs [OPEN_FIREWALL] (one password prompt); whether it finished.
async fn open_firewall() -> bool {
    let status = tokio::process::Command::new("systemd-run")
        .args(["--user", "--wait", "--collect", "--quiet", "--unit=vyoma-desktop-firewall", "pkexec", "sh", "-c", OPEN_FIREWALL])
        .status()
        .await;
    status.is_ok_and(|s| s.success())
}

/// Vyoma's GNOME extension: clipboard sync, the presenter's pointer and the Overview need it.
fn gnome_extension(running: Option<u32>) -> serde_json::Value {
    const UUID: &str = "vyoma@vyoma.dev";
    let detail = if running.is_some_and(|v| v >= vyoma_companion::shell::VERSION) {
        return json!({ "name": "GNOME extension", "state": "ok", "detail": "On" });
    } else if running.is_some() {
        "Updated. Log out and back in once to finish: until then, the newest touchpad gestures wait."
    } else if ![menus::data_home(), PathBuf::from("/usr/share")].into_iter().map(|d| d.join("gnome-shell/extensions").join(UUID)).any(|d| d.join("extension.js").exists()) {
        "Not installed. Reinstall Vyoma to add it: clipboard sync and the presenter's pointer need it."
    } else if std::process::Command::new("gsettings")
        .args(["get", "org.gnome.shell", "enabled-extensions"])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains(&format!("'{UUID}'")))
    {
        "Log out and back in once to turn it on: clipboard sync and the presenter's pointer need it."
    } else {
        "Off. Turn on Vyoma in GNOME's Extensions app: clipboard sync and the presenter's pointer need it."
    };
    json!({ "name": "GNOME extension", "state": "warn", "detail": detail })
}

fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// A [Control::send] notification; [replaces] the one before, so a send shows as one.
async fn sending(notifier: &Notifier, title: &str, body: &str, replaces: Option<u32>) -> Option<u32> {
    notifier.show(Notice { icon: "smartphone", title, body, actions: &[], sticky: false, replaces }).await.ok()
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
    for c in vyoma_companion::usb::clipboard() {
        (c.serial, c.digest).hash(&mut h);
    }
    h.finish()
}

/// Brings a mount up in the file manager: in GVfs for a GTK one (connected even when no window
/// opens, so it's in the sidebar), else its folder. [window]: open one.
fn show_files(m: &phonefiles::Mount, window: bool) {
    let path = m.path.to_string_lossy().into_owned();
    let Some(uri) = m.gvfs_uri() else {
        if window {
            share::open(&path);
        }
        return;
    };
    tokio::spawn(async move {
        let shown = match gvfs::mount(&uri).await {
            Ok(()) if window => gvfs::open(&uri).await,
            other => other,
        };
        if let Err(e) = shown {
            tracing::warn!("phone files in GVfs: {e:#}; opening the mount instead");
            if window {
                share::open(&path);
            }
        }
    });
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
            "incomingCapabilities": ([plugins::INCOMING, features::INCOMING, &[ECHO]].concat()),
            "outgoingCapabilities": ([plugins::OUTGOING, features::OUTGOING, &[ECHO_REQUEST]].concat()),
        }),
    )
}

/// Percent-encodes everything but letters, digits and -._~ (RFC 3986's unreserved).
fn url_encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

fn host_name() -> String {
    let name = std::fs::read_to_string("/proc/sys/kernel/hostname").unwrap_or_default();
    vyoma_core::lan::device_name(name.trim())
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_ufw_rules() {
        assert!(super::ufw_allows("### tuple ### allow tcp 1714:1764 0.0.0.0/0 any 0.0.0.0/0 in\n"));
        assert!(super::ufw_allows("### tuple ### allow any 1716 0.0.0.0/0 any 0.0.0.0/0 in\n"));
        assert!(super::ufw_allows("### tuple ### allow tcp 22,1700:1720 0.0.0.0/0 any 0.0.0.0/0 in\n"));
        assert!(!super::ufw_allows("### tuple ### allow udp 1714:1764 0.0.0.0/0 any 0.0.0.0/0 in\n"));
        assert!(!super::ufw_allows("### tuple ### deny tcp 1716 0.0.0.0/0 any 0.0.0.0/0 in\n"));
        assert!(!super::ufw_allows("### tuple ### allow tcp 22 0.0.0.0/0 any 0.0.0.0/0 in\n"));
    }

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
