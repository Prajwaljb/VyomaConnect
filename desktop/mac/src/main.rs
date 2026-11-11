//! Vyoma's PC side on macOS (docs/PLAN.md section 10, phase 5), on the same core as the Linux app:
//! it finds and links with phones, pairs after asking (a dialog with the code to compare), and
//! serves the stock plugins: ping, battery both ways, find my phone, the screen lock, the clipboard
//! both ways and sharing (files into ~/Downloads, links opened, text onto the clipboard).
//!
//! No window yet, and none of Vyoma's extras: to the phone this is a stock KDE Connect PC.
//! Built and tested on GitHub's macOS runners only; not yet tried on a real Mac.

mod launchd;
mod mac;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, bail};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use vyoma_core::clipboard::{CLIPBOARD, CLIPBOARD_CONNECT, Clipboard, MAX_TEXT};
use vyoma_core::device::{Command, Event, Hub};
use vyoma_core::identity::Identity;
use vyoma_core::lan::{Lan, Ports, Rules, UDP_PORT};
use vyoma_core::link::packet;
use vyoma_core::share::{self, SHARE, SHARE_UPDATE, Shared};
use vyoma_core::trust::TrustStore;

pub const PING: &str = "kdeconnect.ping";
pub const BATTERY: &str = "kdeconnect.battery";
pub const FIND: &str = "kdeconnect.findmyphone.request";
pub const LOCK_REQUEST: &str = "kdeconnect.lock.request";

/// What we take and send. No lock state outward: macOS doesn't tell other apps whether it's locked.
const INCOMING: &[&str] = &[PING, BATTERY, FIND, LOCK_REQUEST, CLIPBOARD, CLIPBOARD_CONNECT, SHARE, SHARE_UPDATE];
const OUTGOING: &[&str] = &[PING, BATTERY, CLIPBOARD, CLIPBOARD_CONNECT, SHARE, SHARE_UPDATE];

/// How often the clipboard is looked at (macOS sends no change events to a program like this one)
/// and the battery read.
const CLIPBOARD_EVERY: Duration = Duration::from_secs(1);
const BATTERY_EVERY: Duration = Duration::from_secs(60);
/// How long a pairing question stays up: the phone gives up after 30 s too.
const PAIR_TIMEOUT: Duration = Duration::from_secs(30);

fn main() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("--install") => launchd::install(),
        Some("--uninstall") => launchd::uninstall(),
        Some(other) => bail!("unknown option {other}: try --install or --uninstall"),
        None => run(std::env::var_os("VYOMA_DIR").map(PathBuf::from).unwrap_or_else(launchd::support_dir)),
    }
}

/// A device as this Mac knows it.
#[derive(Default)]
struct Device {
    name: String,
    paired: bool,
    linked: bool,
    incoming: Vec<String>,
    cert: Vec<u8>,
    /// The address of its newest link, where its payloads are fetched.
    remote: Option<IpAddr>,
    /// Its battery: charge, charging, and whether it said it's low.
    battery: Option<(i64, bool, bool)>,
}

impl Device {
    fn takes(&self, kind: &str) -> bool {
        self.paired && self.linked && self.incoming.iter().any(|t| t == kind)
    }
}

