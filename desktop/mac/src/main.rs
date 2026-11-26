//! Vyoma's PC side on macOS (docs/PLAN.md section 10, phase 5), on the same core as the Linux app:
//! it finds and links with phones, pairs after asking (a dialog with the code to compare), and
//! serves the stock plugins: ping, battery both ways, find my phone, the screen lock, the clipboard
//! both ways, sharing (files into ~/Downloads, links opened, text onto the clipboard), Music and
//! Spotify and the volume ([media]), the touchpad and keyboard ([input]), the phone's
//! notifications, and the commands set up in the window. Vyoma's extras: [vyoma] (stats, power,
//! files), [screen] (screenshots, the live screen on the phone), [mirror] (the phone's screen in a
//! window here), paste, the Spaces gestures, and the USB cable ([usb]).
//!
//! Vyoma's window talks to it over a local socket ([control]).
//! Built and tested on GitHub's macOS runners only; not yet tried on a real Mac.

mod away;
mod bluetooth;
mod brightness;
mod calls;
mod control;
mod finder;
mod handoff;
mod input;
mod launchd;
mod mac;
mod media;
mod mirror;
#[cfg(target_os = "macos")]
mod notify;
mod platform;
mod presenter;
mod camera;
mod screen;
mod update;
mod usb;
mod vyoma;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Result, bail};
use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc};
use vyoma_core::activity::Activity;
use vyoma_core::clipboard::{CLIPBOARD, CLIPBOARD_CONNECT, Clipboard, MAX_TEXT};
use vyoma_engine::commands::{self, Commands, RUNCOMMAND, RUNCOMMAND_REQUEST};
use vyoma_core::device::{Command, Event, Hub};
use vyoma_core::identity::Identity;
use vyoma_core::lan::{Lan, Ports, Rules, UDP_PORT};
use vyoma_core::messages::{self, Store};
use vyoma_core::link::packet;
use vyoma_core::permits::{Permit, Permits};
use vyoma_core::share::{SHARE, SHARE_UPDATE};
use vyoma_core::trust::TrustStore;
use vyoma_engine::dispatch::{self, Route, Touch};
use vyoma_engine::session::{Host, Phone, phone_name};
use vyoma_engine::window::{Answer, CHANGING, Device, Devices, Side};

pub use vyoma_engine::stock::{BATTERY, FIND, LOCK_REQUEST, PING, battery_packet};
use vyoma_engine::stock::{self, Desk};
pub use vyoma_engine::notifications::NOTIFICATION;
use vyoma_engine::notifications::{self, Incoming};
pub const KEYBOARDSTATE: &str = "kdeconnect.mousepad.keyboardstate";

/// What we take and send. No lock state outward: macOS doesn't tell other apps whether it's locked.
const INCOMING: &[&str] = &[
    PING, BATTERY, FIND, LOCK_REQUEST, CLIPBOARD, CLIPBOARD_CONNECT, SHARE, SHARE_UPDATE,
    media::MPRIS_REQUEST, media::VOLUME_REQUEST, RUNCOMMAND_REQUEST, NOTIFICATION, input::MOUSEPAD_REQUEST,
    vyoma_core::telephony::TELEPHONY, presenter::PRESENTER,
    messages::MESSAGES, messages::CONTACTS_UIDS_REPLY, messages::CONTACTS_VCARDS_REPLY, messages::UNLOCKED, messages::LOCKED,
];
const OUTGOING: &[&str] = &[
    PING, BATTERY, FIND, CLIPBOARD, CLIPBOARD_CONNECT, SHARE, SHARE_UPDATE,
    media::MPRIS, media::VOLUME, RUNCOMMAND, input::MOUSEPAD_ECHO, KEYBOARDSTATE,
    messages::REQUEST, messages::REQUEST_CONVERSATIONS, messages::REQUEST_CONVERSATION, messages::CONTACTS_UIDS, messages::CONTACTS_VCARDS,
    messages::UNLOCK, messages::LOCK, vyoma_engine::calls::DIAL, vyoma::LEAVING,
];

/// This release, as the Linux app reads it: the repo's one version.
const VERSION: &str = include_str!("../../../VERSION");
const AGENT_LOCK_TRIES: u32 = 24;
const AGENT_LOCK_WAIT: Duration = Duration::from_millis(250);

/// How often the clipboard is looked at (macOS sends no change events to a program like this one)
/// and the battery read.
const CLIPBOARD_EVERY: Duration = Duration::from_secs(1);
const BATTERY_EVERY: Duration = Duration::from_secs(60);
/// How often the screen's lock and unused texts are looked at (texts lock again with the screen).
const LOCK_EVERY: Duration = Duration::from_secs(3);

/// The switches this Mac has, in the window's order: what it serves so far.
const PERMITS: [Permit; 5] = [Permit::Screen, Permit::Control, Permit::Files, Permit::Camera, Permit::Power];

/// A switch as the Mac's window shows it: the Linux words, less what a Mac can't do.
fn permit_words(p: Permit) -> (&'static str, &'static str) {
    match p {
        Permit::Control => ("Touchpad and keyboard", "Move the pointer, type, paste, switch Spaces"),
        Permit::Power => ("Wake, sleep and shut down", "Also log out and restart"),
        Permit::Camera => ("Take photos with this Mac's camera", "Photos land on the phone"),
        other => (other.name(), other.detail()),
    }
}
/// Finder's Quick Action runs the helper with this and the selected files ([finder]).
const SEND_ANY: &str = "--send-any";
/// How long [SEND_ANY] waits for Vyoma to start, in half seconds.
const SEND_ANY_WAIT_STEPS: u32 = 20;

/// How long a pairing question stays up: the phone gives up after 30 s too.
const PAIR_TIMEOUT: Duration = Duration::from_secs(30);

