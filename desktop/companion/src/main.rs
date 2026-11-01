//! Vyoma companion: runs next to the stock KDE Connect daemon and gives Vyoma on the phone the
//! extras kdeconnectd can't: PC stats, screenshots, camera photos, handoff, file browsing, live screen view, PC controls (brightness, app volume, power), and zero-setup USB.
//!
//! It never listens for strangers: it only dials phones that kdeconnectd already trusts, found
//! via their mDNS announcements, and proves to them that it belongs to this PC by signing with
//! kdeconnectd's key. So there's nothing to pair by hand and nothing new to trust.

mod camera;
mod files;
mod identity;
mod kde;
mod link;
mod media;
mod payload;
mod pc;
mod screen;
mod screenshot;
mod stats;
mod usb;

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
    let shared = Arc::new(Shared { host, me, connected: Mutex::default(), seen: Mutex::default(), media });

    tokio::spawn(usb::watch());

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
            "incomingCapabilities": ["vyoma.stats.request", "vyoma.screenshot.request", "vyoma.camera.request", "vyoma.handoff.request", "vyoma.files.list", "vyoma.files.get", "vyoma.files.put", "vyoma.screen.request", "vyoma.screen.stop", "vyoma.pc.request", "vyoma.pc.set", "vyoma.pc.power"],
            "outgoingCapabilities": ["vyoma.stats", "vyoma.screenshot", "vyoma.camera", "vyoma.handoff", "vyoma.files.listing", "vyoma.files.file", "vyoma.files.saved", "vyoma.screen", "vyoma.pc", "vyoma.pc.powered"],
            // Lets Vyoma show us as part of this PC instead of as a second computer.
            "vyomaHost": shared.host.device_id,
        }),
    )
}

async fn session(shared: &Arc<Shared>, phone_id: &str, addr: SocketAddr, pinned: &[u8]) -> Result<()> {
    let mut link = link::connect(addr, &shared.me, &identity_packet(shared), phone_id, pinned).await?;
    let name = link.phone_identity["body"]["deviceName"].as_str().unwrap_or(phone_id).to_string();
    info!("linked with {name} at {addr}");

    // Ask to pair with a proof signed by kdeconnectd's key; Vyoma accepts it without a prompt.
    let ts = link::now_ms() / 1000;
    let proof = shared.host.sign(&kde::proof_message(&shared.me.device_id, shared.me.cert.as_ref(), phone_id, ts));
    link.sender
        .send(&packet("kdeconnect.pair", json!({ "pair": true, "timestamp": ts, "vyomaHost": shared.host.device_id, "vyomaHostProof": proof })))
        .await?;

    let lease = Arc::new(Mutex::new(Instant::now()));
    let mut stats_task: Option<tokio::task::JoinHandle<()>> = None;
    let mut shot_task: Option<tokio::task::JoinHandle<()>> = None;
    let mut photo_task: Option<tokio::task::JoinHandle<()>> = None;
    let mut screen_task: Option<tokio::task::JoinHandle<()>> = None;
    let pc = pc::worker(link.sender.clone(), name.clone());
    // Listings and downloads; dropping the set when the link closes cancels whatever is left.
    let mut file_tasks = tokio::task::JoinSet::new();
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
            "vyoma.pc.request" | "vyoma.pc.set" | "vyoma.pc.power" => {
                let _ = pc.send(p);
            }
            _ => {}
        }
    };
    for t in [shot_task, photo_task, screen_task].into_iter().flatten() {
        t.abort();
    }
    if let Some(t) = stats_task {
        t.abort();
    }
    info!("{name}: link closed");
    result
}

/// Stats, plus `handoff`: the title of a web video the phone could continue (absent otherwise),
/// so the phone only offers "Continue on phone" when it would work.
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
        if sender.send(&packet("vyoma.stats", body)).await.is_err() {
            return;
        }
    }
}
