//! One phone's session: every vyoma.* feature, as the companion serves it beside kdeconnectd and
//! the standalone desktop app serves it on its own links. The caller owns the link and feeds the
//! packets in; this answers through a [Sender] and ends what it started when the packets stop.

use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::sync::mpsc;
use tracing::{info, warn};
use vyoma_core::identity::Identity;
use vyoma_core::link::{Sender, packet};
use vyoma_core::payload;

use crate::permits::{self, Permit, Permits};
use crate::{accessory, away, bluetooth, camera, desktop, files, gamepad, media, mirror, paste, pc, pointer, screen, screenshot, specs, stats, wallpaper, webcam, xfer};

/// The phone's stats come for this long after it last asked.
const STATS_LEASE: Duration = Duration::from_secs(12);
const STATS_EVERY: Duration = Duration::from_secs(2);

/// What every session on this PC shares.
/// The vyoma.* packets a session takes and sends.
pub const INCOMING: &[&str] = &["vyoma.away", "vyoma.stats.request", "vyoma.screenshot.request", "vyoma.camera.request", "vyoma.handoff.request", "vyoma.files.list", "vyoma.files.get", "vyoma.files.put", "vyoma.screen.request", "vyoma.screen.stop", "vyoma.screen.input", "vyoma.screen.touch", "vyoma.pc.request", "vyoma.pc.set", "vyoma.pc.power", "vyoma.pc.wake", "vyoma.xfer.begin", "vyoma.xfer.data", "vyoma.xfer.end", "vyoma.wallpaper.request", "vyoma.specs.request", "vyoma.gamepad.state", "vyoma.gamepad.close", "vyoma.desktop.step", "vyoma.usb.accessory", "vyoma.webcam", "vyoma.webcam.mic", "vyoma.webcam.turn", "vyoma.mirror", "vyoma.mirror.audio", "vyoma.mirror.copy", "vyoma.mirror.volume", "vyoma.paste", "vyoma.pointer.request", "vyoma.pointer.close"];
pub const OUTGOING: &[&str] = &["vyoma.bluetooth", "vyoma.pointer.ready", "vyoma.paste.ready", "vyoma.stats", "vyoma.screenshot", "vyoma.camera", "vyoma.handoff", "vyoma.files.listing", "vyoma.files.file", "vyoma.files.saved", "vyoma.screen", "vyoma.pc", "vyoma.pc.powered", "vyoma.xfer.ready", "vyoma.xfer.done", "vyoma.wallpaper", "vyoma.specs", "vyoma.gamepad.rumble", "vyoma.gamepad.udp", "vyoma.desktop.stepped", "vyoma.webcam.state", "vyoma.mirror.input", "vyoma.mirror.state", "vyoma.denied"];

pub struct Host {
    pub me: Arc<Identity>,
    /// Tracks what's playing for handoff; None without a session bus.
    pub media: Option<Arc<media::Media>>,
    /// Walk-away rules each phone asked for.
    pub away: Arc<away::Away>,
    /// What each phone may do here.
    pub permits: Arc<Permits>,
}

/// The phone at the other end.
pub struct Phone {
    pub id: String,
    /// Its name as it's shown here (see [phone_name]).
    pub name: String,
    /// Its certificate, which its payload connections must show.
    pub pinned: Vec<u8>,
}

/// A phone's name as it's safe to show: no markup, nothing an argument could mistake for an option.
pub fn phone_name(raw: &str) -> String {
    let clean: String = raw.chars().filter(|c| !matches!(c, '"' | '\'' | ',' | ';' | ':' | '.' | '!' | '?' | '(' | ')' | '[' | ']' | '<' | '>') && !c.is_control()).take(32).collect();
    let clean = clean.trim().trim_start_matches('-').trim();
    if clean.is_empty() { "Phone".to_string() } else { clean.to_string() }
}