fn main() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("--install") => launchd::install(),
        Some("--uninstall") => {
            finder::remove();
            launchd::uninstall()
        }
        Some(SEND_ANY) => send_any(std::env::args().skip(2).collect()),
        Some(mirror::WINDOW_ARG) => mirror::window(&std::env::args().skip(2).collect::<Vec<_>>()),
        Some(presenter::WINDOW_ARG) => presenter::window(),
        Some(other) => bail!("unknown option {other}: try --install or --uninstall"),
        None => run(std::env::var_os("VYOMA_DIR").map(PathBuf::from).unwrap_or_else(launchd::support_dir)),
    }
}

/// A device as this Mac knows it.
/// Waits for this Mac's one-agent lock. Two agents would both answer the phone under one identity,
/// each with its own idea of who is paired, and the network doesn't stop them: the second simply
/// listens on the next port. The wait covers an agent still saying goodbye after a restart.
async fn only_agent(dir: &std::path::Path) -> Result<std::fs::File> {
    for _ in 0..AGENT_LOCK_TRIES {
        if let Some(lock) = launchd::lock(dir)? {
            return Ok(lock);
        }
        tokio::time::sleep(AGENT_LOCK_WAIT).await;
    }
    bail!("Vyoma is already running on this Mac")
}

#[tokio::main]
async fn run(dir: PathBuf) -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "error,vyoma_mac=info,vyoma_core=info,vyoma_engine=info".into()))
        .init();
    vyoma_core::panics::log();
    vyoma_core::identity::use_dir(dir.clone());
    let _only_agent = only_agent(&dir).await?;
    #[cfg(target_os = "macos")]
    notify::start();
    let me = Arc::new(Identity::load_or_create_in(&dir)?);
    let trust = TrustStore::open(&dir)?;
    let mut devices: HashMap<String, Device> = trust
        .ids()
        .map(|id| (id.clone(), Device { name: trust.get(id).unwrap().name.clone(), paired: true, ..Default::default() }))
        .collect();

    let mut battery = mac::battery().await;
    let (events_tx, mut events) = mpsc::unbounded_channel();
    let hub = Hub::new(me.cert.to_vec(), trust, events_tx);
    let linked = hub.linked();
    let rules = Rules { admits: Box::new(|_| true), linked: Box::new(move |id| linked.lock().unwrap().contains(id)) };
    let lan = Lan::bind(me.clone(), identity(&me, mac::name().await, battery.is_some()), rules, Ports::default()).await?;
    let (links_tx, links) = mpsc::channel(16);
    let (commands, commands_rx) = mpsc::unbounded_channel();
    tokio::spawn(lan.clone().run(links_tx));
    tokio::spawn(hub.run(links, commands_rx));
    let everyone = [SocketAddr::from(([255, 255, 255, 255], UDP_PORT))];
    lan.announce(&everyone).await?;
    tracing::info!("vyoma-mac is up as {}", me.device_id);

    // The window's socket.
    let (requests_tx, mut requests) = mpsc::channel::<control::Request>(16);
    let (changes, _) = broadcast::channel::<Value>(64);
    {
        let (path, changes, requests) = (control::socket_path(&dir), changes.clone(), requests_tx.clone());
        tokio::spawn(async move {
            if let Err(e) = control::serve(path, requests, changes).await {
                tracing::warn!("no window socket: {e:#}");
            }
        });
    }
    let run_commands = Commands::new(&dir);
    // What phones may do here, and what they did.
    let permits = Arc::new(Permits::load(dir.join("permissions.json")));
    let activity = Arc::new(Activity::load(dir.join("activity.json")));
    {
        let (changes, mut ticks) = (changes.clone(), activity.subscribe());
        tokio::spawn(async move {
            while ticks.changed().await.is_ok() {
                let _ = changes.send(json!({ "event": "ActivityChanged" }));
            }
        });
    }
    let mut touchpad = dispatch::Touchpad::default();
    let calls = calls::start();
    let walk_away = Arc::new(away::Away::default());
    // Vyoma's own features: the engine runs each paired phone's session, the Mac does the work.
    let mac_side = Arc::new(platform::Mac::new(brightness::start(commands.clone()), walk_away.clone()));
    let host = Arc::new(Host { me: me.clone(), permits: permits.clone(), activity: activity.clone() });
    let mut sessions: HashMap<String, mpsc::UnboundedSender<(Value, std::net::IpAddr)>> = HashMap::new();
    let mut pointer_dot = presenter::Presenter::default();
    // The phone's texts, in memory only, for the window's Messages page.
    let texts = Arc::new(Store::default());
    {
        let (changes, mut ticks) = (changes.clone(), texts.subscribe());
        tokio::spawn(async move {
            while ticks.changed().await.is_ok() {
                let _ = changes.send(json!({ "event": "MessagesChanged" }));
            }
        });
    }
    // Newer releases with a Mac app, once a day.
    let updates = update::load(&dir, VERSION);
    {
        let changes = changes.clone();
        tokio::spawn(update::run(updates.clone(), move || {
            let _ = changes.send(json!({ "event": "Changed" }));
        }));
    }
    // Inside Vyoma.app, pairing is asked in its window (with Accept and Reject by the code), as on
    // Linux; the bare binary asks with a dialog.
    let exe = std::env::current_exe().ok();
    let in_app = exe.as_deref().and_then(launchd::app_of).is_some();
    // Finder's "Send to Phone", pointing at this copy of the app (not one macOS runs from a download).
    if let Some(exe) = exe.as_deref().filter(|e| launchd::app_of(e).is_some_and(finder::lasting)) {
        match finder::install(exe) {
            Ok(true) => tracing::info!("Finder's {} is in place", finder::NAME),
            Ok(false) => {}
            Err(e) => tracing::warn!("no {} in Finder: {e:#}", finder::NAME),
        }
    }
    // The phone's files in the window (its file server, over SFTP).
    let files = vyoma_engine::phonefiles::PhoneFiles::new(&me, commands.clone());
    let window = Window {
        me: me.clone(),
        lan: lan.clone(),
        commands: commands.clone(),
        changes: changes.clone(),
        everyone: everyone.to_vec(),
        run: run_commands.clone(),
        texts: texts.clone(),
        permits: permits.clone(),
        activity: activity.clone(),
        updates,
        clip_seen: vyoma_core::adb::ClipboardSeen::load(&dir),
        side: MacSide { requests: requests_tx },
    };

    // Players and the volume, read while a phone that takes them is linked.
    let media_wanted = Arc::new(AtomicBool::new(false));
    let (media_tx, media_rx) = mpsc::unbounded_channel::<(String, Value)>();
    let (media_out_tx, mut media_out) = mpsc::unbounded_channel::<(Option<String>, Value)>();
    tokio::spawn(media::run(media_wanted.clone(), media_rx, media_out_tx));
    let mut pointer = input::Pointer::default();
    // The USB cable: phones plugged in with USB debugging get the adb tunnel and grants.
    usb::start(lan.tcp_port());
    bluetooth::start(lan.tcp_port(), tokio::runtime::Handle::current());
    {
        let changes = changes.clone();
        vyoma_core::adb::on_clipboard_change(Box::new(move || {
            let _ = changes.send(json!({ "event": "Changed" }));
        }));
    }

    let send = |id: &str, p: Value| {
        let _ = commands.send(Command::Send(id.to_owned(), p));
    };

    // The clipboard, looked at every second; only a change goes on.
    let (copies_tx, mut copies) = mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        let mut last = mac::clipboard().await;
        let mut tick = tokio::time::interval(CLIPBOARD_EVERY);
        loop {
            tick.tick().await;
            let now = mac::clipboard().await;
            if now != last {
                last = now.clone();
                if let Some(text) = now.filter(|t| t.len() <= MAX_TEXT)
                    && copies_tx.send(text).is_err()
                {
                    break;
                }
            }
        }
    });
    let mut clip = Clipboard::default();
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut battery_tick = tokio::time::interval(BATTERY_EVERY);
    battery_tick.tick().await;
    let mut lock_tick = tokio::time::interval(LOCK_EVERY);
    let mut screen_locked = mac::screen_locked();

    // Answers to the pairing questions on screen, and the Found it of a ring.
    let (answers_tx, mut answers) = mpsc::unbounded_channel::<(String, bool)>();
    let mut prompts: HashMap<String, tokio::task::JoinHandle<()>> = HashMap::new();
    let (found_tx, mut found) = mpsc::unbounded_channel::<()>();
    let mut ringer = mac::Ringer::default();
    let arrivals = Arc::new(stock::Arrivals::default());
    // Told again on a new network (the phone's hotspot, another Wi-Fi) and after sleep: before,
    // only at start, so phones found this Mac only when they next looked (15 min, screen off).
    let (moved_tx, mut moved) = mpsc::unbounded_channel();
    tokio::spawn(vyoma_core::networks::follow(moved_tx));

    loop {
        tokio::select! {
            Some(why) = moved.recv() => {
                let mut to = everyone.to_vec();
                to.extend(vyoma_core::networks::broadcasts().into_iter().map(|b| SocketAddr::from((b, UDP_PORT))));
                to.extend(devices.values().filter(|d| d.paired).filter_map(|d| d.remote).map(|ip| SocketAddr::new(ip, UDP_PORT)));
                tracing::info!("{why}: telling the phones");
                if let Err(e) = lan.announce(&to).await {
                    tracing::warn!("couldn't announce: {e:#}");
                }
            }
            Some(event) = events.recv() => { if changes_details(&event) { let _ = changes.send(json!({ "event": "Changed" })); } let event_was_link = matches!(event, Event::Linked { .. } | Event::Gone { .. } | Event::Paired { .. } | Event::Unpaired { .. }); match event {
                Event::Linked { id, name, paired, incoming, cert, remote, .. } => {
                    let d = devices.entry(id.clone()).or_default();
                    *d = Device { name, paired, linked: true, incoming, cert, remote: Some(remote), battery: d.battery, ..Default::default() };
                    tracing::info!("{id}: linked{}", if paired { ", paired" } else { "" });
                    let (back, phone) = (walk_away.clone(), id.clone());
                    tokio::spawn(async move { back.linked(&phone).await });
                    if paired {
                        for p in started(d, battery, &clip) {
                            send(&id, p);
                        }
                        start_session(&mut sessions, &mac_side, &host, &commands, &id, d);
                    }
                }
                Event::Gone { id } => {
                    if let Some(d) = devices.get_mut(&id) {
                        d.linked = false;
                    }
                    files.forget(&id);
                    // Ends everything the phone's session started.
                    sessions.remove(&id);
                    texts.gone(&id);
                    pointer_dot.stop();
                    walk_away.unlinked(&id);
                    let _ = calls.send(calls::Order::Gone(id));
                }
                Event::Routes { id, remotes } => {
                    if let Some(d) = devices.get_mut(&id) {
                        d.remote = vyoma_core::adb::best_route(&remotes, !vyoma_core::adb::cable_ports().is_empty());
                    }
                }
                Event::Prompt { id, name, key } => {
                    let d = devices.entry(id.clone()).or_default();
                    (d.key, d.asking) = (key.clone(), true);
                    if in_app {
                        mac::open_app();
                        mac::notify(&format!("{name} wants to pair"), &format!("Pair in Vyoma if the phone shows the code {key}.")).await;
                    } else {
                        let answers = answers_tx.clone();
                        let asked = id.clone();
                        let text = format!("{name} wants to pair with this Mac.\n\nPair only if the phone shows this same code:\n\n{key}");
                        let task = tokio::spawn(async move {
                            let yes = match mac::ask("Vyoma", &text, "Reject", "Pair", PAIR_TIMEOUT) {
                                Ok(child) => child.wait_with_output().await.is_ok_and(|out| mac::answered_yes(&out.stdout, "Pair")),
                                Err(e) => {
                                    tracing::warn!("couldn't ask about pairing: {e:#}");
                                    false
                                }
                            };
                            let _ = answers.send((asked, yes));
                        });
                        // A second request replaces the first question.
                        if let Some(old) = prompts.insert(id, task) {
                            old.abort();
                        }
                    }
                }
                // Over before it was answered (the phone gave up): aborting drops the dialog's process.
                Event::PairingOver { id } => {
                    if let Some(task) = prompts.remove(&id) {
                        task.abort();
                    }
                    if let Some(d) = devices.get_mut(&id) {
                        (d.key, d.asking) = (String::new(), false);
                    }
                }
                Event::Paired { id, name, .. } => {
                    // Answered in the window: the dialog goes.
                    if let Some(task) = prompts.remove(&id) {
                        task.abort();
                    }
                    let d = devices.entry(id.clone()).or_default();
                    d.paired = true;
                    (d.key, d.asking) = (String::new(), false);
                    d.name = name.clone();
                    mac::notify(&format!("Paired with {name}"), "Your phone and this Mac can now share files, links and the clipboard.").await;
                    for p in started(d, battery, &clip) {
                        send(&id, p);
                    }
                    start_session(&mut sessions, &mac_side, &host, &commands, &id, d);
                }
                Event::Unpaired { id, name } => {
                    if let Some(d) = devices.get_mut(&id) {
                        d.paired = false;
                    }
                    files.forget(&id);
                    sessions.remove(&id);
                    texts.forget(&id);
                    permits.forget(&id);
                    mac::notify(&format!("{name} is no longer paired"), "Pair again from the phone to use it with this Mac.").await;
                }
                Event::Asked { id, key, .. } => {
                    let d = devices.entry(id).or_default();
                    (d.key, d.asking) = (key, false);
                }
                Event::Packet { id, packet, from } => {
                    let Some(d) = devices.get_mut(&id) else { continue };
                    let kind = packet["type"].as_str().unwrap_or("").to_owned();
                    match dispatch::route(&kind) {
                        Route::Session => {
                            // The engine checks the switches and keeps Activity for these itself.
                            if kind == "vyoma.wallpaper.request" {
                                // AppKit tells only this thread where the wallpaper is.
                                mac_side.set_wallpaper(vyoma::wallpaper_path());
                            }
                            if let Some(session) = sessions.get(&id).filter(|_| d.paired) {
                                let _ = session.send((packet, from));
                            }
                        }
                        Route::Media | Route::Volume => {
                            let _ = media_tx.send((id, packet));
                        }
                        Route::Mousepad => match touchpad.on_packet(&permits, &activity, &id, &d.name, permit_words(Permit::Control).0, &packet) {
                            Touch::Refused(no) => {
                                if let Some(no) = no {
                                    send(&id, no);
                                }
                            }
                            Touch::Use { acts, echo } => {
                                if let Some(echo) = echo {
                                    send(&id, echo);
                                }
                                // Said on the phone (every so often: moves stream) and once a run on the Mac,
                                // then watched, so the agent picks the Allow up by itself.
                                if pointer.handle(&input::mapped(acts)).is_err()
                                    && let Some(stuck) = touchpad.stuck(&packet, input::ALLOW_SHORT)
                                {
                                    send(&id, stuck.refusal);
                                    if stuck.first {
                                        mac::notify("Allow Vyoma to control this Mac", input::ALLOW_HOW).await;
                                        tokio::spawn(restart_once_trusted());
                                    }
                                }
                            }
                        },
                        Route::Commands => {
                            let a = commands::on_request(&run_commands, &packet["body"], |name, why| {
                                let (title, body) = commands::failure(&name, why);
                                mac::notify_now(&title, &body);
                            });
                            if let Some(list) = a.list.clone() {
                                send(&id, list);
                            }
                            // The phone's "add command": the editor is Vyoma's window.
                            if a.setup {
                                mac::open_app();
                            }
                            if let Some(line) = a.activity() {
                                tracing::info!("{id}: ran a command");
                                activity.add(&id, &d.name, "command", line);
                            }
                            if let Some(key) = a.unknown {
                                tracing::info!("{id}: no command {key}");
                            }
                        }
                        Route::Notification => phone_notification(&id, d, &packet["body"]).await,
                        Route::TextsLock => {
                            tracing::info!("{id}: {kind}");
                            texts.on_packet(&id, &packet);
                        }
                        Route::Texts => {
                            for p in texts.on_packet(&id, &packet) {
                                send(&id, p);
                            }
                        }
                        Route::Call => {
                            if d.paired {
                                let _ = calls.send(calls::Order::Packet { id, phone: d.name.clone(), packet });
                            }
                        }
                        // The Mac asks the phone nothing about calls; logged all the same.
                        Route::CallDone => tracing::info!("{id}: {}", dispatch::call_done(&packet["body"])),
                        Route::Presenter => {
                            if d.paired {
                                pointer_dot.on_packet(&packet["body"]);
                            }
                        }
                        Route::Stock => {
                            for reply in on_packet(&id, d, &packet, from, &me, &mut clip, &mut ringer, &found_tx, &activity, &arrivals).await {
                                send(&id, reply);
                            }
                        }
                        Route::Other if vyoma_engine::phonefiles::PhoneFiles::answers(&kind) => files.reply(&id, &packet),
                        Route::Other => tracing::debug!("{}: nothing here for {kind}", d.name),
                    }
                }
            }
            if event_was_link {
                media_wanted.store(devices.values().any(|d| d.takes(media::MPRIS) || d.takes(media::VOLUME)), Ordering::Relaxed);
            }},
            Some((to, p)) = media_out.recv() => {
                let kind = p["type"].as_str().unwrap_or("").to_owned();
                for (id, d) in &devices {
                    if d.takes(&kind) && to.as_ref().is_none_or(|t| t == id) {
                        send(id, p.clone());
                    }
                }
            }
            Some(req) = requests.recv() => {
                // sips takes a moment, and only this thread can ask AppKit where the picture is.
                if req.method == "Wallpaper" {
                    let path = vyoma::wallpaper_path();
                    tokio::spawn(async move {
                        let _ = req.reply.send(vyoma::window_wallpaper(path).await);
                    });
                    continue;
                }
                // Listing or copying the phone's files takes a while: not in this loop.
                if let Some(job) = vyoma_engine::phonefiles::request(&files, &req.method, &req.args, &devices, crate::mac::downloads()) {
                    tokio::spawn(async move {
                        let _ = req.reply.send(job.await);
                    });
                    continue;
                }
                let answer = window.answer(&req.method, &req.args, &mut devices).await;
                // Only what the window would show differently: a reread after a read would come
                // back here as another read, round and round.
                if answer.is_ok() && CHANGING.contains(&req.method.as_str()) {
                    let _ = changes.send(json!({ "event": "Changed" }));
                }
                let _ = req.reply.send(answer);
            }
            Some((id, yes)) = answers.recv() => {
                prompts.remove(&id);
                let _ = commands.send(if yes { Command::Accept(id) } else { Command::Reject(id) });
            }
            Some(()) = found.recv() => ringer.stop(),
            Some(text) = copies.recv() => {
                if let Some(p) = clip.changed(text) {
                    for (id, d) in &devices {
                        if d.takes(CLIPBOARD) {
                            send(id, p.clone());
                        }
                    }
                }
            }
            _ = battery_tick.tick() => {
                let now = mac::battery().await;
                if now != battery && let Some(b) = now {
                    for (id, d) in &devices {
                        if d.takes(BATTERY) {
                            send(id, battery_packet(b));
                        }
                    }
                }
                battery = now;
            }
            _ = lock_tick.tick() => {
                // So a new wallpaper reaches the phones (the engine watches what this says).
                mac_side.set_wallpaper(vyoma::wallpaper_path());
                let now = mac::screen_locked();
                let mut relock = texts.expire();
                if now == Some(true) && screen_locked != Some(true) {
                    relock.extend(texts.lock_all());
                }
                screen_locked = now;
                for id in relock {
                    tracing::info!("{id}: texts locked again");
                    send(&id, packet(messages::LOCK, json!({})));
                }
            }
            _ = tokio::signal::ctrl_c() => break,
            _ = terminate.recv() => break,
        }
    }
    // Quitting, or the Mac shutting down (launchd stops agents with SIGTERM): phones stop looking
    // for this Mac until it announces itself again.
    for (id, d) in &devices {
        if d.takes(vyoma::LEAVING) {
            send(id, packet(vyoma::LEAVING, json!({ "why": "stop" })));
        }
    }
    tokio::time::sleep(Duration::from_millis(400)).await;
    Ok(())
}

