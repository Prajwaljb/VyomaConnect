//! One phone's session: every vyoma.* feature, the same on every platform. The caller owns the link
//! and feeds the packets in; this answers through a [Sender], has the [Platform] do the work, and
//! ends what it started when the packets stop.

use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::sync::mpsc;
use tracing::{info, warn};
use vyoma_core::identity::Identity;
use vyoma_core::link::{Sender, packet};
use vyoma_core::payload;

use vyoma_core::activity::{self, Activity};
use vyoma_core::permits::{self, Permit, Permits};

use crate::platform::{Cx, Feature, GamepadInput, Platform, Started};

/// The wallpaper's colours go as a grid this size.
pub const WALLPAPER_COLS: usize = 4;
pub const WALLPAPER_ROWS: usize = 4;

/// The phone's stats come for this long after it last asked.
const STATS_LEASE: Duration = Duration::from_secs(12);
const STATS_EVERY: Duration = Duration::from_secs(2);

/// Every vyoma.* packet a session can take and send, whatever the platform offers (see
/// [crate::platform::capabilities] for what to tell the phone).
pub fn incoming_all() -> Vec<&'static str> {
    Feature::ALL.iter().flat_map(|f| f.incoming().iter().copied()).collect()
}

pub fn outgoing_all() -> Vec<&'static str> {
    Feature::ALL.iter().flat_map(|f| f.outgoing().iter().copied()).collect::<Vec<_>>().into_iter().chain(["vyoma.denied"]).collect()
}

