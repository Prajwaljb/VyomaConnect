//! Every device this PC is linked with: its links, its pairing, and what the desktop must hear
//! about it (the phone's `Device` and `DeviceHub`).
//!
//! One task owns it all ([Hub::run]), fed by new links, packets and the desktop's commands, so
//! nothing here needs a lock. Sending never waits on a peer: each link has a writer of its own
//! with a bounded queue, and a peer that stops reading loses packets, not the hub.
//!
//! A device may have several links at once (both sides dialled, or a reconnect overlapping the
//! old one); all of them must show one certificate. Paired, that's the pinned one; with a pairing
//! on screen, the one whose code is shown; otherwise the newest replaces the others, since ids
//! are public and refusing would let a stranger hold a device's id and keep it out.

use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::link::{self, Link, MAX_PACKET_BYTES, UNPAIRED_PACKET_BYTES, packet};
use crate::pairing::{Change, Outcome, PAIR, Pairing, State};
use crate::trust::{TrustStore, Trusted};

/// Links one unpaired device may hold, and unpaired devices at once: each costs a socket and a task.
const MAX_UNPAIRED_LINKS: usize = 4;
const MAX_STRANGERS: usize = 8;
/// Packets waiting for one link's writer; past this the peer isn't reading and they're dropped.
const SEND_QUEUE: usize = 256;

/// What the desktop asks for.
#[derive(Debug)]
pub enum Command {
    Pair(String),
    Accept(String),
    Reject(String),
    Unpair(String),
    Send(String, Value),
}

/// What the desktop is told.
#[derive(Debug, PartialEq)]
pub enum Event {
    /// Its first link came up. [incoming] and [outgoing]: the packet types it takes and sends;
    /// [cert]: the certificate its links show, to pin payloads to.
    Linked { id: String, name: String, kind: String, paired: bool, incoming: Vec<String>, outgoing: Vec<String>, cert: Vec<u8>, remote: IpAddr },
    /// Its last link closed.
    Gone { id: String },
    /// It asks to pair: show [key] with Accept and Reject.
    Prompt { id: String, name: String, key: String },
    /// We asked: show [key] until it answers.
    Asked { id: String, name: String, key: String },
    /// A pending pairing ended without pairing (refused, ran out, link lost): take the prompt down.
    PairingOver { id: String },
    Paired { id: String, name: String },
    Unpaired { id: String, name: String },
    /// A packet from a paired device, for the plugins. [from]: the address of the link it came
    /// over, where a payload it announces is fetched.
    Packet { id: String, packet: Value, from: IpAddr },
}

struct LinkHandle {
    no: u64,
    cert: Vec<u8>,
    remote: IpAddr,
    queue: mpsc::Sender<Value>,
    reader: JoinHandle<()>,
}

impl LinkHandle {
    fn close(&self) {
        // The writer ends when its queue does; the reader holds the other half of the socket.
        self.reader.abort();
    }
}

struct Device {
    name: String,
    kind: String,
    protocol: i64,
    links: Vec<LinkHandle>,
    pairing: Pairing,
    /// Read by its links' readers: paired devices may send larger packets.
    paired: Arc<AtomicBool>,
    /// An unpaired device that sent something else is told once that we're not paired.
    warned: bool,
}

impl Device {
    /// The certificate the device must show: pinned, shown in a pending code, or its links'.
    fn cert<'a>(&'a self, trusted: Option<&'a Trusted>) -> Option<&'a [u8]> {
        if let Some(t) = trusted {
            return Some(&t.cert);
        }
        match self.pairing.state() {
            State::RequestedByUs { cert, .. } | State::RequestedByPeer { cert, .. } => Some(cert),
            _ => self.links.last().map(|l| l.cert.as_slice()),
        }
    }

    /// Over the newest link first; false if none would take it.
    fn send(&self, packet: &Value) -> bool {
        self.links.iter().rev().any(|l| l.queue.try_send(packet.clone()).is_ok())
    }
}

struct Incoming {
    id: String,
    link: u64,
    /// None: the link closed.
    packet: Option<Value>,
}

pub struct Hub {
    ours: Vec<u8>,
    trust: TrustStore,
    devices: HashMap<String, Device>,
    linked: Arc<Mutex<HashSet<String>>>,
    links_made: u64,
    events: mpsc::UnboundedSender<Event>,
    incoming_tx: mpsc::UnboundedSender<Incoming>,
    incoming: mpsc::UnboundedReceiver<Incoming>,
}

impl Hub {
    /// [ours]: our certificate (DER), for the codes.
    pub fn new(ours: Vec<u8>, trust: TrustStore, events: mpsc::UnboundedSender<Event>) -> Self {
        let (incoming_tx, incoming) = mpsc::unbounded_channel();
        Self { ours, trust, devices: HashMap::new(), linked: Arc::default(), links_made: 0, events, incoming_tx, incoming }
    }

    /// The ids with a live link, for [crate::lan::Rules::linked].
    pub fn linked(&self) -> Arc<Mutex<HashSet<String>>> {
        self.linked.clone()
    }

