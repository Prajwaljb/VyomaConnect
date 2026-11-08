//! Vyoma companion: runs next to the stock KDE Connect daemon and gives Vyoma on the phone the
//! extras kdeconnectd can't: PC stats, screenshots, camera photos, handoff, file browsing, live screen view, PC controls (brightness, app volume, power), and zero-setup USB.
//!
//! It never listens for strangers: it only dials phones that kdeconnectd already trusts, found
//! via their mDNS announcements, and proves to them that it belongs to this PC by signing with
//! kdeconnectd's key. So there's nothing to pair by hand and nothing new to trust.


use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::{Value, json};
use tracing::{info, warn};

use vyoma_companion::session::{self, Host, Phone, phone_name};
use vyoma_companion::{bluetooth, mirror, gamepad, kde, media, setup, tether, usb, webcam};
use vyoma_core::{identity, link};

use identity::Identity;
use kde::KdeHost;
use link::packet;

const SERVICE: &str = "_kdeconnect._udp.local.";
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
    /// Its features, which each phone's session serves.
    features: Arc<Host>,
    links: Links,
    /// Last address each trusted phone announced, so we can redial without waiting for mDNS.
    seen: Mutex<HashMap<String, SocketAddr>>,
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
    let shared = Arc::new(Shared { host, features: Arc::new(Host { me: Arc::new(me), media, away: Arc::default(), permits: Arc::default() }), links: Links::default(), seen: Mutex::default(), sessions: Mutex::default() });

    tokio::spawn(setup::offer());
    tokio::spawn(usb::watch());
    let (bluetooth_links, bluetooth_ports) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(bluetooth::serve(Some(bluetooth_links)));
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
            "deviceId": shared.features.me.device_id,
            "deviceName": format!("{} companion", shared.host.name),
            "deviceType": "desktop",
            "protocolVersion": 8,
            "incomingCapabilities": session::INCOMING,
            "outgoingCapabilities": session::OUTGOING,
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
                match link::connect_trusted(addr, &shared.features.me, &identity_packet(&shared), &free).await {
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
            match link::connect_trusted(addr, &shared.features.me, &identity_packet(&shared), &trusted).await {
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
    let dialled = link::connect(addr, &shared.features.me, &identity_packet(shared), phone_id, pinned).await;
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

async fn run(shared: &Arc<Shared>, phone_id: &str, addr: SocketAddr, pinned: &[u8], mut link: link::Link) -> Result<()> {
    let name = phone_name(link.phone_identity["body"]["deviceName"].as_str().unwrap_or(phone_id));
    info!("linked with {name} at {addr}");

    // Ask to pair with a proof signed by kdeconnectd's key; Vyoma accepts it without a prompt.
    let ts = link::now_ms() / 1000;
    let proof = shared.host.sign(&kde::proof_message(&shared.features.me.device_id, shared.features.me.cert.as_ref(), phone_id, ts));
    link.sender
        .send(&packet("kdeconnect.pair", json!({ "pair": true, "timestamp": ts, "vyomaHost": shared.host.device_id, "vyomaHostProof": proof })))
        .await?;

    shared.features.away.linked(phone_id, shared.features.media.as_ref()).await;
    let result = serve(shared, phone_id, addr, pinned, &mut link, &name).await;
    shared.features.away.unlinked(phone_id, shared.features.media.clone());
    result
}

/// Feeds the link's packets to the phone's session while it stays paired in KDE Connect and no
/// Bluetooth link takes over, then ends the session.
async fn serve(shared: &Arc<Shared>, phone_id: &str, addr: SocketAddr, pinned: &[u8], link: &mut link::Link, name: &str) -> Result<()> {
    let (packets, rx) = tokio::sync::mpsc::unbounded_channel();
    let phone = Phone { id: phone_id.to_string(), name: name.to_string(), pinned: pinned.to_vec() };
    let session = tokio::spawn(session::serve(shared.features.clone(), phone, link.sender.clone(), rx));
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
        match p["type"].as_str().unwrap_or_default() {
            "kdeconnect.pair" => match p["body"]["pair"].as_bool() {
                Some(true) => info!("{name}: paired"),
                _ => warn!("{name}: declined pairing (needs Vyoma with companion support)"),
            },
            _ => {
                let _ = packets.send((p, addr.ip()));
            }
        }
    };
    // Closing the channel ends the session, and with it the screen, camera and controller.
    drop(packets);
    let _ = session.await;
    shared.sessions.lock().unwrap().remove(phone_id);
    result
}

fn still_trusted(trusted: &HashMap<String, Vec<u8>>, phone_id: &str, pinned: &[u8]) -> bool {
    trusted.get(phone_id).is_some_and(|cert| cert.as_slice() == pinned)
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
