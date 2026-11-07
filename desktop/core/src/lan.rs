//! Finding devices on the network, and the links they open to us (the phone's `LanBackend`).
//!
//! Each side broadcasts its identity over UDP 1716. Whoever hears one dials the sender's
//! `tcpPort` ([link::dial]); a device that heard ours dials our listener ([link::accept]). UDP 1716
//! is shared (SO_REUSEADDR: every socket on it gets broadcasts), so this runs beside another KDE
//! Connect; TCP takes the first free port from 1716, so beside kdeconnectd that's 1717.
//!
//! Anyone on the network can start a handshake, so they're limited: 24 at once, 4 per address, 20 s
//! each in all, and discovery dials only KDE Connect's ports, at most 16 at a time.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::ops::RangeInclusive;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::{Semaphore, mpsc};

use crate::identity::Identity;
use crate::link::{self, Link, UNPAIRED_PACKET_BYTES};

pub const UDP_PORT: u16 = 1716;
pub const TCP_PORTS: RangeInclusive<u16> = 1716..=1764;
pub const PROTOCOL_VERSION: i64 = 8;
const MAX_HANDSHAKES: usize = 24;
const MAX_HANDSHAKES_PER_ADDRESS: usize = 4;
const MAX_DISCOVERY_DIALS: usize = 16;
const HANDSHAKE_DEADLINE: Duration = Duration::from_secs(20);

