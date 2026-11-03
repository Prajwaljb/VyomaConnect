//! Vyoma companion: runs next to the stock KDE Connect daemon and gives Vyoma on the phone the
//! extras kdeconnectd can't: PC stats, screenshots, camera photos, handoff, file browsing, live screen view, PC controls (brightness, app volume, power), and zero-setup USB.
//!
//! It never listens for strangers: it only dials phones that kdeconnectd already trusts, found
//! via their mDNS announcements, and proves to them that it belongs to this PC by signing with
//! kdeconnectd's key. So there's nothing to pair by hand and nothing new to trust.

mod away;
mod camera;
mod desktop;
mod files;
mod gamepad;
mod identity;
mod kde;
mod link;
mod media;
mod payload;
mod pc;
mod screen;
mod screenshot;
mod stats;
mod tether;
mod usb;
mod wallpaper;
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

struct Shared {
    host: KdeHost,
    me: Identity,
    connected: Mutex<HashSet<String>>,
    /// Last address each trusted phone announced, so we can redial without waiting for mDNS.
    seen: Mutex<HashMap<String, SocketAddr>>,
    /// Tracks what's playing for handoff; None without a session bus.
    media: Option<Arc<media::Media>>,
    /// Walk-away rules each phone asked for.
    away: Arc<away::Away>,
}