    pub async fn run(mut self, mut links: mpsc::Receiver<Link>, mut commands: mpsc::UnboundedReceiver<Command>) {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                Some(link) = links.recv() => self.add_link(link),
                Some(i) = self.incoming.recv() => self.on_incoming(i),
                Some(c) = commands.recv() => self.on_command(c),
                _ = tick.tick() => self.tick(Instant::now()),
                else => break,
            }
        }
    }

    fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }

    fn add_link(&mut self, mut link: Link) {
        let body = &link.phone_identity["body"];
        let Some(id) = body["deviceId"].as_str().map(str::to_owned) else { return };
        let trusted = self.trust.get(&id).cloned();
        let strangers = self.devices.iter().filter(|(k, _)| self.trust.get(k).is_none()).count();
        if let Some(t) = &trusted
            && t.cert != link.peer_cert
        {
            tracing::warn!("{id}: refused a link with another certificate than the paired one");
            return;
        }
        match self.devices.get_mut(&id) {
            Some(device) => {
                if device.cert(trusted.as_ref()).is_some_and(|c| c != link.peer_cert) {
                    if !matches!(device.pairing.state(), State::Unpaired) {
                        tracing::warn!("{id}: refused a link with another certificate during pairing");
                        return;
                    }
                    // Nobody's trusted under this id yet: the newest wins, as in any reconnect.
                    for l in device.links.drain(..) {
                        l.close();
                    }
                    device.warned = false;
                }
            }
            None if trusted.is_none() && strangers >= MAX_STRANGERS => {
                tracing::warn!("{id}: too many unpaired devices at once, refused");
                return;
            }
            None => {}
        }

        let name = crate::lan::device_name(body["deviceName"].as_str().unwrap_or(""));
        let kind = body["deviceType"].as_str().unwrap_or("").to_owned();
        let protocol = body["protocolVersion"].as_i64().unwrap_or(0);
        let strings = |v: &Value| v.as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_owned)).collect::<Vec<_>>()).unwrap_or_default();
        let (takes, sends) = (strings(&body["incomingCapabilities"]), strings(&body["outgoingCapabilities"]));
        let paired = trusted.is_some();
        let ours = self.ours.clone();
        let device = self.devices.entry(id.clone()).or_insert_with(|| Device {
            name: name.clone(),
            kind: kind.clone(),
            protocol,
            links: Vec::new(),
            pairing: Pairing::new(ours, paired),
            paired: Arc::new(AtomicBool::new(paired)),
            warned: false,
        });
        (device.name, device.kind, device.protocol) = (name.clone(), kind.clone(), protocol);
        if !paired {
            while device.links.len() >= MAX_UNPAIRED_LINKS {
                device.links.remove(0).close();
            }
        }
        let first = device.links.is_empty();

        self.links_made += 1;
        let no = self.links_made;
        let (queue, mut outgoing) = mpsc::channel::<Value>(SEND_QUEUE);
        let sender = link.sender.clone();
        tokio::spawn(async move {
            while let Some(p) = outgoing.recv().await {
                if sender.send(&p).await.is_err() {
                    break;
                }
            }
        });
        let (tx, flag, reader_id) = (self.incoming_tx.clone(), device.paired.clone(), id.clone());
        let reader = tokio::spawn(async move {
            loop {
                link.receiver.limit = if flag.load(Ordering::Relaxed) { MAX_PACKET_BYTES } else { UNPAIRED_PACKET_BYTES };
                match link.receiver.next().await {
                    Ok(Some(p)) => {
                        let _ = tx.send(Incoming { id: reader_id.clone(), link: no, packet: Some(p) });
                    }
                    Ok(None) => break,
                    Err(e) => {
                        tracing::debug!("{reader_id}: link ended: {e:#}");
                        break;
                    }
                }
            }
            let _ = tx.send(Incoming { id: reader_id, link: no, packet: None });
        });
        let (cert, remote) = (link.peer_cert.clone(), link.remote.ip().to_canonical());
        device.links.push(LinkHandle { no, cert: link.peer_cert, remote, queue, reader });
        if first {
            self.linked.lock().unwrap().insert(id.clone());
            tracing::info!("{id}: linked{}", if paired { ", paired" } else { "" });
            self.emit(Event::Linked { id, name, kind, paired, incoming: takes, outgoing: sends, cert, remote });
        }
    }

    fn on_incoming(&mut self, i: Incoming) {
        let Some(device) = self.devices.get_mut(&i.id) else { return };
        let Some(packet) = i.packet else {
            device.links.retain(|l| l.no != i.link);
            if device.links.is_empty() {
                let pending = device.pairing.link_lost().is_some();
                self.devices.remove(&i.id);
                self.linked.lock().unwrap().remove(&i.id);
                if pending {
                    self.emit(Event::PairingOver { id: i.id.clone() });
                }
                tracing::info!("{}: gone", i.id);
                self.emit(Event::Gone { id: i.id });
            }
            return;
        };
        // Read just before a replaced link was closed.
        if !device.links.iter().any(|l| l.no == i.link) {
            return;
        }
        match packet["type"].as_str() {
            Some(PAIR) => {
                let cert = device.links.iter().find(|l| l.no == i.link).map(|l| l.cert.clone()).unwrap_or_default();
                let out = device.pairing.on_packet(&packet["body"], &cert, device.protocol, link::now_ms() / 1000, Instant::now());
                self.apply(&i.id, out);
            }
            // v8 peers may resend it; what counts came during the handshake.
            Some("kdeconnect.identity") => {}
            _ if !device.pairing.is_paired() => {
                if !device.warned {
                    device.warned = true;
                    device.send(&packet_pair(false));
                }
            }
            _ => {
                let from = device.links.iter().find(|l| l.no == i.link).map(|l| l.remote).unwrap_or(IpAddr::from([127, 0, 0, 1]));
                self.emit(Event::Packet { id: i.id, packet, from });
            }
        }
    }

    fn on_command(&mut self, c: Command) {
        let (id, out) = match c {
            Command::Pair(id) => {
                let Some(device) = self.devices.get_mut(&id) else { return };
                let Some(cert) = device.links.last().map(|l| l.cert.clone()) else { return };
                let Some(ask) = device.pairing.request(&cert, link::now_ms() / 1000, Instant::now()) else { return };
                device.send(&ask);
                let (name, key) = (device.name.clone(), device.pairing.key().unwrap_or_default());
                self.emit(Event::Asked { id, name, key });
                return;
            }
            Command::Accept(id) => {
                let out = self.devices.get_mut(&id).map(|d| d.pairing.accept());
                (id, out)
            }
            Command::Reject(id) => {
                let out = self.devices.get_mut(&id).map(|d| d.pairing.reject());
                (id, out)
            }
            Command::Unpair(id) => {
                let out = match self.devices.get_mut(&id) {
                    Some(d) => Some(d.pairing.unpair()),
                    // Not here now: forgetting it is all there is to do.
                    None => self.trust.get(&id).is_some().then(|| Outcome { reply: None, change: Some(Change::Unpaired) }),
                };
                (id, out)
            }
            Command::Send(id, packet) => {
                if let Some(d) = self.devices.get(&id).filter(|d| d.pairing.is_paired()) {
                    d.send(&packet);
                }
                return;
            }
        };
        if let Some(out) = out {
            self.apply(&id, out);
        }
    }

    fn tick(&mut self, now: Instant) {
        let over: Vec<String> = self.devices.iter_mut().filter_map(|(id, d)| d.pairing.tick(now).map(|_| id.clone())).collect();
        for id in over {
            tracing::info!("{id}: pairing ran out");
            self.emit(Event::PairingOver { id });
        }
    }

    /// Sends the reply and carries out the change: pin, forget, or show a prompt.
    fn apply(&mut self, id: &str, out: Outcome) {
        let was_trusted = self.trust.get(id).cloned();
        if let (Some(reply), Some(device)) = (&out.reply, self.devices.get(id)) {
            device.send(reply);
        }
        let device = self.devices.get(id);
        let name = device.map(|d| d.name.clone()).or_else(|| was_trusted.as_ref().map(|t| t.name.clone())).unwrap_or_default();
        match out.change {
            Some(Change::Prompt { key }) => {
                tracing::info!("{id}: asks to pair");
                self.emit(Event::Prompt { id: id.to_owned(), name, key });
            }
            Some(Change::Paired { cert }) => {
                let kind = device.map(|d| d.kind.clone()).unwrap_or_default();
                if let Err(e) = self.trust.put(id, Trusted { name: name.clone(), kind, cert: cert.clone() }) {
                    // Not saved means not paired: it would be forgotten at the next start.
                    tracing::error!("{id}: couldn't save the pairing: {e:#}");
                    if let Some(d) = self.devices.get_mut(id) {
                        let out = d.pairing.unpair();
                        if let Some(reply) = out.reply {
                            d.send(&reply);
                        }
                    }
                    self.emit(Event::PairingOver { id: id.to_owned() });
                    return;
                }
                if let Some(d) = self.devices.get_mut(id) {
                    d.paired.store(true, Ordering::Relaxed);
                    // Links made with another certificate while it was pending can't stay.
                    d.links.retain(|l| {
                        let keep = l.cert == cert;
                        if !keep {
                            l.close();
                        }
                        keep
                    });
                }
                tracing::info!("{id}: paired");
                self.emit(Event::Paired { id: id.to_owned(), name });
            }
            Some(Change::Unpaired) => {
                if let Some(d) = self.devices.get_mut(id) {
                    d.paired.store(false, Ordering::Relaxed);
                }
                if was_trusted.is_some() {
                    if let Err(e) = self.trust.remove(id) {
                        tracing::error!("{id}: couldn't forget the pairing: {e:#}");
                    }
                    tracing::info!("{id}: unpaired");
                    self.emit(Event::Unpaired { id: id.to_owned(), name });
                } else {
                    self.emit(Event::PairingOver { id: id.to_owned() });
                }
            }
            None => {}
        }
    }
}

fn packet_pair(yes: bool) -> Value {
    packet(PAIR, json!({ "pair": yes }))
}