#[tokio::main]
async fn run(dir: PathBuf) -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "vyoma_mac=info,vyoma_core=info".into()))
        .init();
    vyoma_core::identity::use_dir(dir.clone());
    let me = Arc::new(Identity::load_or_create_in(&dir)?);
    let trust = TrustStore::open(&dir)?;
    let mut devices: HashMap<String, Device> = trust
        .ids()
        .map(|id| (id.clone(), Device { name: trust.get(id).unwrap().name.clone(), paired: true, ..Default::default() }))
        .collect();

    let mut battery = mac::battery().await;
    let (events_tx, mut events) = mpsc::unbounded_channel();
    let hub = Hub::new(me.cert.to_vec(), trust, events_tx);
    let linked = hub.linked();
    let rules = Rules { admits: Box::new(|_| true), linked: Box::new(move |id| linked.lock().unwrap().contains(id)) };
    let lan = Lan::bind(me.clone(), identity(&me, mac::name().await, battery.is_some()), rules, Ports::default()).await?;
    let (links_tx, links) = mpsc::channel(16);
    let (commands, commands_rx) = mpsc::unbounded_channel();
    tokio::spawn(lan.clone().run(links_tx));
    tokio::spawn(hub.run(links, commands_rx));
    let everyone = [SocketAddr::from(([255, 255, 255, 255], UDP_PORT))];
    lan.announce(&everyone).await?;
    tracing::info!("vyoma-mac is up as {}", me.device_id);

    let send = |id: &str, p: Value| {
        let _ = commands.send(Command::Send(id.to_owned(), p));
    };

    // The clipboard, looked at every second; only a change goes on.
    let (copies_tx, mut copies) = mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        let mut last = mac::clipboard().await;
        let mut tick = tokio::time::interval(CLIPBOARD_EVERY);
        loop {
            tick.tick().await;
            let now = mac::clipboard().await;
            if now != last {
                last = now.clone();
                if let Some(text) = now.filter(|t| t.len() <= MAX_TEXT)
                    && copies_tx.send(text).is_err()
                {
                    break;
                }
            }
        }
    });
    let mut clip = Clipboard::default();
    let mut battery_tick = tokio::time::interval(BATTERY_EVERY);
    battery_tick.tick().await;

    // Answers to the pairing questions on screen, and the Found it of a ring.
    let (answers_tx, mut answers) = mpsc::unbounded_channel::<(String, bool)>();
    let mut prompts: HashMap<String, tokio::task::JoinHandle<()>> = HashMap::new();
    let (found_tx, mut found) = mpsc::unbounded_channel::<()>();
    let mut ringer = mac::Ringer::default();

    loop {
        tokio::select! {
            Some(event) = events.recv() => match event {
                Event::Linked { id, name, paired, incoming, cert, remote, .. } => {
                    let d = devices.entry(id.clone()).or_default();
                    *d = Device { name, paired, linked: true, incoming, cert, remote: Some(remote), battery: d.battery };
                    tracing::info!("{id}: linked{}", if paired { ", paired" } else { "" });
                    if paired {
                        for p in started(d, battery, &clip) {
                            send(&id, p);
                        }
                    }
                }
                Event::Gone { id } => {
                    if let Some(d) = devices.get_mut(&id) {
                        d.linked = false;
                    }
                }
                Event::Routes { id, remotes } => {
                    if let Some(d) = devices.get_mut(&id) {
                        d.remote = remotes.last().copied();
                    }
                }
                Event::Prompt { id, name, key } => {
                    let answers = answers_tx.clone();
                    let asked = id.clone();
                    let text = format!("{name} wants to pair with this Mac.\n\nPair only if the phone shows this same code:\n\n{key}");
                    let task = tokio::spawn(async move {
                        let yes = match mac::ask("Vyoma", &text, "Reject", "Pair", PAIR_TIMEOUT) {
                            Ok(child) => child.wait_with_output().await.is_ok_and(|out| mac::answered_yes(&out.stdout, "Pair")),
                            Err(e) => {
                                tracing::warn!("couldn't ask about pairing: {e:#}");
                                false
                            }
                        };
                        let _ = answers.send((asked, yes));
                    });
                    // A second request replaces the first question.
                    if let Some(old) = prompts.insert(id, task) {
                        old.abort();
                    }
                }
                // Over before it was answered (the phone gave up): aborting drops the dialog's process.
                Event::PairingOver { id } => {
                    if let Some(task) = prompts.remove(&id) {
                        task.abort();
                    }
                }
                Event::Paired { id, name, .. } => {
                    let d = devices.entry(id.clone()).or_default();
                    d.paired = true;
                    d.name = name.clone();
                    mac::notify(&format!("Paired with {name}"), "Your phone and this Mac can now share files, links and the clipboard.").await;
                    for p in started(d, battery, &clip) {
                        send(&id, p);
                    }
                }
                Event::Unpaired { id, name } => {
                    if let Some(d) = devices.get_mut(&id) {
                        d.paired = false;
                    }
                    mac::notify(&format!("{name} is no longer paired"), "Pair again from the phone to use it with this Mac.").await;
                }
                Event::Asked { .. } => {}
                Event::Packet { id, packet, from } => {
                    let Some(d) = devices.get_mut(&id) else { continue };
                    for reply in on_packet(d, &packet, from, &me, &mut clip, &mut ringer, &found_tx).await {
                        send(&id, reply);
                    }
                }
            },
            Some((id, yes)) = answers.recv() => {
                prompts.remove(&id);
                let _ = commands.send(if yes { Command::Accept(id) } else { Command::Reject(id) });
            }
            Some(()) = found.recv() => ringer.stop(),
            Some(text) = copies.recv() => {
                if let Some(p) = clip.changed(text) {
                    for (id, d) in &devices {
                        if d.takes(CLIPBOARD) {
                            send(id, p.clone());
                        }
                    }
                }
            }
            _ = battery_tick.tick() => {
                let now = mac::battery().await;
                if now != battery && let Some(b) = now {
                    for (id, d) in &devices {
                        if d.takes(BATTERY) {
                            send(id, battery_packet(b));
                        }
                    }
                }
                battery = now;
            }
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    Ok(())
}

fn identity(me: &Identity, name: String, laptop: bool) -> Value {
    packet(
        "kdeconnect.identity",
        json!({
            "deviceId": me.device_id,
            "deviceName": name,
            "deviceType": if laptop { "laptop" } else { "desktop" },
            "protocolVersion": 8,
            "incomingCapabilities": INCOMING,
            "outgoingCapabilities": OUTGOING,
        }),
    )
}

fn battery_packet(b: mac::Battery) -> Value {
    packet(BATTERY, json!({ "currentCharge": b.charge, "isCharging": b.charging, "thresholdEvent": 0 }))
}

/// What a paired device that just linked is sent: this Mac's battery and its clipboard.
fn started(d: &Device, battery: Option<mac::Battery>, clip: &Clipboard) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(b) = battery.filter(|_| d.takes(BATTERY)) {
        out.push(battery_packet(b));
    }
    if let Some(p) = clip.connect_packet().filter(|_| d.takes(CLIPBOARD_CONNECT)) {
        out.push(p);
    }
    out
}