#[tokio::main]
async fn main() -> Result<()> {
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
    let shared = Arc::new(Shared { host, me, connected: Mutex::default(), seen: Mutex::default(), media, away: Arc::default() });

    tokio::spawn(usb::watch());
    tokio::spawn(gamepad::serve_wired());
    tokio::spawn(watch_tether(shared.clone()));

    let mdns = mdns_sd::ServiceDaemon::new()?;
    let events = mdns.browse(SERVICE)?;
    let s = shared.clone();
    tokio::spawn(async move {
        while let Ok(event) = events.recv_async().await {
            if let mdns_sd::ServiceEvent::ServiceResolved(info) = event {
                let Some(id) = info.get_property_val_str("id").map(str::to_string) else { continue };
                let Some(ip) = pick_address(info.get_addresses_v4().into_iter().map(IpAddr::V4).collect()) else { continue };
                s.seen.lock().unwrap().insert(id.clone(), SocketAddr::new(ip, info.get_port()));
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
    if !shared.connected.lock().unwrap().insert(id.to_string()) {
        return;
    }
    let (shared, id) = (shared.clone(), id.to_string());
    tokio::spawn(async move {
        let failed = session(&shared, &id, addr, &cert).await.inspect_err(|e| warn!("{id}: {e:#}")).is_err();
        shared.connected.lock().unwrap().remove(&id);
        if failed && attempt < 3 {
            tokio::time::sleep(Duration::from_secs(2 * (attempt as u64 + 1))).await;
            dial_attempt(&shared, &id, attempt + 1);
        }
    });
}

fn identity_packet(shared: &Shared) -> Value {
    packet(
        "kdeconnect.identity",
        json!({
            "deviceId": shared.me.device_id,
            "deviceName": format!("{} companion", shared.host.name),
            "deviceType": "desktop",
            "protocolVersion": 8,
            "incomingCapabilities": ["vyoma.stats.request", "vyoma.screenshot.request", "vyoma.camera.request", "vyoma.handoff.request", "vyoma.files.list", "vyoma.files.get", "vyoma.files.put", "vyoma.screen.request", "vyoma.screen.stop", "vyoma.pc.request", "vyoma.pc.set", "vyoma.pc.power", "vyoma.pc.wake", "vyoma.xfer.begin", "vyoma.xfer.data", "vyoma.xfer.end", "vyoma.wallpaper.request", "vyoma.gamepad.state", "vyoma.gamepad.close", "vyoma.desktop.step"],
            "outgoingCapabilities": ["vyoma.stats", "vyoma.screenshot", "vyoma.camera", "vyoma.handoff", "vyoma.files.listing", "vyoma.files.file", "vyoma.files.saved", "vyoma.screen", "vyoma.pc", "vyoma.pc.powered", "vyoma.xfer.ready", "vyoma.xfer.done", "vyoma.wallpaper", "vyoma.gamepad.rumble", "vyoma.gamepad.udp", "vyoma.desktop.stepped"],
            // Lets Vyoma show us as part of this PC instead of as a second computer.
            "vyomaHost": shared.host.device_id,
        }),
    )
}

/// The port Vyoma listens on (KDE Connect's).
const PHONE_PORT: u16 = 1716;
const TETHER_POLL: Duration = Duration::from_secs(5);
/// After a tethered address didn't answer as a trusted phone, leave it this long.
const TETHER_BACKOFF: Duration = Duration::from_secs(30);

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
            let connected = shared.connected.lock().unwrap().clone();
            let free: HashMap<String, Vec<u8>> = shared.host.trusted_phones().into_iter().filter(|(id, _)| !connected.contains(id)).collect();
            if free.is_empty() {
                continue;
            }
            next_try.lock().unwrap().insert(addr, Instant::now() + TETHER_BACKOFF);
            let (shared, next_try) = (shared.clone(), next_try.clone());
            tokio::spawn(async move {
                match link::connect_trusted(addr, &shared.me, &identity_packet(&shared), &free).await {
                    Ok((id, link)) => {
                        // A Wi-Fi dial may have won meanwhile: then this one just closes.
                        if !shared.connected.lock().unwrap().insert(id.clone()) {
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
                        shared.connected.lock().unwrap().remove(&id);
                    }
                    // A router (most Wi-Fi gateways) refuses, and a cable may lead to a phone this PC
                    // doesn't trust (an emulator): not worth a line every 30 s.
                    Err(e) if via != tether::Via::Usb => tracing::debug!("{via:?} {addr}: {e:#}"),
                    Err(e) => info!("tethered {}: {e:#}", addr.ip()),
                }
            });
        }
    }
}

async fn session(shared: &Arc<Shared>, phone_id: &str, addr: SocketAddr, pinned: &[u8]) -> Result<()> {
    let link = link::connect(addr, &shared.me, &identity_packet(shared), phone_id, pinned).await?;
    run(shared, phone_id, addr, pinned, link).await
}

async fn run(shared: &Arc<Shared>, phone_id: &str, addr: SocketAddr, pinned: &[u8], mut link: link::Link) -> Result<()> {
    let name = link.phone_identity["body"]["deviceName"].as_str().unwrap_or(phone_id).to_string();
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
    let pc = pc::worker(link.sender.clone(), name.clone());
    // Listings and downloads; dropping the set when the link closes cancels whatever is left.
    let mut file_tasks = tokio::task::JoinSet::new();
    // The phone as a controller: its pad lives while states keep coming (see gamepad.rs).
    let mut pad: Option<(tokio::sync::mpsc::Sender<gamepad::Input>, tokio::task::JoinHandle<()>)> = None;
    // A new wallpaper reaches the phone's background without waiting for it to ask.
    let wall_task = tokio::spawn(watch_wallpaper(link.sender.clone()));
    let result = loop {
        let Some(p) = link.receiver.next().await? else { break Ok(()) };
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
                        match screenshot::capture_and_send(&sender, &shared.me, &pinned).await {
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
                screen_task = Some(tokio::spawn(async move {
                    if let Err(e) = screen::stream(&sender, &shared.me, &pinned, &name).await {
                        warn!("{name}: screen cast failed: {e:#}");
                    }
                }));
            }
            "vyoma.screen.stop" => {
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
                link.sender.send(&packet("vyoma.handoff", reply)).await?;
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
                    match files::send(&sender, &shared.me, &pinned, &rel).await {
                        Ok(()) => info!("{name}: sent a file"),
                        Err(e) => warn!("{name}: file transfer failed: {e:#}"),
                    }
                });
            }
            "vyoma.files.put" => {
                let (sender, shared, pinned, name, phone) = (link.sender.clone(), shared.clone(), pinned.to_vec(), name.clone(), addr.ip());
                file_tasks.spawn(async move {
                    match files::receive(&sender, &shared.me, &pinned, phone, &p).await {
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
    // Dropping the pad lets go of every button.
    if let Some((_, t)) = pad {
        t.abort();
    }
    for t in [shot_task, photo_task, screen_task].into_iter().flatten() {
        t.abort();
    }
    if let Some(t) = stats_task {
        t.abort();
    }
    info!("{name}: link closed");
    result
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
        body["camera"] = json!(camera::available());
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
