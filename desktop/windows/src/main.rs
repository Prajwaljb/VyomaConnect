//! Vyoma's PC side on Windows (docs/PLAN.md section 11), on the same core and engine as the Linux
//! app and the Mac's: it finds and links with phones, pairs after asking (a dialog with the code to
//! compare, or Vyoma's window), and serves the stock plugins: ping, battery both ways, find my
//! phone, the screen lock, the clipboard both ways, sharing (files into Downloads, web links
//! opened, text onto the clipboard), the touchpad and keyboard ([input]), the players and the
//! volume ([media]), the presenter's pointer ([presenter]), the phone's notifications (toasts with Reply and their buttons, [toast]), the
//! phone's texts for the window's Messages page, calls ([calls]), and the commands set up in the
//! window. Vyoma's own features go through the engine ([platform]; the live screen is [cast],
//! brightness and app volumes [pc], the wallpaper's colours [wallpaper], walk-away rules [away],
//! the phone's screen in a window here [mirror], a photo with this PC's camera [camera]).
//! It checks for a newer release once a day ([update]), is in Explorer's Send to ([sendto]), and
//! links over the USB cable through adb ([usb]). The phone is a game controller through the
//! usbip-win2 driver ([gamepad]).
//!
//! It lives in the notification area ([tray]) and starts with Windows once installed
//! (`--install`); Vyoma's window talks to it over a named pipe ([control]). The installer stops it
//! with `--quit`; This PC's firewall check lets phones through with `--open-firewall` ([firewall]).
//!
//! For tests, as the Linux app's: `VYOMA_DIR` (its folder), `VYOMA_PIPE` (the window's pipe),
//! `VYOMA_ALLOW` (device ids, comma-separated: every other device is ignored), `VYOMA_ANNOUNCE`
//! (addresses, `ip` or `ip:port`, announced to instead of the broadcast address), and its own
//! ports: `VYOMA_UDP_PORT`, `VYOMA_TCP_PORTS` (`first-last`, also the ports it dials).
//! Built and tested on GitHub's Windows runners and under Wine only; not yet run on a real Windows PC.

#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

#[cfg(windows)]
mod away;
#[cfg(windows)]
mod calls;
#[cfg(windows)]
mod camera;
#[cfg(windows)]
mod cast;
#[cfg(windows)]
mod control;
#[cfg(windows)]
mod firewall;
#[cfg(windows)]
mod gamepad;
#[cfg(windows)]
mod gpu;
#[cfg(windows)]
mod input;
#[cfg(windows)]
mod media;
#[cfg(windows)]
mod mirror;
#[cfg(windows)]
mod pc;
#[cfg(windows)]
mod platform;
#[cfg(windows)]
mod presenter;
#[cfg(windows)]
mod screen;
#[cfg(windows)]
mod sendto;
#[cfg(windows)]
mod system;
#[cfg(windows)]
mod toast;
#[cfg(windows)]
mod tray;
#[cfg(windows)]
mod update;
#[cfg(windows)]
mod usb;
#[cfg(windows)]
mod wallpaper;
#[cfg(windows)]
mod win;

#[cfg(not(windows))]
fn main() {
    eprintln!("vyoma-windows is Vyoma's PC side for Windows; on Linux run vyoma-desktop, on a Mac Vyoma.app.");
    std::process::exit(2);
}

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    agent::main()
}

#[cfg(windows)]
mod agent {
    use std::collections::HashMap;
    use std::net::{IpAddr, SocketAddr};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
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
    use vyoma_core::lan::{Lan, Ports, Rules};
    use vyoma_core::link::packet;
    use vyoma_core::messages::{self, Store};
    use vyoma_core::permits::{self, Permit, Permits};
    use vyoma_core::release::REPO;
    use vyoma_core::share::{self, SHARE, SHARE_UPDATE, Shared};
    use vyoma_core::telephony;
    use vyoma_core::trust::TrustStore;
    use vyoma_engine::session::{Host, Phone, phone_name};

    use crate::toast::{self, Button, Heard, Shown, Toast};
    use crate::{away, calls, control, firewall, input, media, platform, presenter, system, tray, update, usb, win};
    use vyoma_core::adb::ClipboardSeen;

    pub const PING: &str = "kdeconnect.ping";
    pub const BATTERY: &str = "kdeconnect.battery";
    pub const FIND: &str = "kdeconnect.findmyphone.request";
    pub const LOCK_REQUEST: &str = "kdeconnect.lock.request";
    pub const NOTIFICATION: &str = "kdeconnect.notification";
    /// PC → phone: `{cancel: id}` (closed here), the reply typed in a toast, one of its buttons.
    const NOTIFICATION_REQUEST: &str = "kdeconnect.notification.request";
    const NOTIFICATION_REPLY: &str = "kdeconnect.notification.reply";
    const NOTIFICATION_ACTION: &str = "kdeconnect.notification.action";
    pub const KEYBOARDSTATE: &str = "kdeconnect.mousepad.keyboardstate";
    /// PC → phone just before the agent stops (`why`: stop): the phone stops looking for it.
    pub const LEAVING: &str = "vyoma.pc.leaving";
    /// PC → phone: `{number}`, call it (Messages); the phone checks it all again.
    const DIAL: &str = "vyoma.call.dial";

    /// What we take and send. No lock state outward yet (WTS session notifications, later).
    const INCOMING: &[&str] = &[
        PING, BATTERY, FIND, LOCK_REQUEST, CLIPBOARD, CLIPBOARD_CONNECT, SHARE, SHARE_UPDATE, RUNCOMMAND_REQUEST, NOTIFICATION, input::MOUSEPAD_REQUEST,
        media::MPRIS_REQUEST, media::VOLUME_REQUEST, telephony::TELEPHONY, telephony::DONE, presenter::PRESENTER,
        messages::MESSAGES, messages::CONTACTS_UIDS_REPLY, messages::CONTACTS_VCARDS_REPLY, messages::UNLOCKED, messages::LOCKED,
    ];
    const OUTGOING: &[&str] = &[
        PING, BATTERY, FIND, CLIPBOARD, CLIPBOARD_CONNECT, SHARE, SHARE_UPDATE, RUNCOMMAND, input::MOUSEPAD_ECHO, KEYBOARDSTATE, media::MPRIS, media::VOLUME,
        messages::REQUEST, messages::REQUEST_CONVERSATIONS, messages::REQUEST_CONVERSATION, messages::CONTACTS_UIDS, messages::CONTACTS_VCARDS,
        messages::UNLOCK, messages::LOCK, DIAL, LEAVING, NOTIFICATION_REQUEST, NOTIFICATION_REPLY, NOTIFICATION_ACTION, telephony::MUTE,
        telephony::REQUEST,
    ];

    /// This release, as the Linux app reads it: the repo's one version.
    const VERSION: &str = include_str!("../../../VERSION");
    const CLIPBOARD_EVERY: Duration = Duration::from_millis(500);
    const BATTERY_EVERY: Duration = Duration::from_secs(60);
    /// Unused texts lock again after a while.
    const TEXTS_EVERY: Duration = Duration::from_secs(3);
    /// The touchpad counts as one use in Activity until it's left alone this long.
    const TOUCHPAD_STRETCH: Duration = Duration::from_secs(10 * 60);
    /// The window's program, beside this one.
    const WINDOW_EXE: &str = "vyoma.exe";
    const SEND_ANY: &str = "--send-any";
    /// The log is started afresh past this.
    const LOG_MAX: u64 = 2 << 20;