/// What the window asks that only this Mac answers, and how the Mac shows things (the rest is
/// the engine's [vyoma_engine::window]).
struct MacSide {
    /// For a choice made in a dialog to come back as a request.
    requests: mpsc::Sender<control::Request>,
}

type Window = vyoma_engine::window::Window<MacSide>;

impl Side for MacSide {
    const OS: &'static str = "the Mac";
    const PERMITS: &'static [Permit] = &PERMITS;

    async fn name() -> String {
        mac::name().await
    }

    async fn notify(title: &str, body: &str) {
        mac::notify(title, body).await;
    }

    fn open(url: &str) {
        mac::open(url);
    }

    fn permit_words(p: Permit) -> (&'static str, &'static str) {
        permit_words(p)
    }

    fn pick(w: &Window, phones: Vec<(String, String)>, paths: Vec<String>) -> Answer {
        let names: Vec<String> = phones.iter().map(|p| p.1.clone()).collect();
        let what = if paths.len() == 1 {
            format!("Send \u{201c}{}\u{201d} to which phone?", std::path::Path::new(&paths[0]).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
        } else {
            format!("Send {} items to which phone?", paths.len())
        };
        let requests = w.side.requests.clone();
        tokio::spawn(async move {
            let picked = match mac::choose("Vyoma", &what, "Send", &names) {
                Ok(child) => child.wait_with_output().await.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned()).unwrap_or_default(),
                Err(e) => {
                    tracing::warn!("couldn't ask which phone: {e:#}");
                    return;
                }
            };
            let Some((id, _)) = phones.into_iter().find(|p| !picked.is_empty() && p.1 == picked) else { return };
            let (reply, _) = tokio::sync::oneshot::channel();
            let _ = requests.send(control::Request { method: "SendFiles".into(), args: json!([id, paths, true]), reply }).await;
        });
        Ok(Value::Null)
    }

