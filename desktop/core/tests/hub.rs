//! The hub over real loopback links: a test peer dials it and plays the phone's part.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc;
use vyoma_core::device::{Command, Event, Hub};
use vyoma_core::identity::Identity;
use vyoma_core::lan::{Lan, Ports, Rules};
use vyoma_core::link::{self, Link, packet};
use vyoma_core::pairing::verification_key;
use vyoma_core::trust::TrustStore;

fn dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("vyoma-hub-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn hello(me: &Identity) -> Value {
    packet(
        "kdeconnect.identity",
        json!({ "deviceId": me.device_id, "deviceName": "Test phone", "deviceType": "phone", "protocolVersion": 8,
                "incomingCapabilities": ["kdeconnect.ping"], "outgoingCapabilities": ["kdeconnect.ping"] }),
    )
}

struct Pc {
    me: Arc<Identity>,
    port: u16,
    events: mpsc::UnboundedReceiver<Event>,
    commands: mpsc::UnboundedSender<Command>,
    dir: std::path::PathBuf,
}

impl Pc {
    async fn start(tag: &str) -> Self {
        let dir = dir(tag);
        let me = Arc::new(Identity::load_or_create_in(&dir).unwrap());
        let (events_tx, events) = mpsc::unbounded_channel();
        let hub = Hub::new(me.cert.to_vec(), TrustStore::open(&dir).unwrap(), events_tx);
        let linked = hub.linked();
        let rules = Rules { admits: Box::new(|_| true), linked: Box::new(move |id| linked.lock().unwrap().contains(id)) };
        let lan = Lan::bind(me.clone(), hello(&me), rules, Ports { udp: 0, tcp: 0..=0, peers: 1..=u16::MAX }).await.unwrap();
        let (links_tx, links) = mpsc::channel(4);
        let (commands, commands_rx) = mpsc::unbounded_channel();
        tokio::spawn(lan.clone().run(links_tx));
        tokio::spawn(hub.run(links, commands_rx));
        Self { port: lan.tcp_port(), me, events, commands, dir }
    }

    async fn event(&mut self) -> Event {
        tokio::time::timeout(Duration::from_secs(5), self.events.recv()).await.expect("no event").unwrap()
    }

    async fn dial(&self, phone: &Identity) -> Link {
        let addr = format!("127.0.0.1:{}", self.port).parse().unwrap();
        link::dial(addr, phone, &hello(phone), &self.me.device_id).await.unwrap()
    }
}

async fn next(link: &mut Link) -> Value {
    tokio::time::timeout(Duration::from_secs(5), link.receiver.next()).await.expect("nothing came").unwrap().unwrap()
}

fn phone(tag: &str) -> Arc<Identity> {
    Arc::new(Identity::load_or_create_in(&dir(tag)).unwrap())
}

#[tokio::test]
async fn a_phone_asks_the_pc_accepts_and_packets_flow() {
    let mut pc = Pc::start("pc1").await;
    let ph = phone("ph1");
    let mut link = pc.dial(&ph).await;
    assert!(matches!(pc.event().await, Event::Linked { paired: false, .. }));

    // Unpaired, other packets are answered with pair:false, once.
    link.sender.send(&packet("kdeconnect.ping", json!({}))).await.unwrap();
    link.sender.send(&packet("kdeconnect.ping", json!({}))).await.unwrap();
    assert_eq!(next(&mut link).await["body"], json!({ "pair": false }));

    let ts = link::now_ms() / 1000;
    link.sender.send(&packet("kdeconnect.pair", json!({ "pair": true, "timestamp": ts }))).await.unwrap();
    let Event::Prompt { id, key, .. } = pc.event().await else { panic!("no prompt") };
    assert_eq!(id, ph.device_id);
    assert_eq!(Some(key), verification_key(&ph.cert, &pc.me.cert, Some(ts)));
    pc.commands.send(Command::Accept(id.clone())).unwrap();
    assert_eq!(next(&mut link).await["body"], json!({ "pair": true }));
    assert!(matches!(pc.event().await, Event::Paired { .. }));
    assert!(TrustStore::open(&pc.dir).unwrap().get(&id).is_some(), "the pin is saved");

    link.sender.send(&packet("kdeconnect.ping", json!({}))).await.unwrap();
    let Event::Packet { packet: got, .. } = pc.event().await else { panic!("no packet") };
    assert_eq!(got["type"], "kdeconnect.ping");

    pc.commands.send(Command::Send(id.clone(), packet("kdeconnect.ping", json!({})))).unwrap();
    assert_eq!(next(&mut link).await["type"], "kdeconnect.ping");

    pc.commands.send(Command::Unpair(id.clone())).unwrap();
    assert_eq!(next(&mut link).await["body"], json!({ "pair": false }));
    assert!(matches!(pc.event().await, Event::Unpaired { .. }));
    assert!(TrustStore::open(&pc.dir).unwrap().get(&id).is_none());
}