    /// The switches this PC has, in the window's order: what it serves so far.
    const PERMITS: [Permit; 5] = [Permit::Screen, Permit::Control, Permit::Files, Permit::Camera, Permit::Power];

    fn permit_words(p: Permit) -> (&'static str, &'static str) {
        match p {
            Permit::Screen => ("Screenshots", "The phone can take a screenshot of this PC"),
            Permit::Control => ("Touchpad and keyboard", "Move the pointer, type, paste, switch desktops"),
            Permit::Power => ("Wake, sleep and shut down", "Also sign out and restart"),
            other => (other.name(), other.detail()),
        }
    }

    const AUTOSTART_CHECK: &str = "Start with Windows";

    pub fn main() -> Result<()> {
        match std::env::args().nth(1).as_deref() {
            // From the window's first open: start with Windows from now on, and start now unless running.
            Some("--install") => {
                system::install()?;
                if !system::agent_running() {
                    std::process::Command::new(std::env::current_exe()?).spawn()?;
                }
                Ok(())
            }
            Some("--uninstall") => {
                toast::unregister();
                system::uninstall()
            }
            // Let phones through, as an administrator (Windows asked; see firewall::open).
            Some(firewall::OPEN_ARG) => firewall::allow(),
            // Windows starting the agent for a toast's button while it wasn't running.
            Some("-Embedding") => run(win::data_dir()),
            Some(SEND_ANY) => send_any(std::env::args().skip(2).collect()),
            // The phone's screen in a window, for the agent (see mirror::show).
            Some(crate::mirror::WINDOW_ARG) => crate::mirror::window(&std::env::args().skip(2).collect::<Vec<_>>()),
            // The installer, before it replaces or removes the programs.
            Some("--quit") => {
                quit_running();
                Ok(())
            }
            Some(other) => bail!("unknown option {other}: try --install, --uninstall or --quit"),
            None => run(win::data_dir()),
        }
    }

    /// A device as this PC knows it.
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