    async fn answer(w: &Window, method: &str, args: &Value, _: &mut Devices) -> Option<Answer> {
        Some(match method {
            "Health" => Ok(json!({
                "features": [], "installable": false, "checks": checks(), "ways": ways(), "notes": [],
                "update": w.updates.status(), "version": VERSION.trim(),
            })),
            "FixCheck" => {
                let check = match vyoma_engine::window::text(args, 0) {
                    Ok(c) => c,
                    Err(e) => return Some(Err(e)),
                };
                match check.as_str() {
                    TOUCHPAD_CHECK => {
                        input::ask();
                        mac::open(input::ACCESSIBILITY);
                        tokio::spawn(restart_once_trusted());
                        Ok(Value::Null)
                    }
                    SCREEN_CHECK => {
                        screen::allowed(true);
                        mac::open(screen::SCREEN_RECORDING);
                        Ok(Value::Null)
                    }
                    CAMERA_CHECK => {
                        if camera::access() == camera::Access::NotAsked {
                            let changes = w.changes.clone();
                            tokio::spawn(async move {
                                let answer = camera::ask().await;
                                tracing::info!("camera: {answer:?}");
                                if answer == camera::Access::NotAsked {
                                    // Not from Vyoma.app, or no answer: Settings is where it's turned on.
                                    mac::open(camera::CAMERA_SETTINGS);
                                }
                                let _ = changes.send(json!({ "event": "Changed" }));
                            });
                        } else {
                            mac::open(camera::CAMERA_SETTINGS);
                        }
                        Ok(Value::Null)
                    }
                    CABLE_CHECK => {
                        let changes = w.changes.clone();
                        tokio::spawn(async move {
                            if let Err(e) = usb::fetch_adb().await {
                                tracing::warn!("no adb: {e:#}");
                                mac::notify("Couldn't get adb", &format!("{e:#}")).await;
                            }
                            let _ = changes.send(json!({ "event": "Changed" }));
                        });
                        Ok(Value::Null)
                    }
                    _ => Err(w.not_here()),
                }
            }
            _ => return None,
        })
    }
}

