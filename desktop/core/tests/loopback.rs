//! Two of us on loopback: one announces, the other hears it and dials, and they pair.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;
use tokio::sync::mpsc;
use vyoma_core::identity::Identity;
use vyoma_core::lan::{Lan, Ports, Rules};
use vyoma_core::link::{self, MAX_PACKET_BYTES, packet};
use vyoma_core::pairing::{Change, Pairing};

fn identity(tag: &str) -> Arc<Identity> {
    let dir = std::env::temp_dir().join(format!("vyoma-loopback-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    Arc::new(Identity::load_or_create_in(&dir).unwrap())
}

fn hello(me: &Identity, name: &str) -> serde_json::Value {
    packet(
        "kdeconnect.identity",
        json!({ "deviceId": me.device_id, "deviceName": name, "deviceType": "desktop", "protocolVersion": 8,
                "incomingCapabilities": ["kdeconnect.ping"], "outgoingCapabilities": ["kdeconnect.ping"] }),
    )
}

/// Any free port, and dialling whatever port a hello names.
fn anywhere() -> Ports {
    Ports { udp: 0, tcp: 0..=0, peers: 1..=u16::MAX }
}

fn everyone() -> Rules {
    Rules { admits: Box::new(|_| true), linked: Box::new(|_| false) }
}

#[tokio::test]
async fn hear_dial_and_pair() {
    let (a, b) = (identity("a"), identity("b"));
    let lan_a = Lan::bind(a.clone(), hello(&a, "A"), everyone(), anywhere()).await.unwrap();
    let lan_b = Lan::bind(b.clone(), hello(&b, "B"), everyone(), anywhere()).await.unwrap();
    let (tx_a, mut rx_a) = mpsc::channel(4);
    let (tx_b, mut rx_b) = mpsc::channel(4);
    tokio::spawn(lan_a.clone().run(tx_a));
    tokio::spawn(lan_b.clone().run(tx_b));

    // A announces straight to B's UDP port; B dials A's listener.
    let to_b = format!("127.0.0.1:{}", lan_b.udp_port().unwrap()).parse().unwrap();
    lan_a.announce(&[to_b]).await.unwrap();
    let within = Duration::from_secs(10);
    let mut at_a = tokio::time::timeout(within, rx_a.recv()).await.unwrap().unwrap();
    let mut at_b = tokio::time::timeout(within, rx_b.recv()).await.unwrap().unwrap();
    assert_eq!(at_a.phone_identity["body"]["deviceId"], b.device_id.as_str());
    assert_eq!(at_b.phone_identity["body"]["deviceId"], a.device_id.as_str());
    assert_eq!(at_a.peer_cert, b.cert.to_vec());
    assert_eq!(at_b.peer_cert, a.cert.to_vec());

    // A asks, B shows the code and the user accepts it.
    let now_s = link::now_ms() / 1000;
    let mut pa = Pairing::new(a.cert.to_vec(), false);
    let mut pb = Pairing::new(b.cert.to_vec(), false);
    at_a.sender.send(&pa.request(&at_a.peer_cert, now_s, Instant::now()).unwrap()).await.unwrap();
    let asked = at_b.receiver.next().await.unwrap().unwrap();
    let out = pb.on_packet(&asked["body"], &at_b.peer_cert, 8, now_s, Instant::now());
    let Some(Change::Prompt { key }) = out.change else { panic!("no prompt: {out:?}") };
    assert_eq!(Some(key), pa.key(), "both screens show one code");
    let out = pb.accept();
    at_b.sender.send(&out.reply.unwrap()).await.unwrap();
    assert_eq!(out.change, Some(Change::Paired { cert: a.cert.to_vec() }));

    let yes = at_a.receiver.next().await.unwrap().unwrap();
    let out = pa.on_packet(&yes["body"], &at_a.peer_cert, 8, now_s, Instant::now());
    assert_eq!(out.change, Some(Change::Paired { cert: b.cert.to_vec() }));
    at_a.receiver.limit = MAX_PACKET_BYTES;
}

#[tokio::test]
async fn a_certificate_that_names_someone_else_is_refused() {
    let (a, b) = (identity("c"), identity("d"));
    let lan_a = Lan::bind(a.clone(), hello(&a, "A"), everyone(), anywhere()).await.unwrap();
    let (tx_a, mut rx_a) = mpsc::channel(4);
    tokio::spawn(lan_a.clone().run(tx_a));
    let addr = format!("127.0.0.1:{}", lan_a.tcp_port()).parse().unwrap();
    // B claims an id its certificate doesn't carry.
    let mut lie = hello(&b, "B");
    lie["body"]["deviceId"] = "someone_else_0123456789".into();
    assert!(link::dial(addr, &b, &lie, &a.device_id).await.is_err());
    assert!(tokio::time::timeout(Duration::from_millis(500), rx_a.recv()).await.is_err(), "no link came of it");
}
