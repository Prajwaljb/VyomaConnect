//! A development check of the standalone side: discovery, links and pairing with nothing else
//! running, against the phone's debug build only.
//!
//! VYOMA_DEV_DIR=<folder for its own identity and trust> VYOMA_DEV_ALLOW=<device id,...> \
//!     cargo run --release -p vyoma-core --example devpair [-- <phone address to announce to>]
//!
//! Talks to the listed ids alone, so the release app's pairing is never touched. Commands on
//! stdin: `pair <id>`, `accept`, `reject`, `unpair <id>`, `ping <id>`.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde_json::{Value, json};
use tokio::io::AsyncBufReadExt;
use tokio::sync::mpsc;
use vyoma_core::identity::Identity;
use vyoma_core::lan::{Lan, Ports, Rules, UDP_PORT};
use vyoma_core::link::{self, MAX_PACKET_BYTES, Sender, packet};
use vyoma_core::pairing::{Change, Outcome, PAIR, Pairing};
use vyoma_core::trust::{TrustStore, Trusted};

struct Peer {
    sender: Sender,
    cert: Vec<u8>,
    name: String,
    kind: String,
    protocol: i64,
    pairing: Pairing,
    warned: bool,
    /// Which of its links is current: a replaced one closing doesn't end the device.
    link: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("VYOMA_DEV_DIR").context("set VYOMA_DEV_DIR")?);
    let allow: Vec<String> = std::env::var("VYOMA_DEV_ALLOW").context("set VYOMA_DEV_ALLOW")?.split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect();
    let me = Arc::new(Identity::load_or_create_in(&dir)?);
    let mut trust = TrustStore::open(&dir)?;
    let identity = packet(
        "kdeconnect.identity",
        json!({ "deviceId": me.device_id, "deviceName": "Vyoma dev", "deviceType": "laptop", "protocolVersion": 8,
                "incomingCapabilities": ["kdeconnect.ping"], "outgoingCapabilities": ["kdeconnect.ping"] }),
    );
    let linked = Arc::new(Mutex::new(std::collections::HashSet::<String>::new()));
    let rules = {
        let linked = linked.clone();
        Rules { admits: Box::new(move |id| allow.iter().any(|a| a == id)), linked: Box::new(move |id| linked.lock().unwrap().contains(id)) }
    };
    let lan = Lan::bind(me.clone(), identity, rules, Ports::default()).await?;
    println!("this is {} on tcp/{}", me.device_id, lan.tcp_port());
    let (links_tx, mut links) = mpsc::channel(8);
    tokio::spawn(lan.clone().run(links_tx));
    let targets: Vec<SocketAddr> = std::env::args().skip(1).filter_map(|a| Some(SocketAddr::new(a.parse().ok()?, UDP_PORT))).collect();
    if !targets.is_empty() {
        let (lan, linked) = (lan.clone(), linked.clone());
        tokio::spawn(async move {
            loop {
                // Only while unlinked: KDE Connect answers each announcement with a new link.
                if linked.lock().unwrap().is_empty() {
                    let _ = lan.announce(&targets).await;
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });
    }

    let (packets_tx, mut packets) = mpsc::unbounded_channel::<(String, u64, Option<Value>)>();
    let mut links_made = 0u64;
    let mut peers: HashMap<String, Peer> = HashMap::new();
    let mut stdin = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            Some(mut l) = links.recv() => {
                let id = l.phone_identity["body"]["deviceId"].as_str().unwrap_or_default().to_owned();
                let body = &l.phone_identity["body"];
                let paired = match trust.get(&id) {
                    Some(t) if t.cert == l.peer_cert => true,
                    Some(_) => { println!("{id}: a different certificate than the paired one, refused"); continue }
                    None => false,
                };
                if paired { l.receiver.limit = MAX_PACKET_BYTES; }
                linked.lock().unwrap().insert(id.clone());
                println!("{id}: linked{}", if paired { ", paired" } else { "" });
                links_made += 1;
                let link_no = links_made;
                match peers.get_mut(&id) {
                    // A new link from the same certificate carries on where the last one was.
                    Some(peer) if peer.cert == l.peer_cert => { peer.sender = l.sender.clone(); peer.link = link_no; }
                    _ => {
                        let peer = Peer {
                            sender: l.sender.clone(), cert: l.peer_cert.clone(),
                            name: body["deviceName"].as_str().unwrap_or("").to_owned(), kind: body["deviceType"].as_str().unwrap_or("").to_owned(),
                            protocol: body["protocolVersion"].as_i64().unwrap_or(0), pairing: Pairing::new(me.cert.to_vec(), paired), warned: false, link: link_no,
                        };
                        peers.insert(id.clone(), peer);
                    }
                }
                let tx = packets_tx.clone();
                tokio::spawn(async move {
                    while let Ok(Some(p)) = l.receiver.next().await { let _ = tx.send((id.clone(), link_no, Some(p))); }
                    let _ = tx.send((id, link_no, None));
                });
            }
            Some((id, link_no, p)) = packets.recv() => {
                let Some(peer) = peers.get_mut(&id) else { continue };
                let Some(p) = p else {
                    if link_no != peer.link { continue }
                    println!("{id}: link closed");
                    if let Some(Change::Unpaired) = peer.pairing.link_lost() { println!("{id}: pairing called off"); }
                    peers.remove(&id);
                    linked.lock().unwrap().remove(&id);
                    continue;
                };
                if p["type"] == PAIR {
                    let out = peer.pairing.on_packet(&p["body"], &peer.cert, peer.protocol, link::now_ms() / 1000, Instant::now());
                    apply(&id, peer, out, &mut trust).await;
                } else if !peer.pairing.is_paired() {
                    if !peer.warned { peer.warned = true; let _ = peer.sender.send(&packet(PAIR, json!({ "pair": false }))).await; }
                } else if p["type"] == "kdeconnect.ping" {
                    println!("{id}: ping");
                } else {
                    println!("{id}: {}", p["type"]);
                }
            }
            Ok(Some(line)) = stdin.next_line() => {
                let mut words = line.split_whitespace();
                let (cmd, arg) = (words.next().unwrap_or(""), words.next());
                // With no id, the device whose request is on screen.
                let id = arg.map(str::to_owned).or_else(|| {
                    peers.iter().find(|(_, p)| matches!(p.pairing.state(), vyoma_core::pairing::State::RequestedByPeer { .. })).map(|(id, _)| id.clone())
                });
                let Some((id, peer)) = id.and_then(|id| peers.get_mut(&id).map(|p| (id, p))) else { println!("no such device"); continue };
                let out = match cmd {
                    "pair" => {
                        let cert = peer.cert.clone();
                        let ask = peer.pairing.request(&cert, link::now_ms() / 1000, Instant::now());
                        if let Some(ask) = ask { let _ = peer.sender.send(&ask).await; }
                        println!("{id}: asked, code {}", peer.pairing.key().unwrap_or_default());
                        Outcome::default()
                    }
                    "accept" => peer.pairing.accept(),
                    "reject" => peer.pairing.reject(),
                    "unpair" => peer.pairing.unpair(),
                    "ping" => { let _ = peer.sender.send(&packet("kdeconnect.ping", json!({}))).await; Outcome::default() }
                    _ => { println!("pair <id> | accept | reject | unpair <id> | ping <id>"); Outcome::default() }
                };
                apply(&id, peer, out, &mut trust).await;
            }
            _ = tick.tick() => {
                for (id, peer) in peers.iter_mut() {
                    if peer.pairing.tick(Instant::now()).is_some() { println!("{id}: pairing timed out"); }
                }
            }
        }
    }
}

async fn apply(id: &str, peer: &mut Peer, out: Outcome, trust: &mut TrustStore) {
    if let Some(reply) = out.reply {
        let _ = peer.sender.send(&reply).await;
    }
    match out.change {
        Some(Change::Prompt { key }) => println!("{id}: {} wants to pair, code {key} (accept / reject)", peer.name),
        Some(Change::Paired { cert }) => {
            println!("{id}: paired");
            if let Err(e) = trust.put(id, Trusted { name: peer.name.clone(), kind: peer.kind.clone(), cert }) {
                println!("couldn't save the pairing: {e:#}");
            }
        }
        Some(Change::Unpaired) => {
            println!("{id}: unpaired");
            let _ = trust.remove(id);
        }
        None => {}
    }
}