/// Whether [event] changes what the window shows of a device: not the packets passing through,
/// except the battery it shows.
fn changes_details(event: &Event) -> bool {
    match event {
        Event::Packet { packet, .. } => packet["type"] == BATTERY,
        _ => true,
    }
}

/// Once macOS allows Vyoma in Accessibility, starts the agent again: macOS often lets a process
/// that started without the Allow keep posting into nothing. launchd starts it again at once.
async fn restart_once_trusted() {
    if std::env::var_os("XPC_SERVICE_NAME").is_none() {
        return; // Not launchd's: nobody would start it again.
    }
    while !input::trusted() {
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    tracing::info!("allowed in Accessibility: starting again so the touchpad works");
    tokio::time::sleep(Duration::from_millis(500)).await;
    std::process::exit(0);
}

/// This Mac's "How this Mac does it": macOS's own ways, which don't vary but for the live screen's
/// framework (macOS 12.3); what macOS allows is the checks' part.
fn ways() -> Value {
    use vyoma_engine::ways::Way;
    const SCREEN: &str = "Live screen";
    vyoma_engine::ways::json(&[
        Way::found("Touchpad and keyboard", "macOS's CGEvent (allowed in Accessibility)"),
        Way::found("Clipboard", "macOS's pasteboard"),
        if screen::capture_kit() {
            Way::found(SCREEN, "ScreenCaptureKit, encoded by VideoToolbox")
        } else {
            Way::missing(SCREEN, "Needs macOS 12.3 or newer (ScreenCaptureKit)")
        },
        Way::found("Screenshots", "macOS's screencapture"),
        Way::found("Pointer dot", "A click-through window above everything"),
        Way::found("Desktops", "Mission Control and the Spaces shortcuts"),
    ])
}

/// This Mac's checks for the window's This Mac page: the Accessibility Allow the touchpad needs,
/// Screen Recording for the screen on the phone, the camera's Allow for photos, and adb for the USB cable.
fn checks() -> Value {
    let touch = input::trusted();
    let screen = screen::allowed(false);
    let camera = camera::access();
    let adb = vyoma_core::adb::find_adb().is_some();
    json!([
        {
            "name": TOUCHPAD_CHECK,
            "state": if touch { "ok" } else { "warn" },
            "detail": if touch { "Allowed in Accessibility" } else { "macOS hasn't allowed Vyoma in Accessibility yet. After an update, remove the old Vyoma there and turn the new one on" },
            "action": if touch { "" } else { "Open Settings" },
        },
        {
            "name": SCREEN_CHECK,
            "state": if screen { "ok" } else { "warn" },
            "detail": if screen { "Allowed in Screen Recording: the phone can show this screen and take screenshots" } else { "For the screen on the phone and screenshots: allow Vyoma in Screen Recording" },
            "action": if screen { "" } else { "Open Settings" },
        },
        {
            "name": CAMERA_CHECK,
            "state": if camera == camera::Access::Allowed { "ok" } else { "warn" },
            "detail": match camera {
                camera::Access::Allowed => "Allowed: the phone can take photos with this Mac's camera",
                camera::Access::NotAsked => "For photos from the phone: macOS asks once whether Vyoma may use the camera",
                camera::Access::Refused => "Turned off for Vyoma in Camera settings, so the phone can't take photos",
            },
            "action": match camera {
                camera::Access::Allowed => "",
                camera::Access::NotAsked => "Allow",
                camera::Access::Refused => "Open Settings",
            },
        },
        {
            "name": CABLE_CHECK,
            "state": if adb { "ok" } else { "warn" },
            "detail": if adb {
                if usb::plugged() > 0 { "A phone is plugged in" } else { "Plug the phone in with USB debugging on, and they link over the cable" }
            } else {
                "Linking over the cable needs adb, Google's Android platform tools. Get adb downloads them from Google into Vyoma's folder"
            },
            "action": if adb { "" } else { "Get adb" },
        },
    ])
}

const TOUCHPAD_CHECK: &str = "Touchpad and keyboard";
const SCREEN_CHECK: &str = "Screen";
const CAMERA_CHECK: &str = "Camera";
const CABLE_CHECK: &str = "USB cable";

/// `--send-any [paths]` (Finder's Quick Action): hands the files to the running agent, which picks
/// the phone and says how it went. With the agent not running, opens Vyoma (which starts it) first.
fn send_any(paths: Vec<String>) -> Result<()> {
    use std::io::{BufRead, Write};
    if paths.is_empty() {
        return Ok(());
    }
    let here = std::env::current_dir().unwrap_or_default();
    let paths: Vec<String> = paths.iter().map(|p| here.join(p).to_string_lossy().into_owned()).collect();
    let dir = std::env::var_os("VYOMA_DIR").map(PathBuf::from).unwrap_or_else(launchd::support_dir);
    let socket = std::env::var_os("VYOMA_SOCKET").map(PathBuf::from).unwrap_or_else(|| control::socket_path(&dir));
    for attempt in 0..SEND_ANY_WAIT_STEPS {
        if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(&socket) {
            stream.set_read_timeout(Some(Duration::from_secs(10)))?;
            writeln!(stream, "{}", json!({ "method": "SendAny", "args": [paths] }))?;
            let mut line = String::new();
            std::io::BufReader::new(stream).read_line(&mut line)?;
            // The agent tells of its own failures; this only covers an answer that never came.
            if line.is_empty() {
                mac::notify_now("Couldn't send", "Vyoma didn't answer. Open Vyoma, then try again.");
            }
            return Ok(());
        }
        if attempt == 0 {
            mac::open_app();
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    mac::notify_now("Couldn't send", "Vyoma isn't running on this Mac right now. Open Vyoma, then try again.");
    Ok(())
}

/// Starts phone [id]'s engine session, unless one is running; dropping its entry ends it.
fn start_session(
    sessions: &mut HashMap<String, mpsc::UnboundedSender<(Value, std::net::IpAddr)>>,
    mac_side: &Arc<platform::Mac>,
    host: &Arc<Host>,
    commands: &mpsc::UnboundedSender<Command>,
    id: &str,
    d: &Device,
) {
    if sessions.get(id).is_some_and(|s| !s.is_closed()) {
        return;
    }
    let (packets, rx) = mpsc::unbounded_channel();
    sessions.insert(id.to_owned(), packets);
    let phone = Phone { id: id.to_owned(), name: phone_name(&d.name), pinned: d.cert.clone() };
    tokio::spawn(vyoma_engine::session::serve(mac_side.clone(), host.clone(), phone, vyoma::sender(commands, id), rx));
}

fn identity(me: &Identity, name: String, laptop: bool) -> Value {
    let (engine_in, engine_out) = vyoma_engine::platform::capabilities_for(platform::Mac::offers);
    packet(
        "kdeconnect.identity",
        json!({
            "deviceId": me.device_id,
            "deviceName": name,
            "deviceType": if laptop { "laptop" } else { "desktop" },
            "protocolVersion": 8,
            "incomingCapabilities": ([INCOMING, vyoma_engine::phonefiles::INCOMING, &engine_in].concat()),
            "outgoingCapabilities": ([OUTGOING, vyoma_engine::phonefiles::OUTGOING, &engine_out].concat()),
        }),
    )
}

/// What a paired device that just linked is sent: this Mac's battery and its clipboard.
fn started(d: &Device, battery: Option<mac::Battery>, clip: &Clipboard) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(b) = battery.filter(|_| d.takes(BATTERY)) {
        out.push(battery_packet(b));
    }
    if let Some(p) = clip.connect_packet().filter(|_| d.takes(CLIPBOARD_CONNECT)) {
        out.push(p);
    }
    // The phone's keyboard button: this Mac takes typing.
    if d.takes(KEYBOARDSTATE) {
        out.push(packet(KEYBOARDSTATE, json!({ "state": true })));
    }
    out
}

/// A notification from the phone, shown as the Mac's own, or taken down when the phone's went.
/// The backlog the phone sends when asked (`silent`) isn't shown: it would arrive all at once.
async fn phone_notification(id: &str, d: &Device, b: &Value) {
    // Its own per phone, so two phones' ids never meet.
    let key = |note: &str| format!("phone-{id}-{note}");
    match notifications::parse(&json!({ "body": b })) {
        Incoming::Cancel(note) => mac::withdraw(&key(&note)),
        Incoming::Show(note) => mac::phone_notification(&key(&note.id), &note.from_line(&d.name), note.title_or_app(), &note.text).await,
        Incoming::Ignore => {}
    }
}

/// The stock plugins on the Mac: its notifications, the ringer with a Found it dialog, display
/// sleep as the lock.
struct MacDesk<'a> {
    ringer: &'a mut mac::Ringer,
    found: &'a mpsc::UnboundedSender<()>,
}

impl Desk for MacDesk<'_> {
    async fn tell(&mut self, title: &str, body: &str) {
        mac::notify(title, body).await;
    }

    async fn ring(&mut self, phone: &str) {
        self.ringer.start();
        let (found, text) = (self.found.clone(), format!("{phone} is looking for this Mac."));
        tokio::spawn(async move {
            if let Ok(child) = mac::ask("Vyoma", &text, "Stop", "Found it", Duration::from_secs(60)) {
                let _ = child.wait_with_output().await;
            }
            let _ = found.send(());
        });
    }

    async fn set_locked(&mut self, lock: bool) -> Option<Result<()>> {
        // The display asleep asks for the password when it wakes; nothing unlocks it from outside.
        if !lock {
            return None;
        }
        Some(mac::sleep_display().await)
    }

    async fn copy(&mut self, text: &str) -> Result<()> {
        mac::set_clipboard(text).await
    }

    async fn open_link(&mut self, url: &str) {
        mac::open(url);
    }
}

