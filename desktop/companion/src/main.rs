//! Vyoma companion: runs next to the stock KDE Connect daemon and gives Vyoma on the phone the
//! extras kdeconnectd can't: PC stats, screenshots, camera photos, handoff, file browsing, live screen view, PC controls (brightness, app volume, power), and zero-setup USB.
//!
//! It never listens for strangers: it only dials phones that kdeconnectd already trusts, found
//! via their mDNS announcements, and proves to them that it belongs to this PC by signing with
//! kdeconnectd's key. So there's nothing to pair by hand and nothing new to trust.

mod away;
mod bluetooth;
mod camera;
mod desktop;
mod files;
mod accessory;
mod gamepad;
mod identity;
mod kde;
mod link;
mod media;
mod mirror;
mod notice;
mod payload;
mod paste;
mod pc;
mod pointer;
mod screen;
mod screenshot;
mod setup;
mod stats;
mod tether;
mod usb;
mod specs;
mod wallpaper;
mod webcam;
mod xfer;

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::{Value, json};
use tracing::{info, warn};

use identity::Identity;
use kde::KdeHost;
use link::{Sender, packet};

const SERVICE: &str = "_kdeconnect._udp.local.";
/// How long a stats subscription lasts; the phone renews it while the dashboard is on screen.
const STATS_LEASE: Duration = Duration::from_secs(12);
const STATS_EVERY: Duration = Duration::from_secs(2);
/// How soon a phone unpaired in KDE Connect (lost or stolen, say) loses a session it already has.
const TRUST_CHECK: Duration = Duration::from_secs(5);

/// Which phones have a session, and which are being dialled. Kept apart: a dial that's still failing
/// (the old Wi-Fi address, long after Wi-Fi went) must not turn away a link that works.
#[derive(Default)]
struct Links {
    live: Mutex<HashSet<String>>,
    dialling: Mutex<HashSet<String>>,
}

impl Links {
    /// False if it's linked already or a dial is on its way.
    fn start_dial(&self, id: &str) -> bool {
        !self.live.lock().unwrap().contains(id) && self.dialling.lock().unwrap().insert(id.to_string())
    }

    fn dial_done(&self, id: &str) {
        self.dialling.lock().unwrap().remove(id);
    }

    /// The session is this link's, unless another got there first.
    fn claim(&self, id: &str) -> bool {
        self.live.lock().unwrap().insert(id.to_string())
    }

    fn release(&self, id: &str) {
        self.live.lock().unwrap().remove(id);
    }

    fn linked(&self) -> HashSet<String> {
        self.live.lock().unwrap().clone()
    }
}

struct Shared {
    host: KdeHost,
    me: Identity,
    links: Links,
    /// Last address each trusted phone announced, so we can redial without waiting for mDNS.
    seen: Mutex<HashMap<String, SocketAddr>>,
    /// Tracks what's playing for handoff; None without a session bus.
    media: Option<Arc<media::Media>>,
    /// Walk-away rules each phone asked for.
    away: Arc<away::Away>,
    /// Ends a phone's running session (see take_over).
    sessions: Mutex<HashMap<String, Arc<tokio::sync::Notify>>>,
}

fn main() -> Result<()> {
    // The phone's screen window runs as a child process (see mirror.rs).
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some(mirror::WINDOW_ARG) {
        return mirror::window(&args[2..]);
    }
    match args.get(1).map(String::as_str) {
        Some("--check") => std::process::exit(if setup::print_check() { 0 } else { 1 }),
        Some("--setup") => return setup::run(args.get(2).is_some_and(|a| a == "--dry-run")),
        _ => {}
    }
    daemon()
}