/// What KDE Connect accepts as a device id (v8 makes `[a-zA-Z0-9_]{32,38}`; older peers used
/// shorter hex ids and dashes).
pub fn valid_device_id(id: &str) -> bool {
    (16..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// A peer's name as we show it: no quotes or punctuation KDE Connect forbids, at most 32 characters.
pub fn device_name(raw: &str) -> String {
    let name: String = raw.chars().filter(|c| !"\"',;:.!?()[]<>".contains(*c)).collect();
    let name: String = name.trim().chars().take(32).collect();
    if name.is_empty() { "Unnamed".to_owned() } else { name }
}

/// The parts of an identity packet that decide whether and where to link.
#[derive(Debug, PartialEq)]
pub struct Hello {
    pub id: String,
    pub name: String,
    pub protocol: i64,
    pub tcp_port: Option<u16>,
}

impl Hello {
    pub fn parse(packet: &Value) -> Option<Self> {
        if packet["type"] != "kdeconnect.identity" {
            return None;
        }
        let body = &packet["body"];
        let id = body["deviceId"].as_str().filter(|id| valid_device_id(id))?.to_owned();
        // Peers mix numbers and strings.
        let int = |v: &Value| v.as_i64().or_else(|| v.as_str()?.parse().ok());
        Some(Self {
            id,
            name: device_name(body["deviceName"].as_str().unwrap_or("")),
            protocol: int(&body["protocolVersion"]).unwrap_or(0),
            tcp_port: int(&body["tcpPort"]).and_then(|p| u16::try_from(p).ok()),
        })
    }
}

/// Who gets a link.
pub struct Rules {
    /// Whether to talk to this device at all; asked before TLS.
    pub admits: Box<dyn Fn(&str) -> bool + Send + Sync>,
    /// Already linked: its broadcasts aren't dialled (KDE Connect keeps only the newest link).
    pub linked: Box<dyn Fn(&str) -> bool + Send + Sync>,
}

/// Where to listen and whom to dial.
#[derive(Clone)]
pub struct Ports {
    pub udp: u16,
    /// Our listener takes the first free one.
    pub tcp: RangeInclusive<u16>,
    /// Broadcasts naming a port outside these aren't dialled: an announcement shouldn't be able to
    /// point us at any service on the network.
    pub peers: RangeInclusive<u16>,
}

impl Default for Ports {
    fn default() -> Self {
        Self { udp: UDP_PORT, tcp: TCP_PORTS, peers: TCP_PORTS }
    }
}

/// Counts handshakes in progress, in all and per address; a [Pass] gives its place back when dropped.
struct Gate(Mutex<HashMap<IpAddr, usize>>);

struct Pass(Arc<Gate>, IpAddr);

impl Gate {
    fn enter(self: &Arc<Self>, from: IpAddr) -> Option<Pass> {
        let mut running = self.0.lock().unwrap();
        let mine = running.get(&from).copied().unwrap_or(0);
        if mine >= MAX_HANDSHAKES_PER_ADDRESS || running.values().sum::<usize>() >= MAX_HANDSHAKES {
            return None;
        }
        running.insert(from, mine + 1);
        Some(Pass(self.clone(), from))
    }
}

impl Drop for Pass {
    fn drop(&mut self) {
        let mut running = self.0.0.lock().unwrap();
        if let Some(n) = running.get_mut(&self.1) {
            *n -= 1;
            if *n == 0 {
                running.remove(&self.1);
            }
        }
    }
}

pub struct Lan {
    me: Arc<Identity>,
    /// Our identity packet, `tcpPort` filled in.
    identity: Value,
    tcp: TcpListener,
    /// None when UDP 1716 can't be had: then we only accept, and announce from any port.
    udp: Option<UdpSocket>,
    rules: Rules,
    peer_ports: RangeInclusive<u16>,
    gate: Arc<Gate>,
    dials: Arc<Semaphore>,
    dialling: Mutex<HashSet<String>>,
}

impl Lan {
    /// Listens on the first free TCP port and on the UDP one, sharing it.
    pub async fn bind(me: Arc<Identity>, mut identity: Value, rules: Rules, ports: Ports) -> Result<Arc<Self>> {
        let udp_port = ports.udp;
        let tcp = bind_first_free(ports.tcp)?;
        let port = tcp.local_addr()?.port();
        identity["body"]["tcpPort"] = port.into();
        let udp = match shared_udp(udp_port) {
            Ok(udp) => Some(udp),
            Err(e) => {
                tracing::warn!("can't hear broadcasts on udp/{udp_port}: {e:#}");
                None
            }
        };
        tracing::info!("listening on tcp/{port}{}", if udp.is_some() { format!(", udp/{udp_port}") } else { String::new() });
        Ok(Arc::new(Self {
            me,
            identity,
            tcp,
            udp,
            rules,
            peer_ports: ports.peers,
            gate: Arc::new(Gate(Mutex::new(HashMap::new()))),
            dials: Arc::new(Semaphore::new(MAX_DISCOVERY_DIALS)),
            dialling: Mutex::new(HashSet::new()),
        }))
    }

    pub fn tcp_port(&self) -> u16 {
        self.tcp.local_addr().map(|a| a.port()).unwrap_or(0)
    }

    /// Where we hear broadcasts, if we do.
    pub fn udp_port(&self) -> Option<u16> {
        self.udp.as_ref()?.local_addr().ok().map(|a| a.port())
    }

    pub fn identity(&self) -> &Value {
        &self.identity
    }

    /// Sends our identity to each of [targets] (the broadcast address, or hosts the user added).
    pub async fn announce(&self, targets: &[SocketAddr]) -> Result<()> {
        let mut line = serde_json::to_vec(&self.identity)?;
        line.push(b'\n');
        let (mut v4, mut v6) = (None, None);
        for target in targets {
            // Ours is IPv4; an IPv6 host gets it from a socket of its own.
            let udp = match (&self.udp, target) {
                (Some(udp), SocketAddr::V4(_)) => udp,
                (_, SocketAddr::V4(_)) => match &v4 {
                    Some(udp) => udp,
                    None => {
                        let udp = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await?;
                        udp.set_broadcast(true)?;
                        v4.insert(udp)
                    }
                },
                (_, SocketAddr::V6(_)) => match &v6 {
                    Some(udp) => udp,
                    None => v6.insert(UdpSocket::bind((Ipv6Addr::UNSPECIFIED, 0)).await?),
                },
            };
            if let Err(e) = udp.send_to(&line, target).await {
                tracing::debug!("announce to {target}: {e}");
            }
        }
        Ok(())
    }

    /// Takes links until the channel closes: those dialled to us, and ours to whoever broadcasts.
    pub async fn run(self: Arc<Self>, links: mpsc::Sender<Link>) {
        let heard = self.clone().hear(links.clone());
        let accepted = self.accept(links);
        tokio::join!(heard, accepted);
    }

    async fn accept(self: Arc<Self>, links: mpsc::Sender<Link>) {
        while !links.is_closed() {
            let (mut tcp, from) = match self.tcp.accept().await {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("accept: {e}");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            };
            let Some(pass) = self.gate.enter(from.ip().to_canonical()) else { continue };
            let (lan, links) = (self.clone(), links.clone());
            tokio::spawn(async move {
                let _pass = pass;
                let link = tokio::time::timeout(HANDSHAKE_DEADLINE, async {
                    let hello = link::read_hello(&mut tcp).await?;
                    let id = lan.wanted(Hello::parse(&hello).context("bad identity")?)?;
                    link::accept(tcp, &lan.me, &lan.identity, &id).await
                })
                .await
                .context("handshake took too long");
                match link.and_then(|l| l) {
                    Ok(link) => {
                        let _ = links.send(link).await;
                    }
                    Err(e) => tracing::debug!("incoming from {from}: {e:#}"),
                }
            });
        }
    }

    async fn hear(self: Arc<Self>, links: mpsc::Sender<Link>) {
        let Some(udp) = &self.udp else { return };
        let mut buf = vec![0u8; UNPAIRED_PACKET_BYTES];
        while !links.is_closed() {
            let (n, from) = match udp.recv_from(&mut buf).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("udp: {e}");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            };
            let Some(hello) = serde_json::from_slice::<Value>(&buf[..n]).ok().and_then(|p| Hello::parse(&p)) else { continue };
            let Some(port) = hello.tcp_port.filter(|p| self.peer_ports.contains(p)) else { continue };
            let Ok(id) = self.wanted(hello) else { continue };
            if (self.rules.linked)(&id) || !self.dialling.lock().unwrap().insert(id.clone()) {
                continue;
            }
            let Ok(permit) = self.dials.clone().try_acquire_owned() else {
                self.dialling.lock().unwrap().remove(&id);
                continue;
            };
            let (lan, links) = (self.clone(), links.clone());
            tokio::spawn(async move {
                let _permit = permit;
                let addr = SocketAddr::new(from.ip(), port);
                let link = tokio::time::timeout(HANDSHAKE_DEADLINE, link::dial(addr, &lan.me, &lan.identity, &id)).await.context("handshake took too long");
                lan.dialling.lock().unwrap().remove(&id);
                match link.and_then(|l| l) {
                    Ok(link) => {
                        let _ = links.send(link).await;
                    }
                    Err(e) => tracing::debug!("dialling {id} at {addr}: {e:#}"),
                }
            });
        }
    }

    /// The device's id if we'll link with it: not ourselves, v8, and admitted.
    fn wanted(&self, hello: Hello) -> Result<String> {
        if hello.id == self.me.device_id {
            bail!("ourselves");
        }
        if hello.protocol < PROTOCOL_VERSION {
            bail!("protocol {} is too old", hello.protocol);
        }
        if !(self.rules.admits)(&hello.id) {
            bail!("{} isn't admitted", hello.id);
        }
        Ok(hello.id)
    }
}

/// The first port of [ports] free for a listener, on IPv6 and IPv4 both where the system has IPv6
/// (a peer dials back the address its announcement came from). SO_REUSEADDR so a restart needn't
/// wait out the last run's closed connections; Linux still won't share a port someone listens on.
fn bind_first_free(ports: RangeInclusive<u16>) -> Result<TcpListener> {
    let listen = |port: u16| -> std::io::Result<TcpListener> {
        let dual = Socket::new(Domain::IPV6, Type::STREAM, Some(Protocol::TCP)).and_then(|s| {
            s.set_only_v6(false)?;
            Ok(s)
        });
        let (socket, any) = match dual {
            Ok(s) => (s, SocketAddr::from((Ipv6Addr::UNSPECIFIED, port))),
            Err(_) => (Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?, SocketAddr::from((Ipv4Addr::UNSPECIFIED, port))),
        };
        socket.set_reuse_address(true)?;
        socket.bind(&any.into())?;
        socket.listen(128)?;
        socket.set_nonblocking(true)?;
        TcpListener::from_std(socket.into())
    };
    for port in ports.clone() {
        if let Ok(listener) = listen(port) {
            return Ok(listener);
        }
    }
    bail!("no free TCP port in {ports:?}")
}

/// UDP [port] shared with whoever else listens there (kdeconnectd), broadcasts allowed.
fn shared_udp(port: u16) -> Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    socket.set_broadcast(true)?;
    socket.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)).into())?;
    socket.set_nonblocking(true)?;
    Ok(UdpSocket::from_std(socket.into())?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::packet;
    use serde_json::json;

    #[test]
    fn ids_and_names_are_checked() {
        assert!(valid_device_id("0123456789abcdef"));
        assert!(valid_device_id("a1b2c3d4_e5f6_4a7b_8c9d_0e1f2a3b4c5d"));
        assert!(!valid_device_id("short"));
        assert!(!valid_device_id("0123456789abcdef/../x"));
        assert!(!valid_device_id(&"a".repeat(65)));
        assert_eq!(device_name("  Ann's (new) phone!  "), "Anns new phone");
        assert_eq!(device_name("?!."), "Unnamed");
        assert_eq!(device_name(&"x".repeat(40)).len(), 32);
    }

    #[test]
    fn hellos_take_numbers_or_strings() {
        let p = packet("kdeconnect.identity", json!({"deviceId": "0123456789abcdef", "deviceName": "P", "protocolVersion": "8", "tcpPort": 1716}));
        assert_eq!(Hello::parse(&p), Some(Hello { id: "0123456789abcdef".into(), name: "P".into(), protocol: 8, tcp_port: Some(1716) }));
        assert_eq!(Hello::parse(&packet("kdeconnect.ping", json!({}))), None);
        assert_eq!(Hello::parse(&packet("kdeconnect.identity", json!({"deviceId": "bad"}))), None);
    }

    #[test]
    fn the_gate_counts_per_address_and_in_all() {
        let gate = Arc::new(Gate(Mutex::new(HashMap::new())));
        let a = IpAddr::from([10, 0, 0, 1]);
        let passes: Vec<_> = (0..MAX_HANDSHAKES_PER_ADDRESS).map(|_| gate.enter(a).unwrap()).collect();
        assert!(gate.enter(a).is_none());
        drop(passes);
        assert!(gate.enter(a).is_some());
        let many: Vec<_> = (0..MAX_HANDSHAKES).map(|i| gate.enter(IpAddr::from([10, 0, 1, i as u8])).unwrap()).collect();
        assert!(gate.enter(IpAddr::from([10, 0, 2, 1])).is_none());
        drop(many);
        assert!(gate.0.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn udp_is_shared() {
        let a = shared_udp(0).unwrap();
        let port = a.local_addr().unwrap().port();
        let b = shared_udp(port);
        assert!(b.is_ok(), "a second listener on the same port");
    }
}