/// Handles one stock plugin's packet from a paired device; what to send back.
#[allow(clippy::too_many_arguments)]
async fn on_packet(id: &str, d: &mut Device, p: &Value, from: IpAddr, me: &Arc<Identity>, clip: &mut Clipboard, ringer: &mut mac::Ringer, found: &mpsc::UnboundedSender<()>, activity: &Arc<Activity>, arrivals: &Arc<stock::Arrivals>) -> Vec<Value> {
    let mut phone = stock::Phone { name: d.name.clone(), battery: d.battery };
    let out = stock::on_packet(&mut MacDesk { ringer, found }, &mut phone, clip, p).await;
    d.battery = phone.battery;
    if let Some(incoming) = out.fetch {
        let (me, cert, phone, packet, activity, arrivals, id) = (me.clone(), d.cert.clone(), d.name.clone(), p.clone(), activity.clone(), arrivals.clone(), id.to_owned());
        let from = d.remote.unwrap_or(from);
        tokio::spawn(async move {
            let landed = stock::fetch(&me, &cert, from, &packet, &mac::downloads(), incoming, &arrivals, &id, &phone, "this Mac").await;
            if let Some(text) = landed.activity.clone() {
                activity.add(&id, &phone, "share", text);
            }
            if let Some((title, body)) = landed.summary() {
                mac::notify(&title, &body).await;
            }
        });
    }
    out.send
}

