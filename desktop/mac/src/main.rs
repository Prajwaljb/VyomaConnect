//! Vyoma's PC side on macOS (docs/PLAN.md section 10, phase 5), on the same core as the Linux app:
//! it finds and links with phones, pairs after asking (a dialog with the code to compare), and
//! serves the stock plugins: ping, battery both ways, find my phone, the screen lock, the clipboard
//! both ways, sharing (files into ~/Downloads, links opened, text onto the clipboard), Music and
//! Spotify and the volume ([media]), the touchpad and keyboard ([input]), the phone's
//! notifications, and the commands set up in the window.
//!
//! Vyoma's window talks to it over a local socket ([control]). None of Vyoma's extras yet: to the
//! phone this is a stock KDE Connect PC.
//! Built and tested on GitHub's macOS runners only; not yet tried on a real Mac.

mod control;
mod finder;
mod input;
mod launchd;
mod mac;
mod media;
#[cfg(target_os = "macos")]
mod notify;
mod update;
mod vyoma;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc};
use vyoma_core::activity::Activity;
use vyoma_core::clipboard::{CLIPBOARD, CLIPBOARD_CONNECT, Clipboard, MAX_TEXT};
use vyoma_core::commands::{Commands, RUNCOMMAND, RUNCOMMAND_REQUEST};
use vyoma_core::device::{Command, Event, Hub};
use vyoma_core::identity::Identity;
use vyoma_core::invite::Invite;
use vyoma_core::lan::{Lan, Ports, Rules, UDP_PORT};
use vyoma_core::messages::{self, Store};
use vyoma_core::link::packet;
use vyoma_core::permits::{self, Permit, Permits};
use vyoma_core::share::{self, SHARE, SHARE_UPDATE, Shared};
use vyoma_core::trust::TrustStore;

pub const PING: &str = "kdeconnect.ping";
pub const BATTERY: &str = "kdeconnect.battery";
pub const FIND: &str = "kdeconnect.findmyphone.request";
pub const LOCK_REQUEST: &str = "kdeconnect.lock.request";
pub const NOTIFICATION: &str = "kdeconnect.notification";
pub const KEYBOARDSTATE: &str = "kdeconnect.mousepad.keyboardstate";

/// What we take and send. No lock state outward: macOS doesn't tell other apps whether it's locked.
const INCOMING: &[&str] = &[
    PING, BATTERY, FIND, LOCK_REQUEST, CLIPBOARD, CLIPBOARD_CONNECT, SHARE, SHARE_UPDATE,
    media::MPRIS_REQUEST, media::VOLUME_REQUEST, RUNCOMMAND_REQUEST, NOTIFICATION, input::MOUSEPAD_REQUEST,
    messages::MESSAGES, messages::CONTACTS_UIDS_REPLY, messages::CONTACTS_VCARDS_REPLY, messages::UNLOCKED, messages::LOCKED,
];
const OUTGOING: &[&str] = &[
    PING, BATTERY, FIND, CLIPBOARD, CLIPBOARD_CONNECT, SHARE, SHARE_UPDATE,
    media::MPRIS, media::VOLUME, RUNCOMMAND, input::MOUSEPAD_ECHO, KEYBOARDSTATE,
    messages::REQUEST, messages::REQUEST_CONVERSATIONS, messages::REQUEST_CONVERSATION, messages::CONTACTS_UIDS, messages::CONTACTS_VCARDS,
    messages::UNLOCK, messages::LOCK,
];

/// This release, as the Linux app reads it: the repo's one version.
const VERSION: &str = include_str!("../../../VERSION");
use vyoma_core::release::REPO;

/// How often the clipboard is looked at (macOS sends no change events to a program like this one)
/// and the battery read.
const CLIPBOARD_EVERY: Duration = Duration::from_secs(1);
const BATTERY_EVERY: Duration = Duration::from_secs(60);
/// How often the screen's lock and unused texts are looked at (texts lock again with the screen).
const LOCK_EVERY: Duration = Duration::from_secs(3);
/// The touchpad counts as one use in Activity until it's left alone this long.
const TOUCHPAD_STRETCH: Duration = Duration::from_secs(10 * 60);

/// The switches this Mac has, in the window's order: what it serves so far.
const PERMITS: [Permit; 3] = [Permit::Control, Permit::Files, Permit::Power];