pub struct Host {
    pub me: Arc<Identity>,
    /// What each phone may do here.
    pub permits: Arc<Permits>,
    /// What phones did here.
    pub activity: Arc<Activity>,
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
pub async fn serve<P: Platform>(platform: Arc<P>, host: Arc<Host>, phone: Phone, sender: Sender, mut packets: mpsc::UnboundedReceiver<(Value, IpAddr)>) {
    platform.install_file_hooks();
    let cx = Cx { id: phone.id.clone(), me: host.me.clone(), pinned: phone.pinned.clone(), name: phone.name.clone(), sender: sender.clone() };
    let (phone_id, pinned, name) = (phone.id.as_str(), phone.pinned.as_slice(), phone.name.clone());
    let lease = Arc::new(Mutex::new(Instant::now()));
    let this = platform.this_computer();
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
    let pc = platform.pc_controls(&cx);
    // Listings and downloads; dropping the set when the link closes cancels whatever is left.
    let mut file_tasks = tokio::task::JoinSet::new();
    // The phone as a controller: its pad lives while states keep coming (see gamepad.rs).
    let mut pad: Option<(tokio::sync::mpsc::Sender<GamepadInput>, tokio::task::JoinHandle<()>)> = None;
    // The touchpad over Bluetooth, while its page is open on the phone.
    let mut touchpad: Option<Started> = None;
    // A new wallpaper reaches the phone's background without waiting for it to ask.
    let wall_task = tokio::spawn(watch_wallpaper(platform.clone(), sender.clone()));
    // So the phone can find this PC over Bluetooth once Wi-Fi is gone.
    if let Some(address) = platform.bluetooth_address().await {
        let _ = sender.send(&packet("vyoma.bluetooth", json!({ "address": address }))).await;
    }
    let log = {
        let (activity, id, name) = (host.activity.clone(), phone_id.to_owned(), name.clone());
        move |kind: activity::Kind, text: String| activity.add(&id, &name, kind, text)
    };
    let log = Arc::new(log);
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
                            let pointer = touchpad.take().map(Started::end).is_some();
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
                log("refused", format!("Refused: \u{201c}{}\u{201d} is off", permit.name()));
                let _ = sender.send(&permits::denied(permit, &p)).await;
            }
            continue;
        }
        match kind {
            "vyoma.stats.request" => {
                *lease.lock().unwrap() = Instant::now() + STATS_LEASE;
                if stats_task.as_ref().is_none_or(|t| t.is_finished()) {
                    info!("{name}: streaming stats while the dashboard is open");
                    stats_task = Some(tokio::spawn(stream_stats(platform.clone(), sender.clone(), lease.clone())));
                }
            }
            "vyoma.screenshot.request" => {
                // One at a time; repeated taps while one is in flight are ignored.
                if shot_task.as_ref().is_none_or(|t| t.is_finished()) {
                    let (platform, cx, name, log) = (platform.clone(), cx.clone(), name.clone(), log.clone());
                    shot_task = Some(tokio::spawn(async move {
                        match platform.screenshot(&cx).await {
                            Ok(()) => {
                                info!("{name}: sent a screenshot");
                                log("screenshot", "Took a screenshot".into());
                            }
                            Err(e) => warn!("{name}: screenshot failed: {e:#}"),
                        }
                    }));
                }
            }
            "vyoma.camera.request" => {
                // One at a time, like screenshots.
                if photo_task.as_ref().is_none_or(|t| t.is_finished()) {
                    let (platform, cx, name, log) = (platform.clone(), cx.clone(), name.clone(), log.clone());
                    photo_task = Some(tokio::spawn(async move {
                        match platform.camera_photo(&cx).await {
                            Ok(()) => {
                                info!("{name}: sent a camera photo");
                                log("photo", "Took a photo with the camera".into());
                            }
                            Err(e) => warn!("{name}: camera photo failed: {e:#}"),
                        }
                    }));
                }
            }
            "vyoma.screen.request" => {
                // A new request replaces a cast still running (the phone reopened the viewer).
                if !stop(screen_task.take()) {
                    log("screen", "Watched the screen".into());
                }
                let (platform, cx, name) = (platform.clone(), cx.clone(), name.clone());
                let (touches, inputs) = tokio::sync::mpsc::unbounded_channel();
                screen_input = Some(touches);
                let fps = platform.screen_fps(p["body"]["maxFps"].as_u64());
                screen_task = Some(tokio::spawn(async move {
                    if let Err(e) = platform.screen(&cx, fps, inputs).await {
                        warn!("{name}: screen cast failed: {e:#}");
                    }
                }));
            }
            "vyoma.webcam" => {
                // A new stream replaces one still running (the phone switched cameras).
                if !stop(webcam_task.take()) {
                    log("webcam", "Used as webcam".into());
                }
                turn_tx.send_replace(p["body"]["rotate"].as_u64().unwrap_or(0) as u32);
                let (platform, cx, name, phone, turn) = (platform.clone(), cx.clone(), name.clone(), from, turn_tx.subscribe());
                webcam_task = Some(tokio::spawn(async move {
                    if let Err(e) = platform.webcam(&cx, phone, &p, turn).await {
                        warn!("{name}: phone camera: {e:#}");
                    }
                }));
            }
            "vyoma.mirror" => {
                // A new stream (the phone restarted it) replaces the window.
                if let Some(t) = mirror_task.take() {
                    t.abort();
                }
                let (platform, cx, name, phone) = (platform.clone(), cx.clone(), name.clone(), from);
                mirror_task = Some(tokio::spawn(async move {
                    if let Err(e) = platform.mirror(&cx, phone, &p).await {
                        warn!("{name}: mirror: {e:#}");
                    }
                }));
            }
            "vyoma.mirror.copy" => {
                if let Some(text) = p["body"]["text"].as_str() {
                    let (platform, text) = (platform.clone(), text.to_string());
                    tokio::spawn(async move { platform.mirror_copy(&text).await });
                }
            }
            "vyoma.paste" => {
                if let Some(text) = p["body"]["text"].as_str() {
                    let (platform, text, sender) = (platform.clone(), text.to_string(), sender.clone());
                    tokio::spawn(async move {
                        let body = match platform.paste(&text).await {
                            Ok(()) => json!({}),
                            Err(e) => json!({ "error": format!("{e:#}") }),
                        };
                        let _ = sender.send(&packet("vyoma.paste.ready", body)).await;
                    });
                }
            }
            "vyoma.mirror.volume" => {
                let up = p["body"]["up"].as_bool().unwrap_or(true);
                let platform = platform.clone();
                tokio::spawn(async move { platform.mirror_volume(up).await });
            }
            "vyoma.mirror.audio" => {
                if let Some(t) = sound_task.take() {
                    t.abort();
                }
                let (platform, cx, name, phone) = (platform.clone(), cx.clone(), name.clone(), from);
                sound_task = Some(tokio::spawn(async move {
                    if let Err(e) = platform.mirror_sound(&cx, phone, &p).await {
                        warn!("{name}: phone sound: {e:#}");
                    }
                }));
            }
            "vyoma.webcam.turn" => {
                turn_tx.send_replace(p["body"]["rotate"].as_u64().unwrap_or(0) as u32);
            }
            "vyoma.webcam.mic" => {
                if !stop(mic_task.take()) {
                    log("webcam", "Used as microphone".into());
                }
                let (platform, cx, phone) = (platform.clone(), cx.clone(), from);
                mic_task = Some(tokio::spawn(async move {
                    let _ = platform.webcam_mic(&cx, phone, &p).await;
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
                let (platform, sender) = (platform.clone(), sender.clone());
                tokio::spawn(async move {
                    let reply = match platform.wallpaper_colors().await {
                        Ok(c) => json!({ "cols": WALLPAPER_COLS, "rows": WALLPAPER_ROWS, "colors": c }),
                        Err(e) => json!({ "error": format!("{e:#}") }),
                    };
                    let _ = sender.send(&packet("vyoma.wallpaper", reply)).await;
                });
            }
            "vyoma.specs.request" => {
                let (platform, sender) = (platform.clone(), sender.clone());
                tokio::spawn(async move {
                    let reply = platform.specs().await;
                    let _ = sender.send(&packet("vyoma.specs", reply)).await;
                });
            }
            "vyoma.gamepad.state" => {
                let s = vyoma_core::gamepad::State::parse(&p["body"]);
                if pad.as_ref().is_none_or(|(_, t)| t.is_finished()) {
                    log("control", "Used as game controller".into());
                    let (tx, rx) = tokio::sync::mpsc::channel(64);
                    let (platform, cx) = (platform.clone(), cx.clone());
                    pad = Some((tx, tokio::spawn(async move { platform.gamepad(&cx, rx).await })));
                }
                if let Some((tx, _)) = &pad {
                    // Full means the PC is behind; the next state carries everything anyway.
                    let _ = tx.try_send(GamepadInput::State(s));
                }
            }
            "vyoma.usb.accessory" => {
                // The phone's controller is on a cable without USB debugging: accessory mode.
                let (platform, name) = (platform.clone(), name.clone());
                // "open": false means the phone couldn't open an accessory it already sees (its permission
                // went with a reinstall): switch it again so Android hands the accessory over afresh.
                let reswitch = !p["body"]["open"].as_bool().unwrap_or(false);
                tokio::task::spawn_blocking(move || {
                    if let Err(e) = platform.usb_accessory(reswitch) {
                        warn!("{name}: controller over the cable: {e:#}");
                    }
                });
            }
            "vyoma.desktop.step" => {
                let (platform, sender, up, name) = (platform.clone(), sender.clone(), p["body"]["up"].as_bool().unwrap_or(true), name.clone());
                tokio::spawn(async move {
                    let reply = match platform.desktop_step(up).await {
                        Ok(view) => json!({ "view": view }),
                        Err(e) => {
                            warn!("{name}: desktop gesture failed: {e:#}");
                            json!({ "error": format!("{e:#}") })
                        }
                    };
                    let _ = sender.send(&packet("vyoma.desktop.stepped", reply)).await;
                });
            }
            "vyoma.desktop.switch" => {
                let (platform, right, name) = (platform.clone(), p["body"]["right"].as_bool().unwrap_or(true), name.clone());
                tokio::spawn(async move {
                    if let Err(e) = platform.desktop_switch(right).await {
                        warn!("{name}: desktop switch failed: {e:#}");
                    }
                });
            }
            "vyoma.pointer.request" => {
                // A new request replaces the last (the radio dropped and the phone came back).
                match touchpad.take() {
                    Some(old) => {
                        let _ = (old.close)();
                    }
                    None => log("touchpad", "Used as touchpad".into()),
                }
                touchpad = platform.touchpad(&cx);
            }
            "vyoma.pointer.close" => {
                if let Some(old) = touchpad.take() {
                    let _ = (old.close)();
                }
            }
            "vyoma.gamepad.close" => {
                if let Some((tx, _)) = &pad {
                    let _ = tx.try_send(GamepadInput::Close);
                }
            }
            "vyoma.handoff.request" => {
                let reply = match platform.handoff().await {
                    None => json!({ "error": "nothing with a link is playing on the PC" }),
                    Some(h) => {
                        // The phone takes over: stop it here so they don't play over each other.
                        if p["body"]["pause"].as_bool().unwrap_or(true) {
                            platform.pause_handoff(&h).await;
                        }
                        json!({ "url": h.url, "title": h.title, "position": h.position_ms, "exact": h.exact })
                    }
                };
                // Not `?`: leaving here would skip the cleanup below.
                let _ = sender.send(&packet("vyoma.handoff", reply)).await;
            }
            "vyoma.files.list" => {
                let (sender, rel) = (sender.clone(), p["body"]["path"].as_str().unwrap_or_default().to_string());
                file_tasks.spawn(async move {
                    let _ = vyoma_core::files::list(&sender, &rel).await;
                });
            }
            "vyoma.files.get" => {
                let (sender, host, pinned, name, log) = (sender.clone(), host.clone(), pinned.to_vec(), name.clone(), log.clone());
                let rel = p["body"]["path"].as_str().unwrap_or_default().to_string();
                file_tasks.spawn(async move {
                    match vyoma_core::files::send(&sender, &host.me, &pinned, &rel, &name).await {
                        Ok(()) => {
                            info!("{name}: sent a file");
                            log("files", format!("Copied \u{201c}{}\u{201d} from {this}", activity::file_name(&rel)));
                        }
                        Err(e) => warn!("{name}: file transfer failed: {e:#}"),
                    }
                });
            }
            "vyoma.files.put" => {
                let (sender, host, pinned, name, phone, log) = (sender.clone(), host.clone(), pinned.to_vec(), name.clone(), from, log.clone());
                file_tasks.spawn(async move {
                    match vyoma_core::files::receive(&sender, &host.me, &pinned, phone, &p, &name).await {
                        Ok(()) => {
                            info!("{name}: received a file");
                            log("files", format!("Saved \u{201c}{}\u{201d} to {this}", activity::file_name(p["body"]["name"].as_str().unwrap_or("a file"))));
                        }
                        Err(e) => warn!("{name}: upload failed: {e:#}"),
                    }
                });
            }
            "vyoma.xfer.begin" => {
                let (sender, phone_id) = (sender.clone(), phone_id.to_string());
                file_tasks.spawn(async move {
                    if let Err(e) = vyoma_core::xfer::begin(&sender, &phone_id, &p["body"]).await {
                        warn!("couldn't answer a transfer: {e:#}");
                    }
                });
            }
            "vyoma.xfer.data" => {
                let (sender, host, pinned, name, phone, log) = (sender.clone(), host.clone(), pinned.to_vec(), name.clone(), from, log.clone());
                file_tasks.spawn(async move {
                    match vyoma_core::xfer::data(&sender, &host.me, &pinned, phone, &name, &p).await {
                        Ok(saved) => {
                            info!("{name}: received a file");
                            if let Some(file) = saved {
                                log("files", format!("Sent \u{201c}{file}\u{201d} to {this}"));
                            }
                        }
                        Err(e) => warn!("{name}: transfer stopped: {e:#}"),
                    }
                });
            }
            "vyoma.xfer.end" => {
                if let Some((folder, count)) = vyoma_core::xfer::end(&name, &p["body"]) {
                    log("files", format!("Sent the folder \u{201c}{folder}\u{201d} to {this} ({count} {})", if count == 1 { "file" } else { "files" }));
                }
            }
            "vyoma.away" => platform.away(phone_id, &p["body"]),
            "vyoma.pc.request" | "vyoma.pc.set" | "vyoma.pc.power" | "vyoma.pc.wake" => {
                // Before it's done: a shut down may not leave time after.
                match kind {
                    "vyoma.pc.wake" => log("power", platform.wake_text().into()),
                    "vyoma.pc.power" => {
                        let what = match p["body"]["action"].as_str().unwrap_or_default() {
                            "sleep" => format!("Put {this} to sleep"),
                            "restart" => format!("Restarted {this}"),
                            "shutdown" => format!("Shut down {this}"),
                            "logout" => format!("Logged out of {this}"),
                            _ => String::new(),
                        };
                        if !what.is_empty() {
                            log("power", what);
                        }
                    }
                    _ => {}
                }
                if let Some(pc) = &pc {
                    let _ = pc.send(p);
                }
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
    if let Some(t) = touchpad {
        t.end();
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
async fn watch_wallpaper<P: Platform>(platform: Arc<P>, sender: Sender) {
    // Off the runtime, and given up on after a while: on some desktops it asks another program
    // (gsettings, hyprctl, swww), which can hang with its daemon.
    let signature = || {
        let platform = platform.clone();
        // None: it couldn't tell this time; Some(None): this computer has no wallpaper to follow.
        async move { tokio::time::timeout(Duration::from_secs(10), tokio::task::spawn_blocking(move || platform.wallpaper_signature())).await.ok().and_then(Result::ok) }
    };
    let first = signature().await;
    if first == Some(None) {
        return;
    }
    let mut last = first.flatten();
    let mut tick = tokio::time::interval(Duration::from_secs(3));
    // A slow look (a stuck helper) doesn't make the next ones pile up behind it.
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        let Some(now) = signature().await else { continue };
        if now == last {
            continue;
        }
        last = now;
        // Plasma writes the setting a moment before it's done switching; give it a breath.
        tokio::time::sleep(Duration::from_millis(400)).await;
        if let Ok(c) = platform.wallpaper_colors().await {
            info!("wallpaper changed: sending its colours");
            if sender.send(&packet("vyoma.wallpaper", json!({ "cols": WALLPAPER_COLS, "rows": WALLPAPER_ROWS, "colors": c }))).await.is_err() {
                return;
            }
        }
    }
}

/// Stats, plus `handoff`: the title of a web video the phone could continue (absent otherwise),
/// so the phone only offers "Continue on phone" when it would work; and `camera`: whether a photo
/// can be taken (false behind a closed lid with no other camera); and `ac`: on a charger.
async fn stream_stats<P: Platform>(platform: Arc<P>, sender: Sender, lease: Arc<Mutex<Instant>>) {
    let mut stats = match platform.stats() {
        Ok(s) => s,
        Err(e) => return warn!("stats unavailable: {e:#}"),
    };
    let mut tick = tokio::time::interval(STATS_EVERY);
    while Instant::now() < *lease.lock().unwrap() {
        tick.tick().await;
        let mut body = stats.sample().await;
        if let Some(title) = platform.handoff_offer() {
            body["handoff"] = json!(title);
        }
        // So the phone can say "Camera unavailable" up front instead of after a failed tap.
        let (camera, lid) = platform.camera_state().await;
        body["camera"] = json!(camera);
        if !camera {
            body["cameraLid"] = json!(lid);
        }
        // Stock battery packets only say "charging": a laptop on its charger that holds at a
        // charge limit (or is full) reads as on battery without this.
        if let Some(ac) = platform.on_ac().await {
            body["ac"] = json!(ac);
        }
        if sender.send(&packet("vyoma.stats", body)).await.is_err() {
            return;
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::capabilities;

    /// A computer that can only tell its specs and take screenshots.
    struct Small;

    impl Platform for Small {
        fn supports(&self, f: Feature) -> bool {
            matches!(f, Feature::Specs | Feature::Screenshot)
        }

        async fn specs(&self) -> Value {
            json!({ "cpu": "test" })
        }
    }

    fn host(dir: &std::path::Path) -> Arc<Host> {
        let me = Arc::new(Identity::load_or_create_in(dir).unwrap());
        Arc::new(Host { me, permits: Arc::new(Permits::default()), activity: Arc::new(Activity::default()) })
    }

    async fn next(out: &mut mpsc::UnboundedReceiver<Value>) -> Value {
        tokio::time::timeout(Duration::from_secs(5), out.recv()).await.expect("a reply in time").expect("a reply")
    }

    #[test]
    fn only_what_the_platform_does_is_offered() {
        let (incoming, outgoing) = capabilities(&Small);
        assert_eq!(incoming, ["vyoma.specs.request", "vyoma.screenshot.request"]);
        assert_eq!(outgoing, ["vyoma.specs", "vyoma.screenshot", "vyoma.denied"]);
    }

    #[test]
    fn every_packet_belongs_to_one_feature() {
        for kind in incoming_all() {
            assert_eq!(Feature::ALL.iter().filter(|f| f.incoming().contains(&kind)).count(), 1, "{kind}");
        }
    }

    #[tokio::test]
    async fn asks_the_platform_and_answers_the_phone() {
        let dir = std::env::temp_dir().join(format!("vyoma-engine-test-{}", std::process::id()));
        let (to_phone, mut out) = mpsc::unbounded_channel();
        let (packets, rx) = mpsc::unbounded_channel();
        let phone = Phone { id: "p".into(), name: "Phone".into(), pinned: Vec::new() };
        let session = tokio::spawn(serve(Arc::new(Small), host(&dir), phone, Sender::queue(to_phone), rx));
        let from = IpAddr::from([127, 0, 0, 1]);
        packets.send((packet("vyoma.specs.request", json!({})), from)).unwrap();
        let reply = next(&mut out).await;
        assert_eq!((reply["type"].as_str(), &reply["body"]), (Some("vyoma.specs"), &json!({ "cpu": "test" })));
        drop(packets);
        session.await.unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_switch_turned_off_refuses_with_its_reason() {
        let dir = std::env::temp_dir().join(format!("vyoma-engine-test-off-{}", std::process::id()));
        let host = host(&dir);
        host.permits.set("p", Permit::Screen, false).unwrap();
        let (to_phone, mut out) = mpsc::unbounded_channel();
        let (packets, rx) = mpsc::unbounded_channel();
        let phone = Phone { id: "p".into(), name: "Phone".into(), pinned: Vec::new() };
        let session = tokio::spawn(serve(Arc::new(Small), host, phone, Sender::queue(to_phone), rx));
        packets.send((packet("vyoma.screenshot.request", json!({})), IpAddr::from([127, 0, 0, 1]))).unwrap();
        let reply = next(&mut out).await;
        assert_eq!(reply["type"], "vyoma.denied");
        assert_eq!(reply["body"]["request"], "vyoma.screenshot.request");
        drop(packets);
        session.await.unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }
}