#[cfg(test)]
mod tests {
    use super::*;
    use vyoma_engine::window::{details, url_encode};

    fn phone(incoming: &[&str]) -> Device {
        Device { paired: true, linked: true, incoming: incoming.iter().map(|s| s.to_string()).collect(), ..Default::default() }
    }

    #[test]
    fn a_linked_phone_gets_the_battery_and_clipboard_it_takes() {
        let b = Some(mac::Battery { charge: 70, charging: true });
        let mut clip = Clipboard::default();
        assert!(started(&phone(&[BATTERY, CLIPBOARD_CONNECT]), None, &clip).is_empty(), "a desktop Mac with nothing copied");
        clip.changed("hello".into());
        let kinds: Vec<_> = started(&phone(&[BATTERY, CLIPBOARD_CONNECT]), b, &clip).iter().map(|p| p["type"].as_str().unwrap().to_owned()).collect();
        assert_eq!(kinds, [BATTERY, CLIPBOARD_CONNECT]);
        assert!(started(&phone(&[PING]), b, &clip).is_empty(), "only what it takes");
        assert!(started(&Device { paired: false, ..phone(&[BATTERY]) }, b, &clip).is_empty(), "nothing before pairing");
    }

    #[test]
    fn the_window_reads_a_phone_as_on_linux() {
        let mut d = phone(&[SHARE]);
        (d.name, d.key, d.asking, d.battery) = ("Pixel".into(), "1A2B".into(), true, Some((64, false, false)));
        let permits = Permits::default();
        permits.set("p", Permit::Files, false).unwrap();
        let seen = vyoma_core::adb::ClipboardSeen::load(&std::env::temp_dir().join(format!("vyoma-mac-clip-{}", std::process::id())));
        let v = details::<MacSide>("p", &d, &permits, &seen, 1);
        assert_eq!((v["id"].as_str(), v["key"].as_str(), v["asking"].as_bool()), (Some("p"), Some("1A2B"), Some(true)));
        assert_eq!((v["can"]["share"].as_bool(), v["can"]["ring"].as_bool()), (Some(true), Some(false)), "only what it takes");
        assert_eq!(v["battery"]["charge"], 64);
        assert!(details::<MacSide>("q", &Device::default(), &permits, &seen, 1)["battery"].is_null());
        // Only the switches this Mac has, in Mac words.
        let keys: Vec<(&str, bool)> = v["permits"].as_array().unwrap().iter().map(|p| (p["key"].as_str().unwrap(), p["on"].as_bool().unwrap())).collect();
        assert_eq!(keys, [("screen", true), ("control", true), ("files", false), ("camera", true), ("power", true)]);
        assert_eq!(v["permits"][3]["name"], "Take photos with this Mac's camera");
        assert_eq!(v["permits"][4]["name"], "Wake, sleep and shut down");
        assert_eq!(url_encode("Sam's Mac"), "Sam%27s%20Mac");
    }