/// A switch as the Mac's window shows it: the Linux words, less what a Mac can't do.
fn permit_words(p: Permit) -> (&'static str, &'static str) {
    match p {
        Permit::Control => ("Touchpad and keyboard", "Move the pointer and type"),
        Permit::Power => ("Wake, sleep and shut down", "Also log out and restart"),
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
        Some(other) => bail!("unknown option {other}: try --install or --uninstall"),
        None => run(std::env::var_os("VYOMA_DIR").map(PathBuf::from).unwrap_or_else(launchd::support_dir)),
    }
}

/// A device as this Mac knows it.
#[derive(Default)]
struct Device {
    name: String,
    paired: bool,
    linked: bool,
    incoming: Vec<String>,
    cert: Vec<u8>,
    /// The address of its newest link, where its payloads are fetched.
    remote: Option<IpAddr>,
    /// Its battery: charge, charging, and whether it said it's low.
    battery: Option<(i64, bool, bool)>,
    /// The code of a pairing in progress, and whether it's the phone asking (so Accept and Reject).
    key: String,
    asking: bool,
}

impl Device {
    fn takes(&self, kind: &str) -> bool {
        self.paired && self.linked && self.incoming.iter().any(|t| t == kind)
    }
}

#[tokio::main]
async fn run(dir: PathBuf) -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "vyoma_mac=info,vyoma_core=info".into()))
        .init();
    vyoma_core::identity::use_dir(dir.clone());
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
    let mut refused: HashMap<(String, Permit), Instant> = HashMap::new();
    let mut touched: HashMap<String, Instant> = HashMap::new();
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
    let updates = update::Updates::load(&dir, VERSION);
    {
        let changes = changes.clone();
        tokio::spawn(updates.clone().run(move || {
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
    let window = Window { me: me.clone(), lan: lan.clone(), commands: commands.clone(), changes: changes.clone(), everyone, run: run_commands.clone(), texts: texts.clone(), permits: permits.clone(), activity: activity.clone(), updates, requests: requests_tx };

    // Players and the volume, read while a phone that takes them is linked.
    let media_wanted = Arc::new(AtomicBool::new(false));
    let (media_tx, media_rx) = mpsc::unbounded_channel::<(String, Value)>();
    let (media_out_tx, mut media_out) = mpsc::unbounded_channel::<(Option<String>, Value)>();
    tokio::spawn(media::run(media_wanted.clone(), media_rx, media_out_tx));
    let mut pointer = input::Pointer::default();
    let mut features = vyoma::Vyoma::new(me.clone(), commands.clone(), activity.clone());

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
    let mut battery_tick = tokio::time::interval(BATTERY_EVERY);
    battery_tick.tick().await;
    let mut lock_tick = tokio::time::interval(LOCK_EVERY);
    let mut screen_locked = mac::screen_locked();

    // Answers to the pairing questions on screen, and the Found it of a ring.
    let (answers_tx, mut answers) = mpsc::unbounded_channel::<(String, bool)>();
    let mut prompts: HashMap<String, tokio::task::JoinHandle<()>> = HashMap::new();
    let (found_tx, mut found) = mpsc::unbounded_channel::<()>();
    let mut ringer = mac::Ringer::default();

    loop {
        tokio::select! {
            Some(event) = events.recv() => { if changes_details(&event) { let _ = changes.send(json!({ "event": "Changed" })); } let event_was_link = matches!(event, Event::Linked { .. } | Event::Gone { .. } | Event::Paired { .. } | Event::Unpaired { .. }); match event {
                Event::Linked { id, name, paired, incoming, cert, remote, .. } => {
                    let d = devices.entry(id.clone()).or_default();
                    *d = Device { name, paired, linked: true, incoming, cert, remote: Some(remote), battery: d.battery, ..Default::default() };
                    tracing::info!("{id}: linked{}", if paired { ", paired" } else { "" });
                    if paired {
                        for p in started(d, battery, &clip) {
                            send(&id, p);
                        }
                    }
                }
                Event::Gone { id } => {
                    if let Some(d) = devices.get_mut(&id) {
                        d.linked = false;
                    }
                    features.gone(&id);
                    texts.gone(&id);
                }
                Event::Routes { id, remotes } => {
                    if let Some(d) = devices.get_mut(&id) {
                        d.remote = remotes.last().copied();
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
                }
                Event::Unpaired { id, name } => {
                    if let Some(d) = devices.get_mut(&id) {
                        d.paired = false;
                    }
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
                    // Switched off in the window: the phone hears why (once in a while: streams repeat).
                    if let Some(permit) = Permit::of(&kind).filter(|&p| !permits.allows(&id, p)) {
                        let now = Instant::now();
                        let last = refused.insert((id.clone(), permit), now);
                        if last.is_none_or(|t| now.duration_since(t) > Duration::from_secs(2)) {
                            tracing::info!("{id}: {kind} refused, {} is off for this phone", permit.key());
                            activity.add(&id, &d.name, "refused", format!("Refused: \u{201c}{}\u{201d} is off", permit_words(permit).0));
                            send(&id, permits::denied(permit, &packet));
                        }
                        continue;
                    }
                    match kind.as_str() {
                        media::MPRIS_REQUEST | media::VOLUME_REQUEST => {
                            let _ = media_tx.send((id, packet));
                        }
                        input::MOUSEPAD_REQUEST => {
                            let now = Instant::now();
                            if touched.insert(id.clone(), now).is_none_or(|t| now.duration_since(t) > TOUCHPAD_STRETCH) {
                                activity.add(&id, &d.name, "touchpad", "Used as touchpad");
                            }
                            if let Some(echo) = input::echo(&packet["body"]) {
                                send(&id, echo);
                            }
                            if pointer.handle(&input::acts(&packet["body"])).is_err() {
                                mac::notify(
                                    "Allow Vyoma to control this Mac",
                                    "For the touchpad and keyboard: System Settings > Privacy & Security > Accessibility, then turn on Vyoma.",
                                )
                                .await;
                            }
                        }
                        RUNCOMMAND_REQUEST => {
                            let body = &packet["body"];
                            if body["requestCommandList"].as_bool() == Some(true) {
                                send(&id, run_commands.list_packet());
                            }
                            // The phone's "add command": the editor is Vyoma's window.
                            if body["setup"].as_bool() == Some(true) {
                                mac::open_app();
                            }
                            if let Some(key) = body["key"].as_str() {
                                let ran = run_commands.run(key, |name, why| mac::notify_now(&format!("\u{201c}{name}\u{201d} didn't run"), &format!("{why}. Check it in Vyoma's Commands.")));
                                match ran {
                                    Some(name) => {
                                        tracing::info!("{id}: ran a command");
                                        activity.add(&id, &d.name, "command", format!("Ran \u{201c}{name}\u{201d}"));
                                    }
                                    None => tracing::info!("{id}: no command {key}"),
                                }
                            }
                        }
                        NOTIFICATION => phone_notification(&id, d, &packet["body"]).await,
                        messages::UNLOCKED | messages::LOCKED => {
                            tracing::info!("{id}: {}", packet["type"].as_str().unwrap_or(""));
                            texts.on_packet(&id, &packet);
                        }
                        messages::MESSAGES | messages::CONTACTS_UIDS_REPLY | messages::CONTACTS_VCARDS_REPLY => {
                            for p in texts.on_packet(&id, &packet) {
                                send(&id, p);
                            }
                        }
                        kind if vyoma::takes(kind) && d.paired => {
                            // AppKit tells only this thread where the wallpaper is.
                            let wallpaper = if kind == vyoma::WALLPAPER_REQUEST { vyoma::wallpaper_path() } else { None };
                            features.handle(&id, &vyoma::Phone { name: &d.name, cert: &d.cert, from }, &packet, wallpaper);
                        }
                        _ => {
                            for reply in on_packet(&id, d, &packet, from, &me, &mut clip, &mut ringer, &found_tx, &activity).await {
                                send(&id, reply);
                            }
                        }
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
        }
    }
    Ok(())
}

/// What the window can ask for (the Linux app's D-Bus methods, the ones a Mac has so far).
struct Window {
    me: Arc<Identity>,
    lan: Arc<Lan>,
    commands: mpsc::UnboundedSender<Command>,
    changes: broadcast::Sender<Value>,
    everyone: [SocketAddr; 1],
    run: Commands,
    texts: Arc<Store>,
    permits: Arc<Permits>,
    activity: Arc<Activity>,
    updates: Arc<update::Updates>,
    /// For a choice made in a dialog to come back as a request.
    requests: mpsc::Sender<control::Request>,
}

impl Window {
    async fn answer(&self, method: &str, args: &Value, devices: &mut HashMap<String, Device>) -> Result<Value, String> {
        let command = |c: Command| self.commands.send(c).map(|_| Value::Null).map_err(|_| "Vyoma is stopping".to_owned());
        match method {
            "Phones" => Ok(Value::Array(devices.iter().map(|(id, d)| details(id, d, &self.permits)).collect())),
            "Pair" => command(Command::Pair(control::text(args, 0)?)),
            "Accept" => command(Command::Accept(control::text(args, 0)?)),
            "Reject" => command(Command::Reject(control::text(args, 0)?)),
            "Unpair" => command(Command::Unpair(control::text(args, 0)?)),
            "Ring" => {
                let id = control::text(args, 0)?;
                if !devices.get(&id).is_some_and(|d| d.takes(FIND)) {
                    return Err("That phone can't be rung right now".into());
                }
                command(Command::Send(id, packet(FIND, json!({}))))
            }
            "SendFiles" => {
                let id = control::text(args, 0)?;
                // From Finder ([finder]): nobody's looking at the window, so notifications say how it went.
                let tell = args[2].as_bool().unwrap_or(false);
                self.send(id, strings(&args[1]), devices, tell).await
            }
            "SendAny" => {
                let paths = strings(&args[0]);
                if paths.is_empty() {
                    return Ok(Value::Null);
                }
                let all: Vec<_> = devices.iter().map(|(id, d)| (id.clone(), d.name.clone(), d.paired, d.linked)).collect();
                match share::pick(&all) {
                    share::Pick::One(id) => self.send(id, paths, devices, true).await,
                    share::Pick::NonePaired => {
                        mac::notify("No phone to send to", "Pair your phone in Vyoma first, then try again.").await;
                        Ok(Value::Null)
                    }
                    share::Pick::Ask(phones) => {
                        let names: Vec<String> = phones.iter().map(|p| p.1.clone()).collect();
                        let what = if paths.len() == 1 {
                            format!("Send \u{201c}{}\u{201d} to which phone?", std::path::Path::new(&paths[0]).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
                        } else {
                            format!("Send {} items to which phone?", paths.len())
                        };
                        let requests = self.requests.clone();
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
                }
            }
            "Refresh" => self.lan.announce(&self.everyone).await.map(|_| Value::Null).map_err(|e| format!("{e:#}")),
            "PairingCode" => {
                // No address, no code: a phone couldn't reach this Mac. The window says so (the error's word).
                let addresses: Vec<String> = vyoma_core::invite::local_addresses().iter().map(|a| a.to_string()).collect();
                if addresses.is_empty() {
                    return Err("offline".into());
                }
                let invite = Invite::new(std::time::Instant::now()).ok_or("no randomness")?;
                let secret = vyoma_core::invite::hex(invite.secret());
                command(Command::Invite(Some(invite)))?;
                Ok(json!(format!(
                    "https://github.com/{REPO}/releases/latest#pair?v=1&id={}&name={}&fp={}&s={secret}&a={}&p={}",
                    self.me.device_id,
                    url_encode(&mac::name().await),
                    vyoma_core::invite::fingerprint(&self.me.cert),
                    addresses.join(","),
                    self.lan.tcp_port()
                )))
            }
            "EndPairingCode" => command(Command::Invite(None)),
            "Health" => Ok(json!({
                "features": [], "installable": false, "checks": [], "notes": [],
                "update": self.updates.status(),
            })),
            "SetUpdateChecks" => {
                self.updates.set_checks(args[0].as_bool().ok_or("on or off?")?);
                Ok(Value::Null)
            }
            "Update" => {
                mac::open(&self.updates.page().ok_or("No newer Vyoma for the Mac yet")?);
                Ok(Value::Null)
            }
            "Commands" => Ok(Value::Object(self.run.all().into_iter().collect())),
            "SaveCommand" => {
                let flag = |i: usize| args[i].as_bool().unwrap_or(false);
                let key = self.run.save_one(&control::text(args, 0)?, &control::text(args, 1)?, &control::text(args, 2)?, flag(3), flag(4)).map_err(|e| format!("{e:#}"))?;
                self.commands_changed(devices);
                Ok(json!(key))
            }
            "RemoveCommand" => {
                self.run.remove(&control::text(args, 0)?).map_err(|e| format!("{e:#}"))?;
                self.commands_changed(devices);
                Ok(Value::Null)
            }
            "Activity" => Ok(Value::Array(self.activity.recent())),
            "ClearActivity" => {
                self.activity.clear();
                Ok(Value::Null)
            }
            "SetPermission" => {
                let (id, key) = (control::text(args, 0)?, control::text(args, 1)?);
                let on = args[2].as_bool().ok_or("on or off?")?;
                let p = Permit::from_key(&key).filter(|p| PERMITS.contains(p)).ok_or_else(|| format!("no permission {key}"))?;
                self.permits.set(&id, p, on).map_err(|e| format!("{e:#}"))?;
                tracing::info!("{id}: {key} {}", if on { "allowed" } else { "turned off" });
                Ok(Value::Null)
            }
            "Conversations" => {
                let id = control::text(args, 0)?;
                let d = devices.get(&id).filter(|d| d.takes(messages::REQUEST)).ok_or("This phone doesn't share its texts")?;
                let (json, ask) = self.texts.conversations(&id, d.takes(messages::UNLOCK));
                for p in ask {
                    command(Command::Send(id.clone(), p))?;
                }
                serde_json::from_str(&json).map_err(|e| e.to_string())
            }
            "Thread" => {
                let id = control::text(args, 0)?;
                let thread = args[1].as_i64().ok_or("which conversation?")?;
                let guarded = devices.get(&id).is_some_and(|d| d.takes(messages::UNLOCK));
                let (json, ask) = self.texts.thread(&id, thread, guarded);
                for p in ask {
                    command(Command::Send(id.clone(), p))?;
                }
                serde_json::from_str(&json).map_err(|e| e.to_string())
            }
            "TextThread" => {
                let id = control::text(args, 0)?;
                let thread = args[1].as_i64().ok_or("which conversation?")?;
                let text = control::text(args, 2)?;
                let d = devices.get(&id).filter(|d| d.takes(messages::REQUEST)).ok_or("The phone isn't connected")?;
                let p = self.texts.send(&id, thread, &text, d.takes(messages::UNLOCK))?;
                tracing::info!("{id}: sending a text");
                command(Command::Send(id, p))
            }
            "UnlockTexts" => {
                let id = control::text(args, 0)?;
                let Some(d) = devices.get(&id).filter(|d| d.takes(messages::UNLOCK)) else { return Ok(Value::Null) };
                if !d.linked {
                    return Err("The phone isn't connected".into());
                }
                if let Some(p) = self.texts.unlock(&id) {
                    tracing::info!("{id}: asking to show the texts");
                    command(Command::Send(id, p))?;
                }
                Ok(Value::Null)
            }
            _ => Err("Not on the Mac yet".into()),
        }
    }
}

impl Window {
    /// Sends [paths] (files and folders) to [id]; with [tell], notifications say how it went.
    async fn send(&self, id: String, paths: Vec<String>, devices: &HashMap<String, Device>, tell: bool) -> Result<Value, String> {
        let Some(d) = devices.get(&id).filter(|d| d.takes(SHARE)) else {
            let name = devices.get(&id).map_or("The phone", |d| d.name.as_str());
            let why = format!("{name} isn't connected. Open Vyoma on it, then try again.");
            if tell {
                mac::notify("Couldn't send", &why).await;
            }
            return Err(why);
        };
        let files = match share::expand(&paths) {
            Ok(f) => f,
            Err(e) => {
                if tell {
                    mac::notify("Couldn't send", &format!("{e:#}")).await;
                }
                return Err(format!("{e:#}"));
            }
        };
        let what = share::summary(&files);
        let (me, cert, commands, changes, name) = (self.me.clone(), d.cert.clone(), self.commands.clone(), self.changes.clone(), d.name.clone());
        tokio::spawn(async move {
            let count = files.len();
            let error = match share::send_files(&commands, &id, &me, &cert, files).await {
                Ok(()) => String::new(),
                Err(e) => format!("{e:#}"),
            };
            if tell {
                if error.is_empty() {
                    mac::notify(&format!("Sent to {name}"), &what).await;
                } else {
                    mac::notify(&format!("Couldn't send to {name}"), &error).await;
                }
            }
            let _ = changes.send(json!({ "event": "FilesSent", "id": id, "count": count, "error": error }));
        });
        Ok(Value::Null)
    }

    /// Phones showing the list get the new one.
    fn commands_changed(&self, devices: &HashMap<String, Device>) {
        for (id, d) in devices {
            if d.takes(RUNCOMMAND) {
                let _ = self.commands.send(Command::Send(id.clone(), self.run.list_packet()));
            }
        }
    }
}

/// A device as the window reads it (the Linux app's `Details`).
fn details(id: &str, d: &Device, permits: &Permits) -> Value {
    let mut v = json!({
        "id": id, "name": d.name, "paired": d.paired, "linked": d.linked, "key": d.key, "asking": d.asking,
        "via": if d.linked { "wifi" } else { "" },
        "can": { "share": d.takes(SHARE), "ring": d.takes(FIND), "browse": false, "text": d.takes(messages::REQUEST) },
        // No cable here, so nothing to allow for the clipboard.
        "clipboardSync": "on",
        "permits": PERMITS.iter().map(|&p| {
            let (name, detail) = permit_words(p);
            json!({ "key": p.key(), "name": name, "detail": detail, "on": permits.allows(id, p) })
        }).collect::<Vec<_>>(),
    });
    if let Some((charge, charging, low)) = d.battery {
        v["battery"] = json!({ "charge": charge, "charging": charging, "low": low });
    }
    v
}

/// The window's requests after which it should read again (the rest only read, or tell by their own events).
const CHANGING: &[&str] = &["Pair", "Accept", "Reject", "Unpair", "SetPermission", "SaveCommand", "RemoveCommand", "SetUpdateChecks"];

/// Whether [event] changes what the window shows of a device: not the packets passing through,
/// except the battery it shows.
fn changes_details(event: &Event) -> bool {
    match event {
        Event::Packet { packet, .. } => packet["type"] == BATTERY,
        _ => true,
    }
}

/// A JSON list of strings (the files of a send).
fn strings(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|p| p.as_str().map(str::to_owned)).collect()).unwrap_or_default()
}

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

fn url_encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

fn identity(me: &Identity, name: String, laptop: bool) -> Value {
    packet(
        "kdeconnect.identity",
        json!({
            "deviceId": me.device_id,
            "deviceName": name,
            "deviceType": if laptop { "laptop" } else { "desktop" },
            "protocolVersion": 8,
            "incomingCapabilities": ([INCOMING, vyoma::INCOMING].concat()),
            "outgoingCapabilities": ([OUTGOING, vyoma::OUTGOING].concat()),
        }),
    )
}

fn battery_packet(b: mac::Battery) -> Value {
    packet(BATTERY, json!({ "currentCharge": b.charge, "isCharging": b.charging, "thresholdEvent": 0 }))
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
    let Some(note) = b["id"].as_str() else { return };
    // Its own per phone, so two phones' ids never meet.
    let key = format!("phone-{id}-{note}");
    if b["isCancel"].as_bool() == Some(true) {
        mac::withdraw(&key);
        return;
    }
    if b["silent"].as_bool() == Some(true) {
        return;
    }
    let s = |k: &str| b[k].as_str().unwrap_or("").trim().to_owned();
    let (app, title, text) = (s("appName"), s("title"), s("text"));
    if title.is_empty() && text.is_empty() {
        return;
    }
    let title = if title.is_empty() { app.clone() } else { title };
    let from = if app.is_empty() { d.name.clone() } else { format!("{app} \u{b7} {}", d.name) };
    mac::phone_notification(&key, &from, &title, &text).await;
}

/// Handles one packet from a paired device; what to send back.
#[allow(clippy::too_many_arguments)]
async fn on_packet(id: &str, d: &mut Device, p: &Value, from: IpAddr, me: &Arc<Identity>, clip: &mut Clipboard, ringer: &mut mac::Ringer, found: &mpsc::UnboundedSender<()>, activity: &Arc<Activity>) -> Vec<Value> {
    let body = &p["body"];
    match p["type"].as_str().unwrap_or("") {
        PING => mac::notify(&d.name, body["message"].as_str().unwrap_or("Ping!")).await,
        BATTERY => {
            let charge = body["currentCharge"].as_i64().or_else(|| body["currentCharge"].as_str()?.parse().ok()).unwrap_or(-1);
            let was_low = d.battery.is_some_and(|b| b.2);
            d.battery = (0..=100).contains(&charge).then(|| (charge, body["isCharging"].as_bool().unwrap_or(false), body["thresholdEvent"].as_i64() == Some(1)));
            if let Some((charge, _, true)) = d.battery
                && !was_low
            {
                mac::notify(&format!("{}'s battery is low", d.name), &format!("{charge}% left")).await;
            }
        }
        FIND => {
            ringer.start();
            let (found, text) = (found.clone(), format!("{} is looking for this Mac.", d.name));
            tokio::spawn(async move {
                if let Ok(child) = mac::ask("Vyoma", &text, "Stop", "Found it", Duration::from_secs(60)) {
                    let _ = child.wait_with_output().await;
                }
                let _ = found.send(());
            });
        }
        LOCK_REQUEST => {
            if body["setLocked"].as_bool() == Some(true) {
                match mac::sleep_display().await {
                    Ok(()) => tracing::info!("{}: locked this Mac", d.name),
                    Err(e) => tracing::warn!("{}: {e:#}", d.name),
                }
            }
        }
        CLIPBOARD | CLIPBOARD_CONNECT => {
            if let Some(text) = clip.received(p)
                && let Err(e) = mac::set_clipboard(&text).await
            {
                tracing::warn!("couldn't set the clipboard: {e:#}");
            }
        }
        SHARE => match share::classify(p) {
            Shared::File { name, .. } => {
                let (me, cert, phone, packet, activity, id) = (me.clone(), d.cert.clone(), d.name.clone(), p.clone(), activity.clone(), id.to_owned());
                let from = d.remote.unwrap_or(from);
                tokio::spawn(async move {
                    match share::receive(&me, &cert, from, &packet, &mac::downloads()).await {
                        Ok(path) => {
                            let saved = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(name);
                            activity.add(&id, &phone, "share", format!("Shared \u{201c}{saved}\u{201d} with this Mac"));
                            mac::notify(&format!("From {phone}"), &format!("{saved} is in Downloads")).await;
                        }
                        Err(e) => {
                            tracing::warn!("{name} from {phone}: {e:#}");
                            mac::notify(&format!("{name} didn't arrive"), "The phone stopped sending it. Try again.").await;
                        }
                    }
                });
            }
            Shared::Url(url) => mac::open(&url),
            Shared::Text(text) => {
                if mac::set_clipboard(&text).await.is_ok() {
                    // Ours, so the poll doesn't send it straight back.
                    clip.changed(text);
                    mac::notify(&format!("Text from {}", d.name), "Copied. Paste it anywhere.").await;
                }
            }
            Shared::Nothing => {}
        },
        SHARE_UPDATE => {}
        other => tracing::debug!("{}: nothing here for {other}", d.name),
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let v = details("p", &d, &permits);
        assert_eq!((v["id"].as_str(), v["key"].as_str(), v["asking"].as_bool()), (Some("p"), Some("1A2B"), Some(true)));
        assert_eq!((v["can"]["share"].as_bool(), v["can"]["ring"].as_bool()), (Some(true), Some(false)), "only what it takes");
        assert_eq!(v["battery"]["charge"], 64);
        assert!(details("q", &Device::default(), &permits)["battery"].is_null());
        // Only the switches this Mac has, in Mac words.
        let keys: Vec<(&str, bool)> = v["permits"].as_array().unwrap().iter().map(|p| (p["key"].as_str().unwrap(), p["on"].as_bool().unwrap())).collect();
        assert_eq!(keys, [("control", true), ("files", false), ("power", true)]);
        assert_eq!(v["permits"][2]["name"], "Wake, sleep and shut down");
        assert_eq!(url_encode("Sam's Mac"), "Sam%27s%20Mac");
    }

    #[test]
    fn it_says_what_it_does() {
        let dir = std::env::temp_dir().join(format!("vyoma-mac-id-{}", std::process::id()));
        let me = Identity::load_or_create_in(&dir).unwrap();
        let p = identity(&me, "Sam's MacBook".into(), true);
        assert_eq!(p["body"]["deviceType"], "laptop");
        assert_eq!(p["body"]["incomingCapabilities"].as_array().unwrap().len(), INCOMING.len() + vyoma::INCOMING.len());
        assert!(!OUTGOING.contains(&"kdeconnect.lock"), "no lock state it can't know");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