/// Serves one phone's vyoma.* [packets] (each with the address it came from, where its payload
/// is fetched) until the channel closes, then ends everything it started
/// (screen, camera, controller…). Replies go out through [sender].
pub async fn serve(host: Arc<Host>, phone: Phone, sender: Sender, mut packets: mpsc::UnboundedReceiver<(Value, IpAddr)>) {
    let (phone_id, pinned, name) = (phone.id.as_str(), phone.pinned.as_slice(), phone.name.clone());
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
    let pc = pc::worker(sender.clone(), name.clone());
    // Listings and downloads; dropping the set when the link closes cancels whatever is left.
    let mut file_tasks = tokio::task::JoinSet::new();
    // The phone as a controller: its pad lives while states keep coming (see gamepad.rs).
    let mut pad: Option<(tokio::sync::mpsc::Sender<gamepad::Input>, tokio::task::JoinHandle<()>)> = None;
    // The touchpad over Bluetooth, while its page is open on the phone.
    let mut touchpad: Option<(tokio::sync::mpsc::Sender<pointer::Frame>, tokio::task::JoinHandle<()>)> = None;
    // A new wallpaper reaches the phone's background without waiting for it to ask.
    let wall_task = tokio::spawn(watch_wallpaper(sender.clone()));
    // So the phone can find this PC over Bluetooth once Wi-Fi is gone.
    if let Some(address) = bluetooth::address().await {
        let _ = sender.send(&packet("vyoma.bluetooth", json!({ "address": address }))).await;
    }
    let mut switches = host.permits.subscribe();
    // A refusal goes out at most this often per feature: streams (controller states, touches) repeat.
    let mut refused: std::collections::HashMap<Permit, Instant> = std::collections::HashMap::new();
    loop {
        let (p, from) = tokio::select! {
            got = packets.recv() => match got {
                Some(got) => got,
                None => break,
            },
            Ok(()) = switches.changed() => {
                // Turned off: what it was doing stops now, and the phone hears why.
                for permit in Permit::ALL.into_iter().filter(|&f| !host.permits.allows(phone_id, f)) {
                    let stopped = match permit {
                        Permit::Screen => {
                            screen_input = None;
                            stop(touch_task.take()) | stop(screen_task.take()) | stop(shot_task.take())
                        }
                        Permit::Camera => stop(photo_task.take()),
                        Permit::Webcam => stop(webcam_task.take()) | stop(mic_task.take()),
                        Permit::Control => {
                            let pointer = touchpad.take().is_some_and(|(tx, t)| {
                                if tx.try_send(pointer::Frame::Close).is_err() {
                                    t.abort();
                                }
                                true
                            });
                            pointer | stop(pad.take().map(|(_, t)| t))
                        }
                        Permit::Files => {
                            let busy = !file_tasks.is_empty();
                            file_tasks.abort_all();
                            busy
                        }
                        Permit::Power => false,
                    };
                    if stopped {
                        info!("{name}: {} turned off, stopped", permit.key());
                        let _ = sender.send(&permits::denied(permit, &json!({ "type": "" }))).await;
                    }
                }
                continue;
            }
        };
        while file_tasks.try_join_next().is_some() {}
        let kind = p["type"].as_str().unwrap_or_default();
        if let Some(permit) = Permit::of(kind).filter(|&f| !host.permits.allows(phone_id, f)) {
            let now = Instant::now();
            if refused.get(&permit).is_none_or(|t| now.duration_since(*t) > Duration::from_secs(2)) {
                refused.insert(permit, now);
                info!("{name}: {kind} refused, {} is off for this phone", permit.key());
                let _ = sender.send(&permits::denied(permit, &p)).await;
            }
            continue;
        }
        match kind {
            "vyoma.stats.request" => {
                *lease.lock().unwrap() = Instant::now() + STATS_LEASE;
                if stats_task.as_ref().is_none_or(|t| t.is_finished()) {
                    info!("{name}: streaming stats while the dashboard is open");
                    stats_task = Some(tokio::spawn(stream_stats(sender.clone(), lease.clone(), host.media.clone())));
                }
            }
            "vyoma.screenshot.request" => {
                // One at a time; repeated taps while one is in flight are ignored.
                if shot_task.as_ref().is_none_or(|t| t.is_finished()) {
                    let (sender, host, pinned, name) = (sender.clone(), host.clone(), pinned.to_vec(), name.clone());
                    shot_task = Some(tokio::spawn(async move {
                        match screenshot::capture_and_send(&sender, &host.me, &pinned, &name).await {
                            Ok(()) => info!("{name}: sent a screenshot"),
                            Err(e) => warn!("{name}: screenshot failed: {e:#}"),
                        }
                    }));
                }
            }
            "vyoma.camera.request" => {
                // One at a time, like screenshots.
                if photo_task.as_ref().is_none_or(|t| t.is_finished()) {
                    let (sender, host, pinned, name) = (sender.clone(), host.clone(), pinned.to_vec(), name.clone());
                    photo_task = Some(tokio::spawn(async move {
                        match camera::capture_and_send(&sender, &host.me, &pinned, &name).await {
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
                let (sender, host, pinned, name) = (sender.clone(), host.clone(), pinned.to_vec(), name.clone());
                let (touches, inputs) = tokio::sync::mpsc::unbounded_channel();
                screen_input = Some(touches);
                let fps = screen::fps_for(p["body"]["maxFps"].as_u64());
                screen_task = Some(tokio::spawn(async move {
                    if let Err(e) = screen::stream(&sender, &host.me, &pinned, &name, fps, inputs).await {
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
                let (sender, host, pinned, name, phone, turn) = (sender.clone(), host.clone(), pinned.to_vec(), name.clone(), from, turn_tx.subscribe());
                webcam_task = Some(tokio::spawn(async move {
                    if let Err(e) = webcam::camera(&sender, &host.me, &pinned, phone, &p, turn, &name).await {
                        warn!("{name}: phone camera: {e:#}");
                    }
                }));
            }
            "vyoma.mirror" => {
                // A new stream (the phone restarted it) replaces the window.
                if let Some(t) = mirror_task.take() {
                    t.abort();
                }
                let (sender, host, pinned, name, phone) = (sender.clone(), host.clone(), pinned.to_vec(), name.clone(), from);
                mirror_task = Some(tokio::spawn(async move {
                    if let Err(e) = mirror::show(&sender, &host.me, &pinned, phone, &p, &name).await {
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
                    let (text, sender) = (text.to_string(), sender.clone());
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
                let (host, pinned, name, phone) = (host.clone(), pinned.to_vec(), name.clone(), from);
                sound_task = Some(tokio::spawn(async move {
                    if let Err(e) = mirror::sound(&host.me, &pinned, phone, &p, &name).await {
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
                let (sender, host, pinned, name, phone) = (sender.clone(), host.clone(), pinned.to_vec(), name.clone(), from);
                mic_task = Some(tokio::spawn(async move {
                    let _ = webcam::mic(&sender, &host.me, &pinned, phone, &p, &name).await;
                }));
            }
            "vyoma.screen.touch" => {
                // The viewer's touches as a live stream (over the cable when it's in): one JSON line each.
                if let Some(t) = touch_task.take() {
                    t.abort();
                }
                if let Some(touches) = screen_input.clone() {
                    let (host, pinned, phone) = (host.clone(), pinned.to_vec(), from);
                    touch_task = Some(tokio::spawn(async move {
                        use tokio::io::AsyncBufReadExt;
                        let Ok(stream) = payload::fetch_stream(&host.me, &pinned, phone, &p).await else { return };
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
                let sender = sender.clone();
                tokio::spawn(async move {
                    let reply = match wallpaper::colors().await {
                        Ok(c) => json!({ "cols": wallpaper::COLS, "rows": wallpaper::ROWS, "colors": c }),
                        Err(e) => json!({ "error": format!("{e:#}") }),
                    };
                    let _ = sender.send(&packet("vyoma.wallpaper", reply)).await;
                });
            }
            "vyoma.specs.request" => {
                let sender = sender.clone();
                tokio::spawn(async move {
                    let reply = tokio::task::spawn_blocking(specs::read).await.unwrap_or_else(|e| json!({ "error": e.to_string() }));
                    let _ = sender.send(&packet("vyoma.specs", reply)).await;
                });
            }
            "vyoma.gamepad.state" => {
                let s = gamepad::State::parse(&p["body"]);
                if pad.as_ref().is_none_or(|(_, t)| t.is_finished()) {
                    let (tx, rx) = tokio::sync::mpsc::channel(64);
                    pad = Some((tx, tokio::spawn(gamepad::run(rx, sender.clone(), name.clone()))));
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
                let (sender, up, name) = (sender.clone(), p["body"]["up"].as_bool().unwrap_or(true), name.clone());
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
                touchpad = Some((tx.clone(), tokio::spawn(pointer::serve(sender.clone(), name.clone(), tx, rx))));
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
                let reply = match &host.media {
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
                // Not `?`: leaving here would skip the cleanup below.
                let _ = sender.send(&packet("vyoma.handoff", reply)).await;
            }
            "vyoma.files.list" => {
                let (sender, rel) = (sender.clone(), p["body"]["path"].as_str().unwrap_or_default().to_string());
                file_tasks.spawn(async move {
                    let _ = files::list(&sender, &rel).await;
                });
            }
            "vyoma.files.get" => {
                let (sender, host, pinned, name) = (sender.clone(), host.clone(), pinned.to_vec(), name.clone());
                let rel = p["body"]["path"].as_str().unwrap_or_default().to_string();
                file_tasks.spawn(async move {
                    match files::send(&sender, &host.me, &pinned, &rel, &name).await {
                        Ok(()) => info!("{name}: sent a file"),
                        Err(e) => warn!("{name}: file transfer failed: {e:#}"),
                    }
                });
            }
            "vyoma.files.put" => {
                let (sender, host, pinned, name, phone) = (sender.clone(), host.clone(), pinned.to_vec(), name.clone(), from);
                file_tasks.spawn(async move {
                    match files::receive(&sender, &host.me, &pinned, phone, &p, &name).await {
                        Ok(()) => info!("{name}: received a file"),
                        Err(e) => warn!("{name}: upload failed: {e:#}"),
                    }
                });
            }
            "vyoma.xfer.begin" => {
                let (sender, phone_id) = (sender.clone(), phone_id.to_string());
                file_tasks.spawn(async move {
                    if let Err(e) = xfer::begin(&sender, &phone_id, &p["body"]).await {
                        warn!("couldn't answer a transfer: {e:#}");
                    }
                });
            }
            "vyoma.xfer.data" => {
                let (sender, host, pinned, name, phone) = (sender.clone(), host.clone(), pinned.to_vec(), name.clone(), from);
                file_tasks.spawn(async move {
                    match xfer::data(&sender, &host.me, &pinned, phone, &name, &p).await {
                        Ok(()) => info!("{name}: received a file"),
                        Err(e) => warn!("{name}: transfer stopped: {e:#}"),
                    }
                });
            }
            "vyoma.xfer.end" => xfer::end(&name, &p["body"]),
            "vyoma.away" => host.away.configure(phone_id, &p["body"]),
            "vyoma.pc.request" | "vyoma.pc.set" | "vyoma.pc.power" | "vyoma.pc.wake" => {
                let _ = pc.send(p);
            }
            _ => {}
        }
    }
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
    info!("{name}: link closed");
}


/// Ends [task]; true if it was still going.
fn stop(task: Option<tokio::task::JoinHandle<()>>) -> bool {
    task.is_some_and(|t| {
        let going = !t.is_finished();
        t.abort();
        going
    })
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

