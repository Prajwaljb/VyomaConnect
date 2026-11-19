//! The agent from the outside, as a phone and the window see it: the real program started on its
//! own ports and pipe (loopback only, so no phone nearby hears it), a phone made of vyoma-core
//! that links and pairs with it, and the window's pipe asking for the pairing and the phones.
//! Then the stock plugins and the engine answer: the command list, the stats, About this PC, a
//! screenshot, the players and the speaker. Unpairing, then stopping it as the installer does.

#![cfg(windows)]

use std::io::{BufRead, BufReader, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::{Child, Command as Process};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::time::timeout;
use vyoma_core::device::{Command, Event, Hub};
use vyoma_core::identity::Identity;
use vyoma_core::lan::{Lan, Ports, Rules};
use vyoma_core::link::packet;
use vyoma_core::payload;
use vyoma_core::trust::TrustStore;

const AGENT_UDP: u16 = 1916;
const PHONE_UDP: u16 = 1917;
const WAIT: Duration = Duration::from_secs(30);

struct Agent(Child);

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn scratch(what: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vyoma-e2e-{what}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// One request over the window's pipe, as the window makes it.
fn ask(pipe: &str, method: &str, args: Value) -> Value {
    for _ in 0..100 {
        if let Ok(mut p) = std::fs::OpenOptions::new().read(true).write(true).open(pipe) {
            writeln!(p, "{}", json!({ "method": method, "args": args })).unwrap();
            let mut line = String::new();
            BufReader::new(p).read_line(&mut line).unwrap();
            return serde_json::from_str(&line).unwrap();
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("no pipe {pipe}");
}

async fn window(pipe: &str, method: &str, args: Value) -> Value {
    let (pipe, method) = (pipe.to_owned(), method.to_owned());
    tokio::task::spawn_blocking(move || ask(&pipe, &method, args)).await.unwrap()
}

/// The next event that [want] picks, skipping the rest.
async fn next<T>(events: &mut mpsc::UnboundedReceiver<Event>, want: impl Fn(&Event) -> Option<T>) -> T {
    timeout(WAIT, async {
        loop {
            let e = events.recv().await.expect("the phone's hub stopped");
            if let Some(t) = want(&e) {
                return t;
            }
        }
    })
    .await
    .expect("waited too long")
}

/// The next packet of [kind] from the agent.
async fn packet_of(events: &mut mpsc::UnboundedReceiver<Event>, kind: &str) -> Value {
    next(events, |e| match e {
        Event::Packet { packet, .. } if packet["type"] == kind => Some(packet.clone()),
        _ => None,
    })
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_phone_links_pairs_and_is_served() {
    let (agent_dir, phone_dir) = (scratch("agent"), scratch("phone"));
    let phone = Arc::new(Identity::load_or_create_in(&phone_dir).unwrap());
    let pipe = format!(r"\\.\pipe\vyoma-e2e-{}", std::process::id());
    let mut agent = Agent(
        Process::new(env!("CARGO_BIN_EXE_vyoma-windows"))
            .env("VYOMA_DIR", &agent_dir)
            .env("VYOMA_PIPE", &pipe)
            .env("VYOMA_UDP_PORT", AGENT_UDP.to_string())
            .env("VYOMA_TCP_PORTS", "1914-1915")
            .env("VYOMA_ANNOUNCE", format!("127.0.0.1:{PHONE_UDP}"))
            .env("VYOMA_ALLOW", &phone.device_id)
            .spawn()
            .unwrap(),
    );

    // The phone: its own identity, ports and hub.
    let identity = packet(
        "kdeconnect.identity",
        json!({
            "deviceId": phone.device_id, "deviceName": "Test phone", "deviceType": "phone", "protocolVersion": 8,
            "incomingCapabilities": ["kdeconnect.ping", "kdeconnect.runcommand", "kdeconnect.mousepad.keyboardstate", "vyoma.stats", "vyoma.specs", "vyoma.screenshot", "vyoma.pc.leaving", "kdeconnect.mpris", "kdeconnect.systemvolume", "vyoma.pc", "vyoma.wallpaper", "vyoma.camera", "vyoma.mirror.state", "vyoma.mirror.input"],
            "outgoingCapabilities": ["kdeconnect.ping", "kdeconnect.runcommand.request", "vyoma.stats.request", "vyoma.specs.request", "vyoma.screenshot.request", "kdeconnect.mpris.request", "kdeconnect.systemvolume.request", "vyoma.pc.request", "vyoma.pc.set", "vyoma.wallpaper.request", "vyoma.away", "vyoma.camera.request", "vyoma.mirror"],
        }),
    );
    let (events_tx, mut events) = mpsc::unbounded_channel();
    let hub = Hub::new(phone.cert.to_vec(), TrustStore::open(&phone_dir).unwrap(), events_tx);
    let linked = hub.linked();
    let rules = Rules { admits: Box::new(|_| true), linked: Box::new(move |id| linked.lock().unwrap().contains(id)) };
    let ports = Ports { udp: PHONE_UDP, tcp: 1920..=1921, peers: 1914..=1915 };
    let lan = Lan::bind(phone.clone(), identity, rules, ports).await.unwrap();
    let (links_tx, links) = mpsc::channel(16);
    let (commands, commands_rx) = mpsc::unbounded_channel();
    tokio::spawn(lan.clone().run(links_tx));
    tokio::spawn(hub.run(links, commands_rx));

    // Announced until the agent dials back.
    let to_agent = [SocketAddr::from(([127, 0, 0, 1], AGENT_UDP))];
    let (pc, pc_cert) = timeout(WAIT, async {
        loop {
            let _ = lan.announce(&to_agent).await;
            if let Ok(Some(Event::Linked { id, kind, paired, cert, .. })) = timeout(Duration::from_millis(500), events.recv()).await {
                assert!(!paired);
                assert!(kind == "desktop" || kind == "laptop", "{kind}");
                return (id, cert);
            }
        }
    })
    .await
    .expect("the agent never linked");

    // The window sees the phone and asks to pair; the phone is asked and says yes.
    let phones = window(&pipe, "Phones", json!([])).await;
    let me = phones["ok"].as_array().unwrap().iter().find(|p| p["id"] == phone.device_id.as_str()).expect("the phone in the window").clone();
    assert_eq!((me["linked"].as_bool(), me["paired"].as_bool()), (Some(true), Some(false)));
    assert_eq!(window(&pipe, "Pair", json!([phone.device_id])).await, json!({ "ok": null }));
    let asking = next(&mut events, |e| match e {
        Event::Prompt { id, .. } => Some(id.clone()),
        _ => None,
    })
    .await;
    assert_eq!(asking, pc);
    commands.send(Command::Accept(pc.clone())).unwrap();
    next(&mut events, |e| matches!(e, Event::Paired { .. }).then_some(())).await;

    // Paired: the keyboard state arrives, and the stock plugins and the engine answer.
    packet_of(&mut events, "kdeconnect.mousepad.keyboardstate").await;
    let send = |kind: &str, body: Value| commands.send(Command::Send(pc.clone(), packet(kind, body))).unwrap();
    send("kdeconnect.runcommand.request", json!({ "requestCommandList": true }));
    let list = packet_of(&mut events, "kdeconnect.runcommand").await;
    assert!(list["body"]["commandList"].is_string(), "{list}");
    send("vyoma.specs.request", json!({}));
    let specs = packet_of(&mut events, "vyoma.specs").await;
    assert!(specs["body"]["os"].as_str().is_some_and(|o| !o.is_empty()), "{specs}");
    send("vyoma.stats.request", json!({}));
    let stats = packet_of(&mut events, "vyoma.stats").await;
    assert!(stats["body"]["memTotal"].as_u64().unwrap_or(0) > 0, "{stats}");
    send("vyoma.screenshot.request", json!({}));
    let shot = packet_of(&mut events, "vyoma.screenshot").await;
    println!("screenshot: {} bytes {}", shot["payloadSize"], shot["body"]["error"]);
    // A picture offered, or (no screen to capture, as on a service's desktop) the reason why.
    assert!(shot["payloadSize"].as_u64().unwrap_or(0) > 0 || shot["body"]["error"].is_string(), "{shot}");

    // Players and the speaker (Wine has no players; a sound device or not, the list comes).
    send("kdeconnect.mpris.request", json!({ "requestPlayerList": true }));
    let players = packet_of(&mut events, "kdeconnect.mpris").await;
    assert!(players["body"]["playerList"].is_array(), "{players}");
    send("kdeconnect.systemvolume.request", json!({ "requestSinks": true }));
    let sinks = packet_of(&mut events, "kdeconnect.systemvolume").await;
    assert!(sinks["body"]["sinkList"].is_array(), "{sinks}");
    println!("players: {}, speakers: {}", players["body"]["playerList"], sinks["body"]["sinkList"].as_array().unwrap().len());

    // Brightness and app volumes, and the wallpaper's colours (Wine's desktop: a plain colour).
    send("vyoma.pc.request", json!({}));
    let controls = packet_of(&mut events, "vyoma.pc").await;
    assert!(controls["body"]["displays"].is_array() && controls["body"]["apps"].is_array(), "{controls}");
    send("vyoma.wallpaper.request", json!({}));
    let wallpaper = packet_of(&mut events, "vyoma.wallpaper").await;
    assert_eq!(wallpaper["body"]["colors"].as_array().map(Vec::len), Some(16), "{wallpaper}");
    // Walk-away rules are taken quietly.
    send("vyoma.away", json!({ "mute": { "after": 600 } }));
    // A photo with the phone's Camera switch turned off in the window: refused, never reaching a camera.
    assert_eq!(window(&pipe, "SetPermission", json!([phone.device_id, "camera", false])).await, json!({ "ok": null }));
    send("vyoma.camera.request", json!({}));
    let refused = packet_of(&mut events, "vyoma.denied").await;
    println!("camera: {refused}");

    // The phone's screen in a window here: the agent opens the window (this same program as a
    // child), says so, and says it closed when the phone's stream ends. (Decoding is the unit
    // tests'; this stream is only start codes and filler.)
    let mut offer = payload::Offer::open().await.unwrap();
    let announced = offer.announce(packet("vyoma.mirror", json!({ "width": 320, "height": 240 })), -1).unwrap();
    commands.send(Command::Send(pc.clone(), announced)).unwrap();
    let stream: Vec<u8> = (0..200).flat_map(|i| [0, 0, 0, 1, 0x09, i as u8, 0xf0]).collect();
    let (phone_id, cert) = (phone.clone(), pc_cert.clone());
    let streaming = tokio::spawn(async move { offer.serve(&phone_id, &cert, stream.as_slice()).await });
    let opened = packet_of(&mut events, "vyoma.mirror.state").await;
    assert_eq!((opened["body"]["open"].as_bool(), opened["body"]["sound"].is_array()), (Some(true), true), "{opened}");
    let closed = packet_of(&mut events, "vyoma.mirror.state").await;
    assert_eq!(closed["body"]["open"].as_bool(), Some(false), "{closed}");
    assert!(closed["body"].get("error").is_none(), "{closed}");
    streaming.await.unwrap().unwrap();

    // The window reads it as paired, with this PC's switches; then unpairs it.
    let phones = window(&pipe, "Phones", json!([])).await;
    let me = phones["ok"].as_array().unwrap().iter().find(|p| p["id"] == phone.device_id.as_str()).unwrap().clone();
    assert_eq!(me["paired"].as_bool(), Some(true));
    assert_eq!(me["permits"].as_array().map(Vec::len), Some(5));
    assert!(me["permits"].as_array().unwrap().iter().any(|p| p["key"] == "camera" && p["on"] == false), "{me}");
    let health = window(&pipe, "Health", json!([])).await;
    assert!(health["ok"]["version"].is_string(), "{health}");
    assert_eq!(window(&pipe, "Unpair", json!([phone.device_id])).await, json!({ "ok": null }));
    next(&mut events, |e| matches!(e, Event::Unpaired { .. }).then_some(())).await;

    // The installer's --quit asks over the pipe: the agent stops.
    assert_eq!(window(&pipe, "Quit", json!([])).await, json!({ "ok": null }));
    let stopped = timeout(WAIT, async {
        loop {
            if agent.0.try_wait().unwrap().is_some() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    assert!(stopped.is_ok(), "the agent didn't stop");
    let _ = std::fs::remove_dir_all(&phone_dir);
}