#[tokio::main]
async fn daemon() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "vyoma_companion=info".into()))
        .init();

    let host = KdeHost::load()?;
    let me = Identity::load_or_create()?;
    info!("companion {} for KDE Connect device {} ({})", me.device_id, host.device_id, host.name);
    let media = match media::Media::start().await {
        Ok(m) => Some(m),
        Err(e) => {
            warn!("handoff unavailable: {e:#}");
            None
        }
    };
    let shared = Arc::new(Shared { host, me, links: Links::default(), seen: Mutex::default(), media, away: Arc::default(), sessions: Mutex::default() });

    tokio::spawn(setup::offer());
    tokio::spawn(usb::watch());
    let (bluetooth_links, bluetooth_ports) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(bluetooth::serve(bluetooth_links));
    tokio::spawn(watch_bluetooth(shared.clone(), bluetooth_ports));
    tokio::spawn(gamepad::serve_wired());
    tokio::spawn(webcam::remove_stale_mics());
    tokio::spawn(watch_tether(shared.clone()));

    let mdns = mdns_sd::ServiceDaemon::new()?;
    let events = mdns.browse(SERVICE)?;
    let s = shared.clone();
    tokio::spawn(async move {
        while let Ok(event) = events.recv_async().await {
            if let mdns_sd::ServiceEvent::ServiceResolved(info) = event {
                let Some(id) = info.get_property_val_str("id").map(str::to_string) else { continue };
                let Some(ip) = pick_address(info.get_addresses_v4().into_iter().map(IpAddr::V4).collect()) else { continue };
                {
                    let mut seen = s.seen.lock().unwrap();
                    if !may_remember(&seen, &id, || s.host.trusted_phones().contains_key(&id)) {
                        continue;
                    }
                    seen.insert(id.clone(), SocketAddr::new(ip, info.get_port()));
                }
                dial(&s, &id);
            }
        }
    });

    // mDNS only reports changes; retry known phones that dropped off, e.g. after Wi-Fi roaming.
    let mut retry = tokio::time::interval(Duration::from_secs(30));
    loop {
        tokio::select! {
            _ = retry.tick() => {
                let ids: Vec<String> = shared.seen.lock().unwrap().keys().cloned().collect();
                for id in ids { dial(&shared, &id); }
            }
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    let _ = mdns.shutdown();
    Ok(())
}

fn pick_address(mut ips: Vec<IpAddr>) -> Option<IpAddr> {
    ips.sort_by_key(|ip| ip.is_loopback());
    ips.into_iter().find(|ip| !ip.is_loopback())
}

/// Starts a session with [id] unless one is running or kdeconnectd doesn't trust it.
fn dial(shared: &Arc<Shared>, id: &str) {
    dial_attempt(shared, id, 0);
}

/// After a session fails or drops (the app restarting, an update), redial a few times quickly
/// instead of waiting for the 30 s sweep: the phone is usually back within seconds.
fn dial_attempt(shared: &Arc<Shared>, id: &str, attempt: u32) {
    let trusted = shared.host.trusted_phones();
    let Some(cert) = trusted.get(id).cloned() else { return };
    let Some(addr) = shared.seen.lock().unwrap().get(id).copied() else { return };
    if !shared.links.start_dial(id) {
        return;
    }
    let (shared, id) = (shared.clone(), id.to_string());
    tokio::spawn(async move {
        // Once a round: with the phone off this network, the retries would say the same every few seconds.
        let result = session(&shared, &id, addr, &cert).await;
        // Something else lives at that address now: stop calling it until mDNS finds this phone again.
        if result.as_ref().is_err_and(|e| e.is::<link::NotTrusted>()) {
            info!("{id}: no longer at {addr}");
            forget_at(&mut shared.seen.lock().unwrap(), &id, addr);
            return;
        }
        if let Err(e) = result {
            if attempt == 0 {
                warn!("{id}: {e:#}");
            } else {
                tracing::debug!("{id}: {e:#}");
            }
        }
        // A clean end too: that's what an app restarting looks like.
        if attempt < 3 {
            tokio::time::sleep(Duration::from_secs(2 * (attempt as u64 + 1))).await;
            dial_attempt(&shared, &id, attempt + 1);
        }
    });
}

/// Forgets [id]'s address, unless mDNS has since found it somewhere else.
fn forget_at(seen: &mut HashMap<String, SocketAddr>, id: &str, addr: SocketAddr) {
    if seen.get(id) == Some(&addr) {
        seen.remove(id);
    }
}

/// Phones are remembered before they're trusted, so one paired later gets dialled. But anyone on the
/// network can advertise any number of ids, so that list stops growing once it's full.
fn may_remember(seen: &HashMap<String, SocketAddr>, id: &str, trusted: impl FnOnce() -> bool) -> bool {
    seen.len() < MAX_SEEN || seen.contains_key(id) || trusted()
}

fn identity_packet(shared: &Shared) -> Value {
    packet(
        "kdeconnect.identity",
        json!({
            "deviceId": shared.me.device_id,
            "deviceName": format!("{} companion", shared.host.name),
            "deviceType": "desktop",
            "protocolVersion": 8,
            "incomingCapabilities": ["vyoma.away", "vyoma.stats.request", "vyoma.screenshot.request", "vyoma.camera.request", "vyoma.handoff.request", "vyoma.files.list", "vyoma.files.get", "vyoma.files.put", "vyoma.screen.request", "vyoma.screen.stop", "vyoma.screen.input", "vyoma.screen.touch", "vyoma.pc.request", "vyoma.pc.set", "vyoma.pc.power", "vyoma.pc.wake", "vyoma.xfer.begin", "vyoma.xfer.data", "vyoma.xfer.end", "vyoma.wallpaper.request", "vyoma.specs.request", "vyoma.gamepad.state", "vyoma.gamepad.close", "vyoma.desktop.step", "vyoma.usb.accessory", "vyoma.webcam", "vyoma.webcam.mic", "vyoma.webcam.turn", "vyoma.mirror", "vyoma.mirror.audio", "vyoma.mirror.copy", "vyoma.mirror.volume", "vyoma.paste", "vyoma.pointer.request", "vyoma.pointer.close"],
            "outgoingCapabilities": ["vyoma.bluetooth", "vyoma.pointer.ready", "vyoma.paste.ready", "vyoma.stats", "vyoma.screenshot", "vyoma.camera", "vyoma.handoff", "vyoma.files.listing", "vyoma.files.file", "vyoma.files.saved", "vyoma.screen", "vyoma.pc", "vyoma.pc.powered", "vyoma.xfer.ready", "vyoma.xfer.done", "vyoma.wallpaper", "vyoma.specs", "vyoma.gamepad.rumble", "vyoma.gamepad.udp", "vyoma.desktop.stepped", "vyoma.webcam.state", "vyoma.mirror.input", "vyoma.mirror.state"],
            // Lets Vyoma show us as part of this PC instead of as a second computer.
            "vyomaHost": shared.host.device_id,
        }),
    )
}

const MAX_SEEN: usize = 64;
const TAKE_OVER_TRIES: u32 = 60;
const TAKE_OVER_WAIT: Duration = Duration::from_millis(50);
/// The port Vyoma listens on (KDE Connect's).
const PHONE_PORT: u16 = 1716;
const TETHER_POLL: Duration = Duration::from_secs(2);
/// After a tethered address didn't answer as a trusted phone, leave it this long.
const TETHER_BACKOFF: Duration = Duration::from_secs(30);
/// The same for the cable, which is only dialled while no phone is linked and costs next to nothing:
/// the app is usually back within seconds of restarting, and the phone holds Wi-Fi back only 4 s.
const CABLE_BACKOFF: Duration = Duration::from_secs(2);

/// Phones tethered over USB (see tether.rs): dialled when a trusted phone isn't linked yet. The
/// link is the same as over Wi-Fi, so everything works with no USB debugging.
async fn watch_tether(shared: Arc<Shared>) {
    let next_try: Arc<Mutex<HashMap<SocketAddr, Instant>>> = Arc::default();
    loop {
        tokio::time::sleep(TETHER_POLL).await;
        let mut candidates: Vec<(SocketAddr, tether::Via)> =
            tether::phones().into_iter().map(|(ip, via)| (SocketAddr::new(ip, PHONE_PORT), via)).collect();
        candidates.extend(usb::cable_ports().into_iter().map(|p| (SocketAddr::from(([127, 0, 0, 1], p)), tether::Via::Cable)));
        for (addr, via) in candidates {
            if next_try.lock().unwrap().get(&addr).is_some_and(|t| Instant::now() < *t) {
                continue;
            }
            let connected = shared.links.linked();
            // One phone to a PC: once it's linked, the cable only leads back to it, to be closed again.
            if via == tether::Via::Cable && !connected.is_empty() {
                continue;
            }
            let free: HashMap<String, Vec<u8>> = shared.host.trusted_phones().into_iter().filter(|(id, _)| !connected.contains(id)).collect();
            if free.is_empty() {
                continue;
            }
            let backoff = if via == tether::Via::Cable { CABLE_BACKOFF } else { TETHER_BACKOFF };
            next_try.lock().unwrap().insert(addr, Instant::now() + backoff);
            let (shared, next_try) = (shared.clone(), next_try.clone());
            tokio::spawn(async move {
                match link::connect_trusted(addr, &shared.me, &identity_packet(&shared), &free).await {
                    Ok((id, link)) => {
                        // A Wi-Fi dial may have won meanwhile: then this one just closes.
                        if !shared.links.claim(&id) {
                            return;
                        }
                        match via {
                            tether::Via::Usb => info!("{id}: found over USB tethering at {}", addr.ip()),
                            tether::Via::Hotspot => info!("{id}: found as this PC's hotspot at {}", addr.ip()),
                            tether::Via::Cable => info!("{id}: reached over the USB cable"),
                        }
                        // Not remembered for mDNS-style redials: an adb port is only good while plugged in.
                        if via != tether::Via::Cable {
                            shared.seen.lock().unwrap().insert(id.clone(), addr);
                        }
                        next_try.lock().unwrap().remove(&addr);
                        let _ = run(&shared, &id, addr, &free[&id], link).await.inspect_err(|e| warn!("{id}: {e:#}"));
                        shared.links.release(&id);
                    }
                    // A router (most Wi-Fi gateways) refuses, and a cable may lead to a phone this PC
                    // doesn't trust (an emulator): not worth a line every 30 s.
                    // Nor is the phone that's already linked some other way answering here.
                    Err(e) if via != tether::Via::Usb || e.is::<link::NotTrusted>() => tracing::debug!("{via:?} {addr}: {e:#}"),
                    Err(e) => info!("tethered {}: {e:#}", addr.ip()),
                }
            });
        }
    }
}

/// Phones that opened our Bluetooth service (see bluetooth.rs): each local port leads to one, which
/// answers like over any other link. Ends when the phone has a better way here again.
async fn watch_bluetooth(shared: Arc<Shared>, mut ports: tokio::sync::mpsc::UnboundedReceiver<u16>) {
    while let Some(port) = ports.recv().await {
        let shared = shared.clone();
        tokio::spawn(async move {
            let addr = SocketAddr::from(([127, 0, 0, 1], port));
            let trusted = shared.host.trusted_phones();
            match link::connect_trusted(addr, &shared.me, &identity_packet(&shared), &trusted).await {
                Ok((id, link)) => {
                    if !take_over(&shared, &id).await {
                        return;
                    }
                    info!("{id}: reached over Bluetooth");
                    let _ = run(&shared, &id, addr, &trusted[&id], link).await.inspect_err(|e| warn!("{id}: {e:#}"));
                    shared.links.release(&id);
                    dial(&shared, &id);
                }
                Err(e) => warn!("bluetooth companion link: {e:#}"),
            }
        });
    }
}

/// The phone only opens this tunnel once it has lost the companion, so a session still running here is
/// dead and just hasn't timed out yet (Wi-Fi gone: up to 30 s). End it and take its place. False if the
/// phone stays claimed anyway.
async fn take_over(shared: &Shared, id: &str) -> bool {
    for _ in 0..TAKE_OVER_TRIES {
        if shared.links.claim(id) {
            return true;
        }
        if let Some(session) = shared.sessions.lock().unwrap().get(id) {
            session.notify_one();
        }
        tokio::time::sleep(TAKE_OVER_WAIT).await;
    }
    false
}

async fn session(shared: &Arc<Shared>, phone_id: &str, addr: SocketAddr, pinned: &[u8]) -> Result<()> {
    let dialled = link::connect(addr, &shared.me, &identity_packet(shared), phone_id, pinned).await;
    shared.links.dial_done(phone_id);
    let link = dialled?;
    // The cable or Bluetooth may have won meanwhile: then this one just closes.
    if !shared.links.claim(phone_id) {
        return Ok(());
    }
    let result = run(shared, phone_id, addr, pinned, link).await;
    shared.links.release(phone_id);
    result
}

/// The phone picks its own name, and it ends up in notifications and logs (the protocol's rule).
fn phone_name(raw: &str) -> String {
    let clean: String = raw.chars().filter(|c| !matches!(c, '"' | '\'' | ',' | ';' | ':' | '.' | '!' | '?' | '(' | ')' | '[' | ']' | '<' | '>') && !c.is_control()).take(32).collect();
    let clean = clean.trim().trim_start_matches('-').trim();
    if clean.is_empty() { "Phone".to_string() } else { clean.to_string() }
}

async fn run(shared: &Arc<Shared>, phone_id: &str, addr: SocketAddr, pinned: &[u8], mut link: link::Link) -> Result<()> {
    let name = phone_name(link.phone_identity["body"]["deviceName"].as_str().unwrap_or(phone_id));
    info!("linked with {name} at {addr}");

    // Ask to pair with a proof signed by kdeconnectd's key; Vyoma accepts it without a prompt.
    let ts = link::now_ms() / 1000;
    let proof = shared.host.sign(&kde::proof_message(&shared.me.device_id, shared.me.cert.as_ref(), phone_id, ts));
    link.sender
        .send(&packet("kdeconnect.pair", json!({ "pair": true, "timestamp": ts, "vyomaHost": shared.host.device_id, "vyomaHostProof": proof })))
        .await?;

    shared.away.linked(phone_id, shared.media.as_ref()).await;
    let result = serve(shared, phone_id, addr, pinned, &mut link, &name).await;
    shared.away.unlinked(phone_id, shared.media.clone());
    result
}

async fn serve(shared: &Arc<Shared>, phone_id: &str, addr: SocketAddr, pinned: &[u8], link: &mut link::Link, name: &str) -> Result<()> {
    let name = name.to_string();
    let lease = Arc::new(Mutex::new(Instant::now()));
    let mut stats_task: Option<tokio::task::JoinHandle<()>> = None;
    let mut shot_task: Option<tokio::task::JoinHandle<()>> = None;
    let mut photo_task: Option<tokio::task::JoinHandle<()>> = None;
    let mut screen_task: Option<tokio::task::JoinHandle<()>> = None;
    // Touches from the phone's view of this screen, for the running cast.
    let mut screen_input: Option<tokio::sync::mpsc::UnboundedSender<Value>> = None;
    let mut touch_task: Option<tokio::task::JoinHandle<()>> = None;
    let mut webcam_task: Option<tokio::task::JoinHandle<()>> = None;
    let mut mic_task: Option<tokio::task::JoinHandle<()>> = None;
    let mut mirror_task: Option<tokio::task::JoinHandle<()>> = None;
    let mut sound_task: Option<tokio::task::JoinHandle<()>> = None;
    let (turn_tx, _) = tokio::sync::watch::channel(0u32);
    let pc = pc::worker(link.sender.clone(), name.clone());
    // Listings and downloads; dropping the set when the link closes cancels whatever is left.
    let mut file_tasks = tokio::task::JoinSet::new();
    // The phone as a controller: its pad lives while states keep coming (see gamepad.rs).
    let mut pad: Option<(tokio::sync::mpsc::Sender<gamepad::Input>, tokio::task::JoinHandle<()>)> = None;
    // The touchpad over Bluetooth, while its page is open on the phone.
    let mut touchpad: Option<(tokio::sync::mpsc::Sender<pointer::Frame>, tokio::task::JoinHandle<()>)> = None;
    // A new wallpaper reaches the phone's background without waiting for it to ask.
    let wall_task = tokio::spawn(watch_wallpaper(link.sender.clone()));
    // So the phone can find this PC over Bluetooth once Wi-Fi is gone.
    if let Some(address) = bluetooth::address().await {
        let _ = link.sender.send(&packet("vyoma.bluetooth", json!({ "address": address }))).await;
    }
    let mut trust_check = tokio::time::interval(TRUST_CHECK);
    let replaced = Arc::new(tokio::sync::Notify::new());
    shared.sessions.lock().unwrap().insert(phone_id.to_string(), replaced.clone());
    let result = loop {
        let next = tokio::select! {
            next = link.receiver.next() => next,
            _ = replaced.notified() => {
                info!("{name}: back over Bluetooth; ending the old link");
                break Ok(());
            }
            _ = trust_check.tick() => {
                if !still_trusted(&shared.host.trusted_phones(), phone_id, pinned) {
                    warn!("{name}: no longer paired in KDE Connect; ending its session");
                    break Ok(());
                }
                continue;
            }
        };
        let p = match next {
            Ok(Some(p)) => p,
            Ok(None) => break Ok(()),
            // Through the cleanup below too: the screen, camera and controller must end with the link.
            Err(e) => break Err(e),
        };
        while file_tasks.try_join_next().is_some() {}
        match p["type"].as_str().unwrap_or_default() {
            "kdeconnect.pair" => match p["body"]["pair"].as_bool() {
                Some(true) => info!("{name}: paired"),
                _ => warn!("{name}: declined pairing (needs Vyoma with companion support)"),
            },
            "vyoma.stats.request" => {
                *lease.lock().unwrap() = Instant::now() + STATS_LEASE;
                if stats_task.as_ref().is_none_or(|t| t.is_finished()) {
                    info!("{name}: streaming stats while the dashboard is open");
                    stats_task = Some(tokio::spawn(stream_stats(link.sender.clone(), lease.clone(), shared.media.clone())));
                }
            }
            "vyoma.screenshot.request" => {
                // One at a time; repeated taps while one is in flight are ignored.
                if shot_task.as_ref().is_none_or(|t| t.is_finished()) {
                    let (sender, shared, pinned, name) = (link.sender.clone(), shared.clone(), pinned.to_vec(), name.clone());
                    shot_task = Some(tokio::spawn(async move {
                        match screenshot::capture_and_send(&sender, &shared.me, &pinned, &name).await {
                            Ok(()) => info!("{name}: sent a screenshot"),
                            Err(e) => warn!("{name}: screenshot failed: {e:#}"),
                        }
                    }));
                }
            }
            "vyoma.camera.request" => {
                // One at a time, like screenshots.
                if photo_task.as_ref().is_none_or(|t| t.is_finished()) {
                    let (sender, shared, pinned, name) = (link.sender.clone(), shared.clone(), pinned.to_vec(), name.clone());
                    photo_task = Some(tokio::spawn(async move {
                        match camera::capture_and_send(&sender, &shared.me, &pinned, &name).await {
                            Ok(()) => info!("{name}: sent a camera photo"),
                            Err(e) => warn!("{name}: camera photo failed: {e:#}"),
                        }
                    }));
                }
            }
            "vyoma.screen.request" => {
                // A new request replaces a cast still running (the phone reopened the viewer).
                if let Some(t) = screen_task.take() {
                    t.abort();
                }
                let (sender, shared, pinned, name) = (link.sender.clone(), shared.clone(), pinned.to_vec(), name.clone());
                let (touches, inputs) = tokio::sync::mpsc::unbounded_channel();
                screen_input = Some(touches);
                let fps = screen::fps_for(p["body"]["maxFps"].as_u64());
                screen_task = Some(tokio::spawn(async move {
                    if let Err(e) = screen::stream(&sender, &shared.me, &pinned, &name, fps, inputs).await {
                        warn!("{name}: screen cast failed: {e:#}");
                    }
                }));
            }
            "vyoma.webcam" => {
                // A new stream replaces one still running (the phone switched cameras).
                if let Some(t) = webcam_task.take() {
                    t.abort();
                }
                turn_tx.send_replace(p["body"]["rotate"].as_u64().unwrap_or(0) as u32);
                let (sender, shared, pinned, name, phone, turn) = (link.sender.clone(), shared.clone(), pinned.to_vec(), name.clone(), addr.ip(), turn_tx.subscribe());
                webcam_task = Some(tokio::spawn(async move {
                    if let Err(e) = webcam::camera(&sender, &shared.me, &pinned, phone, &p, turn, &name).await {
                        warn!("{name}: phone camera: {e:#}");
                    }
                }));
            }
            "vyoma.mirror" => {
                // A new stream (the phone restarted it) replaces the window.
                if let Some(t) = mirror_task.take() {
                    t.abort();
                }
                let (sender, shared, pinned, name, phone) = (link.sender.clone(), shared.clone(), pinned.to_vec(), name.clone(), addr.ip());
                mirror_task = Some(tokio::spawn(async move {
                    if let Err(e) = mirror::show(&sender, &shared.me, &pinned, phone, &p, &name).await {
                        warn!("{name}: mirror: {e:#}");
                    }
                }));
            }
            "vyoma.mirror.copy" => {
                if let Some(text) = p["body"]["text"].as_str() {
                    let text = text.to_string();
                    tokio::spawn(async move { mirror::copy(&text).await });
                }
            }
            "vyoma.paste" => {
                if let Some(text) = p["body"]["text"].as_str() {
                    let (text, sender) = (text.to_string(), link.sender.clone());
                    tokio::spawn(async move {
                        let body = match paste::put(&text).await {
                            Ok(()) => json!({}),
                            Err(e) => json!({ "error": format!("{e:#}") }),
                        };
                        let _ = sender.send(&packet("vyoma.paste.ready", body)).await;
                    });
                }
            }
            "vyoma.mirror.volume" => {
                let up = p["body"]["up"].as_bool().unwrap_or(true);
                tokio::spawn(mirror::volume(up));
            }
            "vyoma.mirror.audio" => {
                if let Some(t) = sound_task.take() {
                    t.abort();
                }
                let (shared, pinned, name, phone) = (shared.clone(), pinned.to_vec(), name.clone(), addr.ip());
                sound_task = Some(tokio::spawn(async move {
                    if let Err(e) = mirror::sound(&shared.me, &pinned, phone, &p, &name).await {
                        warn!("{name}: phone sound: {e:#}");
                    }
                }));
            }
            "vyoma.webcam.turn" => {
                turn_tx.send_replace(p["body"]["rotate"].as_u64().unwrap_or(0) as u32);
            }
            "vyoma.webcam.mic" => {
                if let Some(t) = mic_task.take() {
                    t.abort();
                }
                let (sender, shared, pinned, name, phone) = (link.sender.clone(), shared.clone(), pinned.to_vec(), name.clone(), addr.ip());
                mic_task = Some(tokio::spawn(async move {
                    let _ = webcam::mic(&sender, &shared.me, &pinned, phone, &p, &name).await;
                }));
            }
            "vyoma.screen.touch" => {
                // The viewer's touches as a live stream (over the cable when it's in): one JSON line each.
                if let Some(t) = touch_task.take() {
                    t.abort();
                }
                if let Some(touches) = screen_input.clone() {
                    let (shared, pinned, phone) = (shared.clone(), pinned.to_vec(), addr.ip());
                    touch_task = Some(tokio::spawn(async move {
                        use tokio::io::AsyncBufReadExt;
                        let Ok(stream) = payload::fetch_stream(&shared.me, &pinned, phone, &p).await else { return };
                        info!("touch control stream open");
                        let mut lines = tokio::io::BufReader::new(stream).lines();
                        let (mut chars, mut keys, mut texts, mut biggest) = (0usize, 0usize, 0usize, 0usize);
                        while let Ok(Some(line)) = lines.next_line().await {
                            let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                            match v["kind"].as_str() {
                                Some("text") => {
                                    let n = v["text"].as_str().map_or(0, |t| t.chars().count());
                                    chars += n;
                                    texts += 1;
                                    biggest = biggest.max(n);
                                }
                                Some("key") => keys += 1,
                                _ => {}
                            }
                            if touches.send(v).is_err() {
                                break;
                            }
                        }
                        info!("touch control stream closed ({chars} characters typed in {texts} messages, at most {biggest} at once; {keys} keys)");
                    }));
                }
            }
            "vyoma.screen.input" => {
                if let Some(t) = &screen_input {
                    let _ = t.send(p["body"].clone());
                }
            }
            "vyoma.screen.stop" => {
                screen_input = None;
                if let Some(t) = touch_task.take() {
                    t.abort();
                }
                if let Some(t) = screen_task.take() {
                    t.abort();
                    info!("{name}: screen cast stopped");
                }
            }
            "vyoma.wallpaper.request" => {
                let sender = link.sender.clone();
                tokio::spawn(async move {
                    let reply = match wallpaper::colors().await {
                        Ok(c) => json!({ "cols": wallpaper::COLS, "rows": wallpaper::ROWS, "colors": c }),
                        Err(e) => json!({ "error": format!("{e:#}") }),
                    };
                    let _ = sender.send(&packet("vyoma.wallpaper", reply)).await;
                });
            }
            "vyoma.specs.request" => {
                let sender = link.sender.clone();
                tokio::spawn(async move {
                    let reply = tokio::task::spawn_blocking(specs::read).await.unwrap_or_else(|e| json!({ "error": e.to_string() }));
                    let _ = sender.send(&packet("vyoma.specs", reply)).await;
                });
            }
            "vyoma.gamepad.state" => {
                let s = gamepad::State::parse(&p["body"]);
                if pad.as_ref().is_none_or(|(_, t)| t.is_finished()) {
                    let (tx, rx) = tokio::sync::mpsc::channel(64);
                    pad = Some((tx, tokio::spawn(gamepad::run(rx, link.sender.clone(), name.clone()))));
                }
                if let Some((tx, _)) = &pad {
                    // Full means the PC is behind; the next state carries everything anyway.
                    let _ = tx.try_send(gamepad::Input::State(s));
                }
            }
            "vyoma.usb.accessory" => {
                // The phone's controller is on a cable without USB debugging: accessory mode.
                let name = name.clone();
                // "open": false means the phone couldn't open an accessory it already sees (its permission
                // went with a reinstall): switch it again so Android hands the accessory over afresh.
                let reswitch = !p["body"]["open"].as_bool().unwrap_or(false);
                tokio::task::spawn_blocking(move || {
                    if let Err(e) = accessory::start(reswitch) {
                        warn!("{name}: controller over the cable: {e:#}");
                    }
                });
            }
            "vyoma.desktop.step" => {
                let (sender, up, name) = (link.sender.clone(), p["body"]["up"].as_bool().unwrap_or(true), name.clone());
                tokio::spawn(async move {
                    let reply = match desktop::step(up).await {
                        Ok(view) => json!({ "view": view }),
                        Err(e) => {
                            warn!("{name}: desktop gesture failed: {e:#}");
                            json!({ "error": format!("{e:#}") })
                        }
                    };
                    let _ = sender.send(&packet("vyoma.desktop.stepped", reply)).await;
                });
            }
            "vyoma.pointer.request" => {
                // A new request replaces the last (the radio dropped and the phone came back).
                if let Some((tx, _)) = touchpad.take() {
                    let _ = tx.try_send(pointer::Frame::Close);
                }
                let (tx, rx) = tokio::sync::mpsc::channel(256);
                touchpad = Some((tx.clone(), tokio::spawn(pointer::serve(link.sender.clone(), name.clone(), tx, rx))));
            }
            "vyoma.pointer.close" => {
                if let Some((tx, _)) = touchpad.take() {
                    let _ = tx.try_send(pointer::Frame::Close);
                }
            }
            "vyoma.gamepad.close" => {
                if let Some((tx, _)) = &pad {
                    let _ = tx.try_send(gamepad::Input::Close);
                }
            }
            "vyoma.handoff.request" => {
                let reply = match &shared.media {
                    None => json!({ "error": "can't see the PC's media players" }),
                    Some(media) => match media.handoff().await {
                        None => json!({ "error": "nothing with a link is playing on the PC" }),
                        Some(h) => {
                            // The phone takes over: stop it here so they don't play over each other.
                            if p["body"]["pause"].as_bool().unwrap_or(true) {
                                media.pause(&h).await;
                            }
                            json!({ "url": h.url, "title": h.title, "position": h.position_ms, "exact": h.exact })
                        }
                    },
                };
                // Not `?`: leaving here skips the cleanup below, and a dead link ends the loop anyway.
                let _ = link.sender.send(&packet("vyoma.handoff", reply)).await;
            }
            "vyoma.files.list" => {
                let (sender, rel) = (link.sender.clone(), p["body"]["path"].as_str().unwrap_or_default().to_string());
                file_tasks.spawn(async move {
                    let _ = files::list(&sender, &rel).await;
                });
            }
            "vyoma.files.get" => {
                let (sender, shared, pinned, name) = (link.sender.clone(), shared.clone(), pinned.to_vec(), name.clone());
                let rel = p["body"]["path"].as_str().unwrap_or_default().to_string();
                file_tasks.spawn(async move {
                    match files::send(&sender, &shared.me, &pinned, &rel, &name).await {
                        Ok(()) => info!("{name}: sent a file"),
                        Err(e) => warn!("{name}: file transfer failed: {e:#}"),
                    }
                });
            }
            "vyoma.files.put" => {
                let (sender, shared, pinned, name, phone) = (link.sender.clone(), shared.clone(), pinned.to_vec(), name.clone(), addr.ip());
                file_tasks.spawn(async move {
                    match files::receive(&sender, &shared.me, &pinned, phone, &p, &name).await {
                        Ok(()) => info!("{name}: received a file"),
                        Err(e) => warn!("{name}: upload failed: {e:#}"),
                    }
                });
            }
            "vyoma.xfer.begin" => {
                let (sender, phone_id) = (link.sender.clone(), phone_id.to_string());
                file_tasks.spawn(async move {
                    if let Err(e) = xfer::begin(&sender, &phone_id, &p["body"]).await {
                        warn!("couldn't answer a transfer: {e:#}");
                    }
                });
            }
            "vyoma.xfer.data" => {
                let (sender, shared, pinned, name, phone) = (link.sender.clone(), shared.clone(), pinned.to_vec(), name.clone(), addr.ip());
                file_tasks.spawn(async move {
                    match xfer::data(&sender, &shared.me, &pinned, phone, &name, &p).await {
                        Ok(()) => info!("{name}: received a file"),
                        Err(e) => warn!("{name}: transfer stopped: {e:#}"),
                    }
                });
            }
            "vyoma.xfer.end" => xfer::end(&name, &p["body"]),
            "vyoma.away" => shared.away.configure(phone_id, &p["body"]),
            "vyoma.pc.request" | "vyoma.pc.set" | "vyoma.pc.power" | "vyoma.pc.wake" => {
                let _ = pc.send(p);
            }
            _ => {}
        }
    };
    wall_task.abort();
    // The phone's camera and mic go with its link (the mic's PipeWire source is removed on drop).
    for t in [webcam_task.take(), mic_task.take(), mirror_task.take(), sound_task.take()].into_iter().flatten() {
        t.abort();
    }
    // Lets go of whatever the touchpad held, then ends by itself.
    if let Some((tx, t)) = touchpad {
        if tx.try_send(pointer::Frame::Close).is_err() {
            t.abort();
        }
    }
    // Dropping the pad lets go of every button.
    if let Some((_, t)) = pad {
        t.abort();
    }
    for t in [shot_task, photo_task, screen_task, touch_task].into_iter().flatten() {
        t.abort();
    }
    if let Some(t) = stats_task {
        t.abort();
    }
    shared.sessions.lock().unwrap().remove(phone_id);
    info!("{name}: link closed");
    result
}

fn still_trusted(trusted: &HashMap<String, Vec<u8>>, phone_id: &str, pinned: &[u8]) -> bool {
    trusted.get(phone_id).is_some_and(|cert| cert.as_slice() == pinned)
}

/// Sends the wallpaper's colours whenever it changes (checked every few seconds; the phone asks for
/// the current one itself when it connects).
async fn watch_wallpaper(sender: Sender) {
    let mut last = wallpaper::signature();
    let mut tick = tokio::time::interval(Duration::from_secs(3));
    loop {
        tick.tick().await;
        let now = wallpaper::signature();
        if now == last {
            continue;
        }
        last = now;
        // Plasma writes the setting a moment before it's done switching; give it a breath.
        tokio::time::sleep(Duration::from_millis(400)).await;
        if let Ok(c) = wallpaper::colors().await {
            info!("wallpaper changed: sending its colours");
            if sender.send(&packet("vyoma.wallpaper", json!({ "cols": wallpaper::COLS, "rows": wallpaper::ROWS, "colors": c }))).await.is_err() {
                return;
            }
        }
    }
}

/// Whether the PC runs from a charger (mains or a USB-C power source); None with no such supply (a desktop).
fn on_ac() -> Option<bool> {
    let mut seen = false;
    for entry in std::fs::read_dir("/sys/class/power_supply").ok()?.flatten() {
        let read = |f: &str| std::fs::read_to_string(entry.path().join(f)).map(|s| s.trim().to_owned()).unwrap_or_default();
        if !matches!(read("type").as_str(), "Mains" | "USB") {
            continue;
        }
        seen = true;
        if read("online") == "1" {
            return Some(true);
        }
    }
    seen.then_some(false)
}

/// Stats, plus `handoff`: the title of a web video the phone could continue (absent otherwise),
/// so the phone only offers "Continue on phone" when it would work; and `camera`: whether a photo
/// can be taken (false behind a closed lid with no other camera); and `ac`: on a charger.
async fn stream_stats(sender: Sender, lease: Arc<Mutex<Instant>>, media: Option<Arc<media::Media>>) {
    let mut stats = match stats::Stats::new() {
        Ok(s) => s,
        Err(e) => return warn!("stats unavailable: {e:#}"),
    };
    let mut tick = tokio::time::interval(STATS_EVERY);
    while Instant::now() < *lease.lock().unwrap() {
        tick.tick().await;
        let mut body = stats.sample();
        if let Some(title) = media.as_ref().and_then(|m| m.offer()) {
            body["handoff"] = json!(title);
        }
        // So the phone can say "Camera unavailable" up front instead of after a failed tap.
        let camera = camera::available();
        body["camera"] = json!(camera);
        if !camera {
            body["cameraLid"] = json!(camera::behind_lid());
        }
        // Stock battery packets only say "charging": a laptop on its charger that holds at a
        // charge limit (or is full) reads as on battery without this.
        if let Some(ac) = on_ac() {
            body["ac"] = json!(ac);
        }
        if sender.send(&packet("vyoma.stats", body)).await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn phone_names_cant_become_options_or_markup() {
        assert_eq!(super::phone_name("--icon=x <b>Pixel</b>"), "icon=x bPixel/b");
        assert_eq!(super::phone_name("-- \n"), "Phone");
        assert_eq!(super::phone_name(&"a".repeat(99)).len(), 32);
    }

    use super::*;

    #[test]
    fn only_the_address_that_answered_wrongly_is_forgotten() {
        let (old, new): (SocketAddr, SocketAddr) = ("10.0.0.1:1716".parse().unwrap(), "10.0.0.2:1716".parse().unwrap());
        let mut seen = HashMap::from([("phone".to_string(), old)]);
        forget_at(&mut seen, "phone", new);
        assert_eq!(seen.get("phone"), Some(&old));
        forget_at(&mut seen, "phone", old);
        assert!(seen.is_empty());
    }

    #[test]
    fn strangers_cant_grow_the_list_of_phones_forever() {
        let addr = SocketAddr::from(([127, 0, 0, 1], 1716));
        let mut seen: HashMap<String, SocketAddr> = (0..MAX_SEEN - 1).map(|i| (format!("stranger{i}"), addr)).collect();
        assert!(may_remember(&seen, "new", || false));
        seen.insert("new".to_string(), addr);
        assert!(!may_remember(&seen, "another", || false));
        assert!(may_remember(&seen, "new", || false), "a known one can change address");
        assert!(may_remember(&seen, "paired", || true));
    }

    #[test]
    fn a_session_lasts_only_as_long_as_its_pairing() {
        let trusted = HashMap::from([("phone".to_string(), vec![1, 2, 3])]);
        assert!(still_trusted(&trusted, "phone", &[1, 2, 3]));
        assert!(!still_trusted(&trusted, "phone", &[9]));
        assert!(!still_trusted(&trusted, "other", &[1, 2, 3]));
        assert!(!still_trusted(&HashMap::new(), "phone", &[1, 2, 3]));
    }

    #[test]
    fn a_failing_dial_doesnt_turn_away_a_working_link() {
        let links = Links::default();
        assert!(links.start_dial("phone"));
        assert!(!links.start_dial("phone"), "one dial at a time");
        assert!(links.claim("phone"), "the cable links while the dial is still failing");
        links.dial_done("phone");
        assert!(!links.start_dial("phone"), "no dialling while it's linked");
        assert!(!links.claim("phone"), "one session at a time");
        links.release("phone");
        assert!(links.start_dial("phone"));
    }
}