    #[test]
    fn the_cable_wins_while_its_in() {
        let (wifi, cable): (IpAddr, IpAddr) = ("192.168.1.5".parse().unwrap(), "127.0.0.1".parse().unwrap());
        assert_eq!(vyoma_core::adb::best_route(&[cable, wifi], true), Some(cable));
        assert_eq!(vyoma_core::adb::best_route(&[cable, wifi], false), Some(wifi));
        assert_eq!(vyoma_core::adb::best_route(&[cable], false), Some(cable), "whatever there is");
        let mut d = phone(&[]);
        d.remote = Some(cable);
        assert_eq!(d.via(), "cable");
        d.remote = Some(wifi);
        assert_eq!(d.via(), "wifi");
        d.linked = false;
        assert_eq!(d.via(), "");
    }

    #[test]
    fn it_says_what_it_does() {
        let dir = std::env::temp_dir().join(format!("vyoma-mac-id-{}", std::process::id()));
        let me = Identity::load_or_create_in(&dir).unwrap();
        let p = identity(&me, "Sam's MacBook".into(), true);
        assert_eq!(p["body"]["deviceType"], "laptop");
        let incoming = p["body"]["incomingCapabilities"].as_array().unwrap();
        assert_eq!(incoming.len(), INCOMING.len() + vyoma_engine::phonefiles::INCOMING.len() + vyoma_engine::platform::capabilities_for(platform::Mac::offers).0.len());
        // The phone loads its file server's plugin only for a PC that lists its packets.
        let outgoing = p["body"]["outgoingCapabilities"].as_array().unwrap();
        assert!(incoming.iter().any(|t| t == "kdeconnect.sftp") && outgoing.iter().any(|t| t == "kdeconnect.sftp.request"));
        assert!(outgoing.iter().any(|t| t == "vyoma.files.thumbs.request"));
        // What the phone looks for before it shows a tile (its PcFeatures).
        for offered in ["vyoma.away", "kdeconnect.telephony", "kdeconnect.presenter", "vyoma.screenshot.request", "vyoma.camera.request", "vyoma.screen.request", "vyoma.mirror", "vyoma.paste", "vyoma.desktop.step"] {
            assert!(incoming.iter().any(|t| t == offered), "{offered}");
        }
        assert!(!OUTGOING.contains(&"kdeconnect.lock"), "no lock state it can't know");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