/// Handles one packet from a paired device; what to send back.
async fn on_packet(d: &mut Device, p: &Value, from: IpAddr, me: &Arc<Identity>, clip: &mut Clipboard, ringer: &mut mac::Ringer, found: &mpsc::UnboundedSender<()>) -> Vec<Value> {
    let body = &p["body"];
    match p["type"].as_str().unwrap_or("") {
        PING => mac::notify(&d.name, body["message"].as_str().unwrap_or("Ping!")).await,
        BATTERY => {
            let charge = body["currentCharge"].as_i64().or_else(|| body["currentCharge"].as_str()?.parse().ok()).unwrap_or(-1);
            let was_low = d.battery.is_some_and(|b| b.2);
            d.battery = (0..=100).contains(&charge).then(|| (charge, body["isCharging"].as_bool().unwrap_or(false), body["thresholdEvent"].as_i64() == Some(1)));
            if let Some((charge, _, true)) = d.battery
                && !was_low
            {
                mac::notify(&format!("{}'s battery is low", d.name), &format!("{charge}% left")).await;
            }
        }
        FIND => {
            ringer.start();
            let (found, text) = (found.clone(), format!("{} is looking for this Mac.", d.name));
            tokio::spawn(async move {
                if let Ok(child) = mac::ask("Vyoma", &text, "Stop", "Found it", Duration::from_secs(60)) {
                    let _ = child.wait_with_output().await;
                }
                let _ = found.send(());
            });
        }
        LOCK_REQUEST => {
            if body["setLocked"].as_bool() == Some(true) {
                match mac::sleep_display().await {
                    Ok(()) => tracing::info!("{}: locked this Mac", d.name),
                    Err(e) => tracing::warn!("{}: {e:#}", d.name),
                }
            }
        }
        CLIPBOARD | CLIPBOARD_CONNECT => {
            if let Some(text) = clip.received(p)
                && let Err(e) = mac::set_clipboard(&text).await
            {
                tracing::warn!("couldn't set the clipboard: {e:#}");
            }
        }
        SHARE => match share::classify(p) {
            Shared::File { name, .. } => {
                let (me, cert, phone, packet) = (me.clone(), d.cert.clone(), d.name.clone(), p.clone());
                let from = d.remote.unwrap_or(from);
                tokio::spawn(async move {
                    match share::receive(&me, &cert, from, &packet, &mac::downloads()).await {
                        Ok(path) => {
                            let saved = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(name);
                            mac::notify(&format!("From {phone}"), &format!("{saved} is in Downloads")).await;
                        }
                        Err(e) => {
                            tracing::warn!("{name} from {phone}: {e:#}");
                            mac::notify(&format!("{name} didn't arrive"), "The phone stopped sending it. Try again.").await;
                        }
                    }
                });
            }
            Shared::Url(url) => mac::open(&url),
            Shared::Text(text) => {
                if mac::set_clipboard(&text).await.is_ok() {
                    // Ours, so the poll doesn't send it straight back.
                    clip.changed(text);
                    mac::notify(&format!("Text from {}", d.name), "Copied. Paste it anywhere.").await;
                }
            }
            Shared::Nothing => {}
        },
        SHARE_UPDATE => {}
        other => tracing::debug!("{}: nothing here for {other}", d.name),
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phone(incoming: &[&str]) -> Device {
        Device { paired: true, linked: true, incoming: incoming.iter().map(|s| s.to_string()).collect(), ..Default::default() }
    }

    #[test]
    fn a_linked_phone_gets_the_battery_and_clipboard_it_takes() {
        let b = Some(mac::Battery { charge: 70, charging: true });
        let mut clip = Clipboard::default();
        assert!(started(&phone(&[BATTERY, CLIPBOARD_CONNECT]), None, &clip).is_empty(), "a desktop Mac with nothing copied");
        clip.changed("hello".into());
        let kinds: Vec<_> = started(&phone(&[BATTERY, CLIPBOARD_CONNECT]), b, &clip).iter().map(|p| p["type"].as_str().unwrap().to_owned()).collect();
        assert_eq!(kinds, [BATTERY, CLIPBOARD_CONNECT]);
        assert!(started(&phone(&[PING]), b, &clip).is_empty(), "only what it takes");
        assert!(started(&Device { paired: false, ..phone(&[BATTERY]) }, b, &clip).is_empty(), "nothing before pairing");
    }

    #[test]
    fn it_says_what_it_does() {
        let dir = std::env::temp_dir().join(format!("vyoma-mac-id-{}", std::process::id()));
        let me = Identity::load_or_create_in(&dir).unwrap();
        let p = identity(&me, "Sam's MacBook".into(), true);
        assert_eq!(p["body"]["deviceType"], "laptop");
        assert_eq!(p["body"]["incomingCapabilities"].as_array().unwrap().len(), INCOMING.len());
        assert!(!OUTGOING.contains(&"kdeconnect.lock"), "no lock state it can't know");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