#[tokio::test]
async fn the_pc_asks_and_the_phone_says_yes() {
    let mut pc = Pc::start("pc2").await;
    let ph = phone("ph2");
    let mut link = pc.dial(&ph).await;
    pc.event().await;
    pc.commands.send(Command::Pair(ph.device_id.clone())).unwrap();
    let asked = next(&mut link).await;
    let Event::Asked { key, .. } = pc.event().await else { panic!("not asked") };
    assert_eq!(Some(key), verification_key(&ph.cert, &pc.me.cert, asked["body"]["timestamp"].as_i64()));
    link.sender.send(&packet("kdeconnect.pair", json!({ "pair": true }))).await.unwrap();
    assert!(matches!(pc.event().await, Event::Paired { .. }));
}

#[tokio::test]
async fn once_paired_another_certificate_under_that_id_is_refused() {
    let mut pc = Pc::start("pc3").await;
    let ph = phone("ph3");
    let link = pc.dial(&ph).await;
    pc.event().await;
    link.sender.send(&packet("kdeconnect.pair", json!({ "pair": true, "timestamp": link::now_ms() / 1000 }))).await.unwrap();
    pc.event().await;
    pc.commands.send(Command::Accept(ph.device_id.clone())).unwrap();
    pc.event().await;

    // Someone else's key and certificate, under the phone's id.
    let other = phone("ph3-impostor");
    let fake = Identity { device_id: ph.device_id.clone(), cert: other.cert.clone(), key: other.key.clone_key() };
    let addr = format!("127.0.0.1:{}", pc.port).parse().unwrap();
    // Its certificate names another id, so it's refused already at the handshake...
    assert!(link::dial(addr, &fake, &hello(&ph), &pc.me.device_id).await.is_err());
    // ...and the real phone is still linked and paired.
    link.sender.send(&packet("kdeconnect.ping", json!({}))).await.unwrap();
    assert!(matches!(pc.event().await, Event::Packet { .. }));
}

#[tokio::test]
async fn a_second_link_carries_on_the_pairing() {
    let mut pc = Pc::start("pc4").await;
    let ph = phone("ph4");
    let first = pc.dial(&ph).await;
    pc.event().await;
    first.sender.send(&packet("kdeconnect.pair", json!({ "pair": true, "timestamp": link::now_ms() / 1000 }))).await.unwrap();
    assert!(matches!(pc.event().await, Event::Prompt { .. }));
    // The phone reconnects while the code is on screen; the old link goes.
    let mut second = pc.dial(&ph).await;
    // Once the hub has it: with no link at all for a moment, the request would end.
    tokio::time::sleep(Duration::from_millis(200)).await;
    drop(first);
    tokio::time::sleep(Duration::from_millis(200)).await;
    pc.commands.send(Command::Accept(ph.device_id.clone())).unwrap();
    assert_eq!(next(&mut second).await["body"], json!({ "pair": true }));
    assert!(matches!(pc.event().await, Event::Paired { .. }));
}