    /// The log, in Vyoma's folder (a program in the notification area has no console).
    fn start_log(dir: &std::path::Path) {
        let path = dir.join("vyoma.log");
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > LOG_MAX) {
            let _ = std::fs::remove_file(&path);
        }
        let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "vyoma_windows=info,vyoma_core=info,vyoma_engine=info".into());
        match std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            Ok(file) => tracing_subscriber::fmt().with_env_filter(filter).with_ansi(false).with_writer(Mutex::new(file)).init(),
            Err(_) => tracing_subscriber::fmt().with_env_filter(filter).init(),
        }
    }

    /// Screenshots and touches in real pixels on every display, not Windows' scaled guess.
    fn per_monitor_dpi() {
        use windows_sys::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
        // SAFETY: called once at start, before any window.
        unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    }

    #[tokio::main]
    async fn run(dir: PathBuf) -> Result<()> {
        std::fs::create_dir_all(&dir)?;
        start_log(&dir);
        per_monitor_dpi();
        let Some(_only_agent) = system::only_agent() else {
            tracing::info!("another agent is running in this session");
            return Ok(());
        };
        vyoma_core::identity::use_dir(dir.clone());
        let me = Arc::new(Identity::load_or_create_in(&dir)?);
        let trust = TrustStore::open(&dir)?;
        let mut devices: HashMap<String, Device> = trust
            .ids()
            .map(|id| (id.clone(), Device { name: trust.get(id).unwrap().name.clone(), paired: true, ..Default::default() }))
            .collect();

        let (said_tx, mut said) = mpsc::unbounded_channel();
        let tray = tray::start(said_tx);
        if tray.is_none() {
            tracing::warn!("no notification area icon");
        }
        // Toasts' buttons and closes come back here.
        let (heard_tx, mut heard) = mpsc::unbounded_channel::<Heard>();
        if toast::start(heard_tx) {
            tracing::info!("toasts ready");
        }

        let mut battery = win::battery();
        let (events_tx, mut events) = mpsc::unbounded_channel();
        let hub = Hub::new(me.cert.to_vec(), trust, events_tx);
        let linked = hub.linked();
        let allow = list_env("VYOMA_ALLOW");
        let rules = Rules {
            admits: Box::new(move |id| allow.as_ref().is_none_or(|a| a.iter().any(|x| x == id))),
            linked: Box::new(move |id| linked.lock().unwrap().contains(id)),
        };
        let ports = ports()?;
        let everyone = announce_to(ports.udp)?;
        let lan = Lan::bind(me.clone(), identity(&me, win::name(), battery.is_some()), rules, ports).await?;
        let (links_tx, links) = mpsc::channel(16);
        let (commands, commands_rx) = mpsc::unbounded_channel();
        tokio::spawn(lan.clone().run(links_tx));
        tokio::spawn(hub.run(links, commands_rx));
        lan.announce(&everyone).await?;
        tracing::info!("vyoma-windows is up as {}", me.device_id);

        // The window's pipe.
        let (requests_tx, mut requests) = mpsc::channel::<control::Request>(16);
        let (changes, _) = broadcast::channel::<Value>(64);
        {
            let (changes, requests) = (changes.clone(), requests_tx.clone());
            tokio::spawn(async move {
                if let Err(e) = control::serve(control::pipe_name(), requests, changes).await {
                    tracing::warn!("no window pipe: {e:#}");
                }
            });
        }
        let run_commands = Commands::new(&dir);
        // Newer releases with a Windows installer, once a day.
        let updates = update::load(&dir, VERSION);
        {
            let changes = changes.clone();
            tokio::spawn(update::run(updates.clone(), move || {
                let _ = changes.send(json!({ "event": "Changed" }));
            }));
        }
        let permits = Arc::new(Permits::load(dir.join("permissions.json")));
        let activity = Arc::new(Activity::load(dir.join("activity.json")));
        forward(activity.subscribe(), changes.clone(), "ActivityChanged");
        let texts = Arc::new(Store::default());
        forward(texts.subscribe(), changes.clone(), "MessagesChanged");
        let mut refused: HashMap<(String, Permit), Instant> = HashMap::new();
        let mut touched: HashMap<String, Instant> = HashMap::new();
        // Vyoma's own features: the engine runs each paired phone's session, Windows does the work.
        let pc = Arc::new(platform::Windows::default());
        let host = Arc::new(Host { me: me.clone(), permits: permits.clone(), activity: activity.clone() });
        let mut sessions: HashMap<String, mpsc::UnboundedSender<(Value, IpAddr)>> = HashMap::new();
        let engine_takes = vyoma_engine::session::incoming_all();
        let window = Window { me: me.clone(), lan: lan.clone(), commands: commands.clone(), changes: changes.clone(), everyone, run: run_commands.clone(), texts: texts.clone(), permits: permits.clone(), activity: activity.clone(), requests: requests_tx, pending: Mutex::default(), updates: updates.clone(), clip_seen: ClipboardSeen::load(&dir) };
        let mut pointer = input::Pointer::default();
        let mut dot = presenter::Presenter::default();
        // Players and the volume, read on a thread of their own while a phone that takes them is linked.
        let media_wanted = Arc::new(AtomicBool::new(false));
        let (media_tx, media_rx) = std::sync::mpsc::channel::<(String, Value)>();
        let (media_out_tx, mut media_out) = mpsc::unbounded_channel::<media::Out>();
        {
            let wanted = media_wanted.clone();
            std::thread::Builder::new().name("vyoma-media".into()).spawn(move || media::run(wanted, media_rx, media_out_tx))?;
        }
        let mut untrusted_told: Option<Instant> = None;
        // Calls on the phones: toasts, music paused for them.
        let calls = calls::start();
        // The USB cable: phones plugged in with USB debugging link through adb's tunnel.
        usb::start(lan.tcp_port());
        // The controller's states down the cable (adb's reverse of the phone's 1866); pads a
        // previous run left behind aren't retried.
        tokio::spawn(vyoma_core::gamepad::serve_wired());
        tokio::task::spawn_blocking(crate::gamepad::tidy);
        {
            let changes = changes.clone();
            vyoma_core::adb::on_clipboard_change(Box::new(move || {
                let _ = changes.send(json!({ "event": "Changed" }));
            }));
        }

        let send = |id: &str, p: Value| {
            let _ = commands.send(Command::Send(id.to_owned(), p));
        };

        // The clipboard: Windows counts its changes, so only a new count is read.
        let (copies_tx, mut copies) = mpsc::unbounded_channel::<String>();
        tokio::spawn(async move {
            let mut seen = win::clipboard_changes();
            let mut tick = tokio::time::interval(CLIPBOARD_EVERY);
            loop {
                tick.tick().await;
                let now = win::clipboard_changes();
                if now == seen {
                    continue;
                }
                seen = now;
                if let Ok(Some(text)) = tokio::task::spawn_blocking(win::clipboard).await
                    && text.len() <= MAX_TEXT
                    && copies_tx.send(text).is_err()
                {
                    break;
                }
            }
        });
        let mut clip = Clipboard::default();
        let mut battery_tick = tokio::time::interval(BATTERY_EVERY);
        battery_tick.tick().await;
        let mut texts_tick = tokio::time::interval(TEXTS_EVERY);

        // Answers to the pairing questions on screen: the phone, the code asked about, yes or no.
        let (answers_tx, mut answers) = mpsc::unbounded_channel::<(String, String, bool)>();
        let mut ringer = win::Ringer::default();

        loop {
            tokio::select! {
                Some(event) = events.recv() => {
                    if changes_details(&event) {
                        let _ = changes.send(json!({ "event": "Changed" }));
                    }
                    match event {
                        Event::Linked { id, name, paired, incoming, cert, remote, .. } => {
                            let d = devices.entry(id.clone()).or_default();
                            *d = Device { name, paired, linked: true, incoming, cert, remote: Some(remote), battery: d.battery, ..Default::default() };
                            tracing::info!("{id}: linked{}", if paired { ", paired" } else { "" });
                            if paired {
                                for p in started(d, battery, &clip) {
                                    send(&id, p);
                                }
                                start_session(&mut sessions, &pc, &host, &commands, &id, d);
                                // Back: a pending walk-away is off, and what one did is put back.
                                let (rules, phone) = (pc.away.clone(), id.clone());
                                tokio::spawn(async move { rules.linked(&phone, &away::Hands).await });
                            }
                        }
                        Event::Gone { id } => {
                            if let Some(d) = devices.get_mut(&id) {
                                d.linked = false;
                            }
                            sessions.remove(&id);
                            texts.gone(&id);
                            pc.away.gone(&id, Arc::new(away::Hands));
                            dot.stop();
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
                            if toast::show(&pair_toast(&id, &name, &key)) != Shown::Yes {
                                // No toasts (or turned off): a dialog asks instead.
                                let answers = answers_tx.clone();
                                let text = format!("{name} wants to pair with this PC.\n\nPair only if the phone shows this same code:\n\n{key}\n\nPair now?");
                                tokio::task::spawn_blocking(move || {
                                    let yes = win::ask("Vyoma", &text);
                                    let _ = answers.send((id, key, yes));
                                });
                            }
                        }
                        Event::PairingOver { id } => {
                            toast::remove(PAIR_TAG, &toast::short(&id));
                            if let Some(d) = devices.get_mut(&id) {
                                (d.key, d.asking) = (String::new(), false);
                            }
                        }
                        Event::Paired { id, name, .. } => {
                            toast::remove(PAIR_TAG, &toast::short(&id));
                            let d = devices.entry(id.clone()).or_default();
                            d.paired = true;
                            (d.key, d.asking) = (String::new(), false);
                            d.name = name.clone();
                            tray::notify(&format!("Paired with {name}"), "Your phone and this PC can now share files, links and the clipboard.");
                            for p in started(d, battery, &clip) {
                                send(&id, p);
                            }
                            start_session(&mut sessions, &pc, &host, &commands, &id, d);
                        }
                        Event::Unpaired { id, name } => {
                            if let Some(d) = devices.get_mut(&id) {
                                d.paired = false;
                            }
                            sessions.remove(&id);
                            texts.forget(&id);
                            permits.forget(&id);
                            tray::notify(&format!("{name} is no longer paired"), "Pair again from the phone to use it with this PC.");
                        }
                        Event::Asked { id, key, .. } => {
                            let d = devices.entry(id).or_default();
                            (d.key, d.asking) = (key, false);
                        }
                        Event::Packet { id, packet, from } => {
                            let Some(d) = devices.get_mut(&id) else { continue };
                            let kind = packet["type"].as_str().unwrap_or("").to_owned();
                            if engine_takes.contains(&kind.as_str()) {
                                // The engine checks the switches and keeps Activity for these itself.
                                if let Some(session) = sessions.get(&id).filter(|_| d.paired) {
                                    let _ = session.send((packet, from));
                                }
                                continue;
                            }
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
                                presenter::PRESENTER => dot.on_packet(&packet["body"]),
                                input::MOUSEPAD_REQUEST => {
                                    let now = Instant::now();
                                    if touched.insert(id.clone(), now).is_none_or(|t| now.duration_since(t) > TOUCHPAD_STRETCH) {
                                        activity.add(&id, &d.name, "touchpad", "Used as touchpad");
                                    }
                                    if let Some(echo) = input::echo(&packet["body"]) {
                                        send(&id, echo);
                                    }
                                    if let Err(e) = pointer.handle(&input::acts(&packet["body"])) {
                                        // An administrator's window or the secure desktop in front: said
                                        // on the phone every so often (moves stream).
                                        let now = Instant::now();
                                        if untrusted_told.is_none_or(|t| now.duration_since(t) > Duration::from_secs(15)) {
                                            untrusted_told = Some(now);
                                            tracing::info!("{id}: input refused: {e:#}");
                                            let mut no = permits::denied(Permit::Control, &packet);
                                            no["body"]["reason"] = json!("Windows isn't taking input right now (a window running as administrator, or the lock screen)");
                                            send(&id, no);
                                        }
                                    }
                                }
                                RUNCOMMAND_REQUEST => {
                                    let body = &packet["body"];
                                    if body["requestCommandList"].as_bool() == Some(true) {
                                        send(&id, run_commands.list_packet());
                                    }
                                    // The phone's "add command": the editor is Vyoma's window.
                                    if body["setup"].as_bool() == Some(true) {
                                        open_window(&["--page", "commands"]);
                                    }
                                    if let Some(key) = body["key"].as_str() {
                                        let ran = run_commands.run(key, |name, why| tray::notify(&format!("\u{201c}{name}\u{201d} didn't run"), &format!("{why}. Check it in Vyoma's Commands.")));
                                        match ran {
                                            Some(name) => {
                                                tracing::info!("{id}: ran a command");
                                                activity.add(&id, &d.name, "command", format!("Ran \u{201c}{name}\u{201d}"));
                                            }
                                            None => tracing::info!("{id}: no command {key}"),
                                        }
                                    }
                                }
                                NOTIFICATION => phone_notification(&id, d, &packet["body"]),
                                telephony::TELEPHONY => {
                                    let _ = calls.send(calls::Order::Packet { id: id.clone(), phone: d.name.clone(), packet });
                                }
                                telephony::DONE => {
                                    let b = &packet["body"];
                                    match b["error"].as_str() {
                                        Some(e) => tracing::info!("{id}: the phone couldn't {}: {e}", b["action"].as_str().unwrap_or("do that")),
                                        None => tracing::info!("{id}: call {}", b["action"].as_str().unwrap_or("done")),
                                    }
                                }
                                messages::UNLOCKED | messages::LOCKED => {
                                    tracing::info!("{id}: {kind}");
                                    texts.on_packet(&id, &packet);
                                }
                                messages::MESSAGES | messages::CONTACTS_UIDS_REPLY | messages::CONTACTS_VCARDS_REPLY => {
                                    for p in texts.on_packet(&id, &packet) {
                                        send(&id, p);
                                    }
                                }
                                _ => {
                                    for reply in on_packet(&id, d, &packet, from, &me, &mut clip, &mut ringer, &activity).await {
                                        send(&id, reply);
                                    }
                                }
                            }
                        }
                    }
                    media_wanted.store(devices.values().any(|d| d.takes(media::MPRIS) || d.takes(media::VOLUME)), Ordering::Relaxed);
                },
                Some(out) = media_out.recv() => match out {
                    media::Out::Packet(to, p) => {
                        let kind = p["type"].as_str().unwrap_or("").to_owned();
                        for (id, d) in &devices {
                            if d.takes(&kind) && to.as_ref().is_none_or(|t| t == id) {
                                send(id, p.clone());
                            }
                        }
                    }
                    // A cover the phone asked for, as a payload.
                    media::Out::Art { id, packet, bytes } => {
                        if let Some(d) = devices.get(&id).filter(|d| d.takes(media::MPRIS)) {
                            let (sender, me, cert) = (platform::sender(&commands, &id), me.clone(), d.cert.clone());
                            tokio::spawn(async move {
                                if let Err(e) = vyoma_core::payload::offer(&sender, &me, &cert, packet, bytes.len() as u64, bytes.as_slice()).await {
                                    tracing::info!("{id}: cover not sent: {e:#}");
                                }
                            });
                        }
                    }
                },
                Some(req) = requests.recv() => {
                    tracing::debug!("window: {}", req.method);
                    if req.method == "Quit" {
                        let _ = req.reply.send(Ok(Value::Null));
                        break;
                    }
                    let answer = window.answer(&req.method, &req.args, &mut devices).await;
                    // Only what the window would show differently: a reread after a read would come
                    // back here as another read, round and round.
                    if answer.is_ok() && CHANGING.contains(&req.method.as_str()) {
                        let _ = changes.send(json!({ "event": "Changed" }));
                    }
                    let _ = req.reply.send(answer);
                }
                Some(h) = heard.recv() => {
                    let (args, reply, closed) = match h {
                        Heard::Pressed { args, reply } => (toast::parse_args(&args), reply, false),
                        Heard::Closed { args } => (toast::parse_args(&args), None, true),
                    };
                    let get = |k: &str| args.get(k).cloned().unwrap_or_default();
                    let (what, id) = (get("do"), get("dev"));
                    let d = devices.get(&id);
                    match (what.as_str(), closed) {
                        ("pair", false) => {
                            if d.is_some_and(|d| d.asking && d.key == get("key")) {
                                let _ = commands.send(if get("yes") == "1" { Command::Accept(id) } else { Command::Reject(id) });
                            }
                        }
                        ("reply", false) => {
                            if let Some(text) = reply.filter(|r| !r.trim().is_empty())
                                && d.is_some_and(|d| d.takes(NOTIFICATION_REPLY))
                            {
                                send(&id, packet(NOTIFICATION_REPLY, json!({ "requestReplyId": get("rid"), "message": text })));
                            }
                        }
                        // A ringing toast's Silence or Decline.
                        ("call", false) => {
                            if let Some(p) = calls::request(&get("what"))
                                && d.is_some_and(|d| d.takes(p["type"].as_str().unwrap_or_default()))
                            {
                                send(&id, p);
                            }
                        }
                        ("action", false) => {
                            if d.is_some_and(|d| d.takes(NOTIFICATION_ACTION)) {
                                send(&id, packet(NOTIFICATION_ACTION, json!({ "key": get("key"), "action": get("action") })));
                            }
                        }
                        ("send", false) => {
                            let paths = window.pending.lock().unwrap().remove(&get("t"));
                            if let Some(paths) = paths {
                                let _ = window.send(id, paths, &devices, true).await;
                            }
                        }
                        // A phone's notification closed here: gone on the phone too, if it can be.
                        ("note", true) => {
                            if get("clear") == "1" && d.is_some_and(|d| d.takes(NOTIFICATION_REQUEST)) {
                                send(&id, packet(NOTIFICATION_REQUEST, json!({ "cancel": get("key") })));
                            }
                        }
                        // The cable's question: Allow or Not now (the toast itself opens Vyoma).
                        ("cable", false) => match args.get("yes") {
                            Some(yes) => usb::answered(&get("t"), yes == "1"),
                            None => open_window(&[]),
                        },
                        (_, false) => open_window(&[]),
                        _ => {}
                    }
                }
                Some((id, key, yes)) = answers.recv() => {
                    // Only for the pairing the question was about: a newer request has a new code.
                    if devices.get(&id).is_some_and(|d| d.asking && d.key == key) {
                        let _ = commands.send(if yes { Command::Accept(id) } else { Command::Reject(id) });
                    }
                }
                Some(text) = copies.recv() => {
                    if let Some(p) = clip.changed(text) {
                        for (id, d) in &devices {
                            if d.takes(CLIPBOARD) {
                                send(id, p.clone());
                            }
                        }
                    }
                }
                Some(s) = said.recv() => match s {
                    tray::Said::Open => open_window(&[]),
                    tray::Said::Quit | tray::Said::Ending => break,
                },
                _ = battery_tick.tick() => {
                    let now = win::battery();
                    if now != battery && let Some(b) = now {
                        for (id, d) in &devices {
                            if d.takes(BATTERY) {
                                send(id, battery_packet(b));
                            }
                        }
                    }
                    battery = now;
                }
                _ = texts_tick.tick() => {
                    for id in texts.expire() {
                        tracing::info!("{id}: texts locked again");
                        send(&id, packet(messages::LOCK, json!({})));
                    }
                }
                _ = tokio::signal::ctrl_c() => break,
            }
        }
        // Quitting, or Windows ending the session: phones stop looking for this PC until it
        // announces itself again.
        for (id, d) in &devices {
            if d.takes(LEAVING) {
                send(id, packet(LEAVING, json!({ "why": "stop" })));
            }
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
        if let Some(t) = tray {
            t.close();
        }
        Ok(())
    }

    /// A comma-separated list from the environment, if set.
    fn list_env(name: &str) -> Option<Vec<String>> {
        std::env::var(name).ok().map(|s| s.split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect())
    }

    /// KDE Connect's ports, unless a test gives its own.
    fn ports() -> Result<Ports> {
        let mut ports = Ports::default();
        if let Ok(udp) = std::env::var("VYOMA_UDP_PORT") {
            ports.udp = udp.trim().parse().map_err(|_| anyhow::anyhow!("VYOMA_UDP_PORT: {udp}"))?;
        }
        if let Ok(tcp) = std::env::var("VYOMA_TCP_PORTS") {
            let (a, b) = tcp.split_once('-').ok_or_else(|| anyhow::anyhow!("VYOMA_TCP_PORTS: first-last"))?;
            let (a, b): (u16, u16) = (a.trim().parse()?, b.trim().parse()?);
            (ports.tcp, ports.peers) = (a..=b, a..=b);
        }
        Ok(ports)
    }

    /// Whom to announce to: everyone on the network, or the addresses a test names.
    fn announce_to(udp: u16) -> Result<Vec<SocketAddr>> {
        let Some(list) = list_env("VYOMA_ANNOUNCE") else {
            return Ok(vec![SocketAddr::from(([255, 255, 255, 255], udp))]);
        };
        list.iter()
            .map(|a| a.parse::<SocketAddr>().or_else(|_| a.parse::<IpAddr>().map(|ip| SocketAddr::new(ip, udp))).map_err(|_| anyhow::anyhow!("VYOMA_ANNOUNCE: {a}")))
            .collect()
    }

    /// Passes a store's ticks on to the window as [event].
    fn forward(mut ticks: tokio::sync::watch::Receiver<u64>, changes: broadcast::Sender<Value>, event: &'static str) {
        tokio::spawn(async move {
            while ticks.changed().await.is_ok() {
                let _ = changes.send(json!({ "event": event }));
            }
        });
    }

    /// Opens Vyoma's window (it shows the page asked for in the one already open).
    fn open_window(args: &[&str]) {
        let Some(exe) = std::env::current_exe().ok().map(|e| e.with_file_name(WINDOW_EXE)).filter(|e| e.exists()) else {
            tray::notify("Vyoma's window isn't here", "Reinstall Vyoma to get it back.");
            return;
        };
        if let Err(e) = std::process::Command::new(&exe).args(args).spawn() {
            tracing::warn!("couldn't open the window: {e}");
        }
    }

    /// Window methods after which it should read everything again.
    const CHANGING: &[&str] = &["Pair", "Accept", "Reject", "Unpair", "SetPermission", "FixCheck", "SaveCommand", "RemoveCommand"];

    fn changes_details(event: &Event) -> bool {
        match event {
            Event::Packet { packet, .. } => packet["type"] == BATTERY,
            _ => true,
        }
    }

    /// What the window can ask for (the Linux app's D-Bus methods, the ones Windows has so far).
    struct Window {
        me: Arc<Identity>,
        lan: Arc<Lan>,
        commands: mpsc::UnboundedSender<Command>,
        changes: broadcast::Sender<Value>,
        everyone: Vec<SocketAddr>,
        run: Commands,
        texts: Arc<Store>,
        permits: Arc<Permits>,
        activity: Arc<Activity>,
        /// For a choice made in a dialog to come back as a request.
        requests: mpsc::Sender<control::Request>,
        /// Files waiting for the person to pick a phone in a toast, by the toast's token.
        pending: Mutex<HashMap<String, Vec<String>>>,
        updates: Arc<update::Updates>,
        /// Phones whose background clipboard sync was seen granted.
        clip_seen: ClipboardSeen,
    }

    impl Window {
        async fn answer(&self, method: &str, args: &Value, devices: &mut HashMap<String, Device>) -> Result<Value, String> {
            let command = |c: Command| self.commands.send(c).map(|_| Value::Null).map_err(|_| "Vyoma is stopping".to_owned());
            match method {
                "Phones" => {
                    let linked = devices.values().filter(|d| d.paired && d.linked).count();
                    Ok(Value::Array(devices.iter().map(|(id, d)| details(id, d, &self.permits, &self.clip_seen, linked)).collect()))
                }
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
                            tray::notify("No phone to send to", "Pair your phone in Vyoma first, then try again.");
                            Ok(Value::Null)
                        }
                        share::Pick::Ask(phones) => {
                            let what = if paths.len() == 1 { "this".to_owned() } else { format!("these {} items", paths.len()) };
                            // A toast with a button per phone, where there are toasts.
                            let token = toast::short(&format!("{paths:?}{:?}", Instant::now()));
                            let t = Toast {
                                tag: token.clone(),
                                group: "send".into(),
                                title: format!("Send {what} to which phone?"),
                                body: share::summary(&share::expand(&paths).unwrap_or_default()),
                                launch: toast::args(&[("do", "open")]),
                                buttons: phones.iter().map(|(id, name)| Button { label: name.clone(), args: toast::args(&[("do", "send"), ("dev", id), ("t", &token)]), with_reply: false }).collect(),
                                ..Default::default()
                            };
                            if phones.len() <= 5 && toast::show(&t) == Shown::Yes {
                                self.pending.lock().unwrap().insert(token, paths);
                                return Ok(Value::Null);
                            }
                            let requests = self.requests.clone();
                            // One question per phone, in turn, until one's picked.
                            tokio::task::spawn_blocking(move || {
                                let Some((id, _)) = phones.into_iter().find(|p| win::ask("Vyoma", &format!("Send {what} to {}?", p.1))) else { return };
                                let (reply, _) = tokio::sync::oneshot::channel();
                                let _ = requests.blocking_send(control::Request { method: "SendFiles".into(), args: json!([id, paths, true]), reply });
                            });
                            Ok(Value::Null)
                        }
                    }
                }
                "Refresh" => self.lan.announce(&self.everyone).await.map(|_| Value::Null).map_err(|e| format!("{e:#}")),
                "PairingCode" => {
                    // No address, no code: a phone couldn't reach this PC. The window says so (the error's word).
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
                        url_encode(&win::name()),
                        vyoma_core::invite::fingerprint(&self.me.cert),
                        addresses.join(","),
                        self.lan.tcp_port()
                    )))
                }
                "EndPairingCode" => command(Command::Invite(None)),
                // The window's look: the wallpaper's colours.
                "Wallpaper" => tokio::task::spawn_blocking(crate::wallpaper::for_window).await.map_err(|e| e.to_string())?.map_err(|e| format!("{e:#}")),
                "Health" => {
                    let checks = tokio::task::spawn_blocking(checks).await.map_err(|e| e.to_string())?;
                    Ok(json!({ "features": [], "installable": false, "checks": checks, "notes": [], "update": self.updates.status(), "version": VERSION.trim() }))
                }
                "FixCheck" => match control::text(args, 0)?.as_str() {
                    AUTOSTART_CHECK => system::install().map(|_| Value::Null).map_err(|e| format!("{e:#}")),
                    usb::CHECK => {
                        let changes = self.changes.clone();
                        tokio::spawn(async move {
                            if let Err(e) = usb::fetch_adb().await {
                                tracing::warn!("no adb: {e:#}");
                                let text = format!("{e:#}");
                                tokio::task::spawn_blocking(move || tray::notify("Couldn't get adb", &text));
                            }
                            let _ = changes.send(json!({ "event": "Changed" }));
                        });
                        Ok(Value::Null)
                    }
                    crate::gamepad::CHECK => {
                        let changes = self.changes.clone();
                        tokio::spawn(async move {
                            let installed = changes.clone();
                            let done = crate::gamepad::fetch_driver(move || {
                                let _ = installed.send(json!({ "event": "Changed" }));
                            });
                            if let Err(e) = done.await {
                                tracing::warn!("no controller driver: {e:#}");
                                let text = format!("{e:#}");
                                tokio::task::spawn_blocking(move || tray::notify("Couldn't get the controller driver", &text));
                            }
                            let _ = changes.send(json!({ "event": "Changed" }));
                        });
                        Ok(Value::Null)
                    }
                    crate::camera::CHECK => {
                        win::open(crate::camera::SETTINGS);
                        Ok(Value::Null)
                    }
                    _ => Err("Not on Windows yet".into()),
                },
                // Let phones through: Windows asks for an administrator; FirewallDone says how it went.
                "OpenFirewall" => {
                    let changes = self.changes.clone();
                    tokio::spawn(async move {
                        let done = tokio::task::spawn_blocking(firewall::open).await;
                        let ok = matches!(done, Ok(Ok(())));
                        if let Ok(Err(e)) = &done {
                            tracing::info!("firewall: {e:#}");
                        }
                        let _ = changes.send(json!({ "event": "FirewallDone", "ok": ok }));
                    });
                    Ok(Value::Null)
                }
                "AllowClipboardSync" => {
                    let done = vyoma_core::adb::allow_clipboard().await.map_err(|e| format!("{e:#}"))?;
                    tracing::info!("background clipboard sync allowed for {done} app(s)");
                    Ok(Value::Null)
                }
                "SetUpdateChecks" => {
                    self.updates.set_checks(args[0].as_bool().ok_or("on or off?")?);
                    Ok(Value::Null)
                }
                // Update: the release's page, where the installer is.
                "Update" => {
                    win::open(&self.updates.page().ok_or("No newer Vyoma for Windows yet")?);
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
                    let can_call = d.takes(DIAL);
                    for p in ask {
                        command(Command::Send(id.clone(), p))?;
                    }
                    let mut v: Value = serde_json::from_str(&json).map_err(|e| e.to_string())?;
                    // Calls go through the phone, their sound staying there (no hands-free here yet).
                    (v["canCall"], v["callHere"]) = (json!(can_call), json!(false));
                    Ok(v)
                }
                "Contacts" | "SearchTexts" => {
                    let id = control::text(args, 0)?;
                    let d = devices.get(&id).filter(|d| d.takes(messages::REQUEST)).ok_or("This phone doesn't share its texts")?;
                    let guarded = d.takes(messages::UNLOCK);
                    let (json, ask) = if method == "Contacts" { self.texts.contacts(&id, guarded) } else { self.texts.search(&id, &control::text(args, 1)?, guarded) };
                    for p in ask {
                        command(Command::Send(id.clone(), p))?;
                    }
                    serde_json::from_str(&json).map_err(|e| e.to_string())
                }
                "ThreadFor" => {
                    let id = control::text(args, 0)?;
                    let d = devices.get(&id).filter(|d| d.takes(messages::REQUEST)).ok_or("This phone doesn't share its texts")?;
                    Ok(json!(self.texts.thread_for(&id, &control::text(args, 1)?, d.takes(messages::UNLOCK))?))
                }
                "Call" => {
                    let id = control::text(args, 0)?;
                    let d = devices.get(&id).filter(|d| d.linked).ok_or("The phone isn't connected")?;
                    if !d.takes(DIAL) {
                        return Err("Update Vyoma on the phone to call from here".into());
                    }
                    let number = messages::dialable(&control::text(args, 1)?).ok_or("That isn't a number to call")?;
                    self.texts.may_call(&id, d.takes(messages::UNLOCK))?;
                    tracing::info!("{id}: asking the phone to make a call");
                    command(Command::Send(id, packet(DIAL, json!({ "number": number }))))
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
                _ => Err("Not on Windows yet".into()),
            }
        }

        /// Sends [paths] (files and folders) to [id]; with [tell], notifications say how it went.
        async fn send(&self, id: String, paths: Vec<String>, devices: &HashMap<String, Device>, tell: bool) -> Result<Value, String> {
            let Some(d) = devices.get(&id).filter(|d| d.takes(SHARE)) else {
                let name = devices.get(&id).map_or("The phone", |d| d.name.as_str());
                let why = format!("{name} isn't connected. Open Vyoma on it, then try again.");
                if tell {
                    tray::notify("Couldn't send", &why);
                }
                return Err(why);
            };
            let files = share::expand(&paths).map_err(|e| {
                if tell {
                    tray::notify("Couldn't send", &format!("{e:#}"));
                }
                format!("{e:#}")
            })?;
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
                        tray::notify(&format!("Sent to {name}"), &what);
                    } else {
                        tray::notify(&format!("Couldn't send to {name}"), &error);
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

    /// A device as the window reads it (the Linux app's `Details`). [linked]: how many paired phones are linked.
    fn details(id: &str, d: &Device, permits: &Permits, clip_seen: &ClipboardSeen, linked: usize) -> Value {
        let via = via(d);
        let mut v = json!({
            "id": id, "name": d.name, "paired": d.paired, "linked": d.linked, "key": d.key, "asking": d.asking,
            "via": via,
            "can": { "share": d.takes(SHARE), "ring": d.takes(FIND), "browse": false, "text": d.takes(messages::REQUEST) },
            // The cable's one-time Allow for background clipboard sync (the window's Clipboard card).
            "clipboardSync": clip_seen.state(id, d.paired && d.linked && (via == "cable" || linked == 1)),
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

    /// How a linked phone is reached: loopback is the USB cable (adb's tunnel).
    fn via(d: &Device) -> &'static str {
        match d.remote.filter(|_| d.linked) {
            None => "",
            Some(r) if r.is_loopback() => "cable",
            Some(_) => "wifi",
        }
    }

    /// This PC's checks for the window's This PC page (blocking: the firewall's rules are read).
    fn checks() -> Value {
        let autostart = system::installed();
        let mut all = vec![json!({
            "name": AUTOSTART_CHECK,
            "state": if autostart { "ok" } else { "warn" },
            "detail": if autostart { "Vyoma starts when you sign in" } else { "Vyoma runs only until you sign out. Turn on to start it with Windows" },
            "action": if autostart { "" } else { "Turn on" },
        })];
        all.extend(firewall::check());
        all.push(usb::check());
        all.push(crate::gamepad::check());
        all.extend(crate::camera::check());
        Value::Array(all)
    }

    /// A JSON list of strings (the files of a send).
    fn strings(v: &Value) -> Vec<String> {
        v.as_array().map(|a| a.iter().filter_map(|p| p.as_str().map(str::to_owned)).collect()).unwrap_or_default()
    }

    fn url_encode(s: &str) -> String {
        s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
    }

    /// `--send-any [paths]` (Explorer's Send to): hands the files to the running agent, which
    /// picks the phone and says how it went.
    fn send_any(paths: Vec<String>) -> Result<()> {
        use std::io::{BufRead, Write};
        if paths.is_empty() {
            return Ok(());
        }
        let here = std::env::current_dir().unwrap_or_default();
        let paths: Vec<String> = paths.iter().map(|p| here.join(p).to_string_lossy().into_owned()).collect();
        let pipe = control::pipe_name();
        for _ in 0..20 {
            if let Ok(mut stream) = std::fs::OpenOptions::new().read(true).write(true).open(&pipe) {
                writeln!(stream, "{}", json!({ "method": "SendAny", "args": [paths] }))?;
                let mut line = String::new();
                std::io::BufReader::new(stream).read_line(&mut line)?;
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        win::tell("Couldn't send", "Vyoma isn't running right now. Start Vyoma, then try again.");
        Ok(())
    }

    /// `--quit`: closes Vyoma's window and stops this session's agent (it tells phones it's
    /// leaving), waiting a few seconds for both to be gone.
    fn quit_running() {
        use std::io::{BufRead, Write};
        system::close_windows();
        if system::agent_running()
            && let Ok(mut pipe) = std::fs::OpenOptions::new().read(true).write(true).open(control::pipe_name())
        {
            let _ = writeln!(pipe, "{}", json!({ "method": "Quit", "args": [] }));
            let mut line = String::new();
            let _ = std::io::BufReader::new(pipe).read_line(&mut line);
        }
        let until = Instant::now() + Duration::from_secs(5);
        while (system::agent_running() || system::window_running()) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Starts phone [id]'s engine session, unless one is running; dropping its entry ends it.
    fn start_session(
        sessions: &mut HashMap<String, mpsc::UnboundedSender<(Value, IpAddr)>>,
        pc: &Arc<platform::Windows>,
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
        tokio::spawn(vyoma_engine::session::serve(pc.clone(), host.clone(), phone, platform::sender(commands, id), rx));
    }

    fn identity(me: &Identity, name: String, laptop: bool) -> Value {
        let (engine_in, engine_out) = vyoma_engine::platform::capabilities_for(platform::Windows::offers);
        packet(
            "kdeconnect.identity",
            json!({
                "deviceId": me.device_id,
                "deviceName": name,
                "deviceType": if laptop { "laptop" } else { "desktop" },
                "protocolVersion": 8,
                "incomingCapabilities": ([INCOMING, &engine_in].concat()),
                "outgoingCapabilities": ([OUTGOING, &engine_out].concat()),
            }),
        )
    }

    fn battery_packet(b: win::Battery) -> Value {
        packet(BATTERY, json!({ "currentCharge": b.charge, "isCharging": b.charging, "thresholdEvent": 0 }))
    }

    /// What a paired device that just linked is sent: this PC's battery, its clipboard, and that
    /// it takes typing (the phone's keyboard button).
    fn started(d: &Device, battery: Option<win::Battery>, clip: &Clipboard) -> Vec<Value> {
        let mut out = Vec::new();
        if let Some(b) = battery.filter(|_| d.takes(BATTERY)) {
            out.push(battery_packet(b));
        }
        if let Some(p) = clip.connect_packet().filter(|_| d.takes(CLIPBOARD_CONNECT)) {
            out.push(p);
        }
        if d.takes(KEYBOARDSTATE) {
            out.push(packet(KEYBOARDSTATE, json!({ "state": true })));
        }
        out
    }

    /// The pairing question's toast for [id] (one at a time per phone).
    const PAIR_TAG: &str = "pair";

    fn pair_toast(id: &str, name: &str, key: &str) -> Toast {
        let answer = |yes: &str| toast::args(&[("do", "pair"), ("dev", id), ("key", key), ("yes", yes)]);
        Toast {
            tag: PAIR_TAG.into(),
            group: toast::short(id),
            title: format!("{name} wants to pair"),
            body: format!("Pair only if the phone shows this same code: {key}"),
            launch: toast::args(&[("do", "open")]),
            buttons: vec![Button { label: "Accept".into(), args: answer("1"), with_reply: false }, Button { label: "Reject".into(), args: answer("0"), with_reply: false }],
            urgent: true,
            ..Default::default()
        }
    }

    /// The toast for a notification from phone [id]: Reply where the phone can take one, its own
    /// buttons (three at most, as on Linux). Pressing them, or closing it, comes back with what to
    /// tell the phone in the arguments, so nothing is kept here.
    fn note_toast(id: &str, phone: &str, b: &Value) -> Option<Toast> {
        let s = |k: &str| b[k].as_str().unwrap_or("").trim().to_owned();
        let (key, app, title, text) = (s("id"), s("appName"), s("title"), s("text"));
        if key.is_empty() || (title.is_empty() && text.is_empty()) {
            return None;
        }
        let rid = s("requestReplyId");
        let mut buttons = Vec::new();
        if !rid.is_empty() {
            buttons.push(Button { label: "Send".into(), args: toast::args(&[("do", "reply"), ("dev", id), ("rid", &rid)]), with_reply: true });
        }
        for action in b["actions"].as_array().into_iter().flatten().filter_map(|a| a.as_str()).take(3) {
            buttons.push(Button { label: action.to_owned(), args: toast::args(&[("do", "action"), ("dev", id), ("key", &key), ("action", action)]), with_reply: false });
        }
        let clear = if b["isClearable"].as_bool() == Some(true) { "1" } else { "0" };
        let to = if title.is_empty() { app.clone() } else { title.clone() };
        Some(Toast {
            tag: toast::short(&key),
            group: toast::short(id),
            title: if title.is_empty() { app.clone() } else { title },
            body: text,
            attribution: if app.is_empty() { phone.to_owned() } else { format!("{app} \u{b7} {phone}") },
            launch: toast::args(&[("do", "note"), ("dev", id), ("key", &key), ("clear", clear)]),
            reply: (!rid.is_empty()).then(|| format!("Reply to {to}")),
            buttons,
            urgent: false,
        })
    }

    /// A notification from the phone: a toast (taken down when the phone dismisses it), or a
    /// balloon where there are no toasts. The backlog the phone sends when asked (`silent`) isn't shown.
    fn phone_notification(id: &str, d: &Device, b: &Value) {
        if b["isCancel"].as_bool() == Some(true) {
            if let Some(key) = b["id"].as_str() {
                toast::remove(&toast::short(key), &toast::short(id));
            }
            return;
        }
        if b["silent"].as_bool() == Some(true) {
            return;
        }
        let Some(t) = note_toast(id, &d.name, b) else { return };
        if toast::show(&t) == Shown::Unavailable {
            tray::notify(&format!("{} ({})", t.title, t.attribution), &t.body);
        }
    }

    /// Handles one packet from a paired device; what to send back.
    #[allow(clippy::too_many_arguments)]
    async fn on_packet(id: &str, d: &mut Device, p: &Value, from: IpAddr, me: &Arc<Identity>, clip: &mut Clipboard, ringer: &mut win::Ringer, activity: &Arc<Activity>) -> Vec<Value> {
        let body = &p["body"];
        match p["type"].as_str().unwrap_or("") {
            PING => tray::notify(&d.name, body["message"].as_str().unwrap_or("Ping!")),
            BATTERY => {
                let charge = body["currentCharge"].as_i64().or_else(|| body["currentCharge"].as_str()?.parse().ok()).unwrap_or(-1);
                let was_low = d.battery.is_some_and(|b| b.2);
                d.battery = (0..=100).contains(&charge).then(|| (charge, body["isCharging"].as_bool().unwrap_or(false), body["thresholdEvent"].as_i64() == Some(1)));
                if let Some((charge, _, true)) = d.battery
                    && !was_low
                {
                    tray::notify(&format!("{}'s battery is low", d.name), &format!("{charge}% left"));
                }
            }
            FIND => {
                ringer.start();
                tray::notify(&format!("{} is looking for this PC", d.name), "Found it? Open Vyoma or wait a minute; it stops by itself.");
            }
            LOCK_REQUEST => {
                if body["setLocked"].as_bool() == Some(true) {
                    match win::lock() {
                        Ok(()) => tracing::info!("{}: locked this PC", d.name),
                        Err(e) => tracing::warn!("{}: {e:#}", d.name),
                    }
                }
            }
            CLIPBOARD | CLIPBOARD_CONNECT => {
                if let Some(text) = clip.received(p)
                    && let Err(e) = win::set_clipboard(&text)
                {
                    tracing::warn!("couldn't set the clipboard: {e:#}");
                }
            }
            SHARE => match share::classify(p) {
                Shared::File { name, .. } => {
                    let (me, cert, phone, packet, activity, id) = (me.clone(), d.cert.clone(), d.name.clone(), p.clone(), activity.clone(), id.to_owned());
                    let from = d.remote.unwrap_or(from);
                    tokio::spawn(async move {
                        match share::receive(&me, &cert, from, &packet, &win::downloads()).await {
                            Ok(path) => {
                                let saved = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(name);
                                activity.add(&id, &phone, "share", format!("Shared \u{201c}{saved}\u{201d} with this PC"));
                                tray::notify(&format!("From {phone}"), &format!("{saved} is in Downloads"));
                            }
                            Err(e) => {
                                tracing::warn!("{name} from {phone}: {e:#}");
                                tray::notify(&format!("{name} didn't arrive"), "The phone stopped sending it. Try again.");
                            }
                        }
                    });
                }
                Shared::Url(url) => {
                    if win::openable(&url) {
                        win::open(&url);
                    } else {
                        // Not a web page: on the clipboard instead, for the person to decide.
                        let _ = win::set_clipboard(&url);
                        tray::notify(&format!("Link from {}", d.name), "Copied. It isn't a web page, so it wasn't opened.");
                    }
                }
                Shared::Text(text) => {
                    if win::set_clipboard(&text).is_ok() {
                        // Ours, so the poll doesn't send it straight back.
                        clip.changed(text);
                        tray::notify(&format!("Text from {}", d.name), "Copied. Paste it anywhere.");
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
        fn a_linked_phone_gets_the_battery_clipboard_and_keyboard_it_takes() {
            let b = Some(win::Battery { charge: 70, charging: true });
            let mut clip = Clipboard::default();
            assert!(started(&phone(&[BATTERY, CLIPBOARD_CONNECT]), None, &clip).is_empty(), "a desktop with nothing copied");
            clip.changed("hello".into());
            let kinds: Vec<_> = started(&phone(&[BATTERY, CLIPBOARD_CONNECT, KEYBOARDSTATE]), b, &clip).iter().map(|p| p["type"].as_str().unwrap().to_owned()).collect();
            assert_eq!(kinds, [BATTERY, CLIPBOARD_CONNECT, KEYBOARDSTATE]);
            assert!(started(&Device { paired: false, ..phone(&[BATTERY]) }, b, &clip).is_empty(), "nothing before pairing");
        }

        #[test]
        fn the_window_reads_a_phone_as_on_linux() {
            let mut d = phone(&[SHARE]);
            (d.name, d.key, d.asking, d.battery) = ("Pixel".into(), "1A2B".into(), true, Some((64, false, false)));
            let permits = Permits::default();
            permits.set("p", Permit::Files, false).unwrap();
            let dir = std::env::temp_dir().join(format!("vyoma-details-{}", std::process::id()));
            let _ = std::fs::create_dir_all(&dir);
            let v = details("p", &d, &permits, &ClipboardSeen::load(&dir), 1);
            assert_eq!(v["clipboardSync"], "plug", "unknown until it's plugged in");
            assert_eq!(v["via"], "", "not linked");
            d.linked = true;
            d.remote = Some("127.0.0.1".parse().unwrap());
            assert_eq!(via(&d), "cable");
            d.remote = Some("192.0.2.7".parse().unwrap());
            assert_eq!(via(&d), "wifi");
            d.linked = false;
            let _ = std::fs::remove_dir_all(&dir);
            assert_eq!((v["id"].as_str(), v["key"].as_str(), v["asking"].as_bool()), (Some("p"), Some("1A2B"), Some(true)));
            assert_eq!((v["can"]["share"].as_bool(), v["can"]["ring"].as_bool()), (Some(true), Some(false)), "only what it takes");
            assert_eq!(v["battery"]["charge"], 64);
            let keys: Vec<(&str, bool)> = v["permits"].as_array().unwrap().iter().map(|p| (p["key"].as_str().unwrap(), p["on"].as_bool().unwrap())).collect();
            assert_eq!(keys, [("screen", true), ("control", true), ("files", false), ("camera", true), ("power", true)]);
            assert_eq!(url_encode("Sam's PC"), "Sam%27s%20PC");
        }

        #[test]
        fn phone_notifications_become_toasts_with_their_buttons() {
            let b = json!({ "id": "k1", "appName": "Chat", "title": "Sam", "text": "Lunch?", "requestReplyId": "r1", "actions": ["Mark read", "Mute", "Archive", "Fourth"], "isClearable": true });
            let t = note_toast("phone1", "Pixel", &b).unwrap();
            assert_eq!((t.title.as_str(), t.body.as_str(), t.attribution.as_str()), ("Sam", "Lunch?", "Chat \u{b7} Pixel"));
            assert_eq!(t.reply.as_deref(), Some("Reply to Sam"));
            let labels: Vec<&str> = t.buttons.iter().map(|b| b.label.as_str()).collect();
            assert_eq!(labels, ["Send", "Mark read", "Mute", "Archive"], "Reply, then three of its own");
            let send = toast::parse_args(&t.buttons[0].args);
            assert_eq!((send["do"].as_str(), send["dev"].as_str(), send["rid"].as_str()), ("reply", "phone1", "r1"));
            let closed = toast::parse_args(&t.launch);
            assert_eq!((closed["do"].as_str(), closed["key"].as_str(), closed["clear"].as_str()), ("note", "k1", "1"));
            assert!(note_toast("p", "P", &json!({ "id": "k", "title": "" })).is_none(), "nothing to say");
            let plain = note_toast("p", "P", &json!({ "id": "k", "appName": "Mail", "text": "New mail" })).unwrap();
            assert_eq!((plain.title.as_str(), plain.reply.is_none(), plain.buttons.len()), ("Mail", true, 0));
            let pair = pair_toast("p", "Pixel", "1A2B");
            assert!(pair.urgent && pair.body.contains("1A2B") && toast::parse_args(&pair.buttons[1].args)["yes"] == "0");
        }

        #[test]
        fn it_says_what_it_does() {
            let dir = std::env::temp_dir().join(format!("vyoma-windows-id-{}", std::process::id()));
            let me = Identity::load_or_create_in(&dir).unwrap();
            let p = identity(&me, "DESKTOP-TEST".into(), false);
            assert_eq!(p["body"]["deviceType"], "desktop");
            let incoming = p["body"]["incomingCapabilities"].as_array().unwrap();
            for offered in ["kdeconnect.mousepad.request", "kdeconnect.share.request", "vyoma.stats.request", "vyoma.screenshot.request", "vyoma.files.list"] {
                assert!(incoming.iter().any(|t| t == offered), "{offered}");
            }
            assert!(incoming.iter().any(|t| t == "vyoma.screen.request"), "the live screen");
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }
}
