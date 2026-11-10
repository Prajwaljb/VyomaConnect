//! Vyoma's desktop window. It only shows and asks: the daemon (`vyoma-desktop`) owns the links,
//! so closing or crashing this never drops a phone. `--snapshot <png>` renders it offscreen with
//! made-up phones instead (`--dark`, `--plain` for no wallpaper, `--scene phones|pairing|nearby|away|unpair|add-done|notice|cable|pc|pc-setup|activity|commands|commands-empty|commands-edit|setup|setup-pairing|setup-search|setup-ready|setup-moment|empty|add|down`, `--setup` to open on setup). `--page commands|this-pc|activity|<phone id>` opens on that page;
//! a window already open shows it instead (`instance.rs`).

mod daemon;
mod instance;
mod look;
mod qr;
mod snapshot;

use std::cell::RefCell;
use std::sync::mpsc;
use std::time::Duration;

use daemon::{ActivityEntry, CommandData, Daemon, DbusDaemon, Event, Health, PhoneData};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

slint::include_modules!();

/// How long a notice stays, and how often the window rereads the daemon without being told to
/// (a battery that moved without a change signal, a daemon that restarted).
const NOTICE_FOR: Duration = Duration::from_millis(3200);
const REREAD_EVERY: Duration = Duration::from_secs(5);
const RECONNECT_EVERY: Duration = Duration::from_secs(2);
/// How long "You're connected" stays.
const MOMENT_FOR: Duration = Duration::from_millis(2600);
/// How long the code shows its tick before the moment: the tick lands at about 0.8 s.
const CODE_DONE_FOR: Duration = Duration::from_millis(1300);
/// How often the wallpaper's colours are reread (a new wallpaper says nothing).
const WALLPAPER_EVERY: Duration = Duration::from_secs(60);
/// The sidebar's id for This PC's page.
pub const THIS_PC: &str = "this-pc";
/// And the activity's.
pub const ACTIVITY: &str = "activity";
/// And the commands'.
pub const COMMANDS: &str = "commands";
/// The Add a phone page, with the pairing code.
pub const ADD: &str = "add";

/// Ready-made commands for the Commands page: only what the phone can't already do itself (it
/// locks, mutes, sleeps and shuts the PC down on its own), and only what this PC has the tools for.
/// (name, command line, ask for fingerprint).
fn suggestions() -> &'static [(&'static str, String, bool)] {
    static LIST: std::sync::OnceLock<Vec<(&'static str, String, bool)>> = std::sync::OnceLock::new();
    LIST.get_or_init(|| {
        let has = |tool: &str| std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(tool).is_file()));
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase();
        let mut out = Vec::new();
        if desktop.contains("kde") && has("kscreen-doctor") {
            out.push(("Turn the screens off", "kscreen-doctor --dpms off".to_owned(), false));
        } else if desktop.contains("gnome") && has("busctl") {
            out.push(("Turn the screens off", "busctl --user call org.gnome.Shell /org/gnome/ScreenSaver org.gnome.ScreenSaver SetActive b true".to_owned(), false));
        }
        if has("powerprofilesctl") {
            out.push(("Power saver", "powerprofilesctl set power-saver".to_owned(), false));
            out.push(("Balanced power", "powerprofilesctl set balanced".to_owned(), false));
            out.push(("Full performance", "powerprofilesctl set performance".to_owned(), false));
        }
        if has("gio") {
            out.push(("Empty the trash", "gio trash --empty".to_owned(), true));
        }
        if desktop.contains("kde") && has("systemctl") {
            out.push(("Restart the panel and desktop", "systemctl --user restart plasma-plasmashell".to_owned(), false));
        }
        out
    })
}
thread_local! {
    /// The devices last read, on the UI thread.
    static SHOWN: RefCell<Vec<PhoneData>> = const { RefCell::new(Vec::new()) };
    static NOTICES: RefCell<u64> = const { RefCell::new(0) };
    /// The commands last read, for the editor's checks.
    static COMMANDS_SHOWN: RefCell<Vec<CommandData>> = const { RefCell::new(Vec::new()) };
    /// What the status bubble's button does while it shows.
    static NOTICE_ACTION: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if let Some(at) = args.iter().position(|a| a == "--snapshot") {
        let path = args.get(at + 1).ok_or_else(|| anyhow::anyhow!("--snapshot needs a path"))?;
        let scene = args.iter().position(|a| a == "--scene").and_then(|i| args.get(i + 1)).map(String::as_str).unwrap_or("phones");
        let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1));
        let size = value("--size").and_then(|s| s.split_once('x')).and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)));
        let scroll = value("--scroll").and_then(|s| s.parse().ok()).unwrap_or(0.0);
        return snapshot::run(path, args.iter().any(|a| a == "--dark"), args.iter().any(|a| a == "--plain"), scene, size, scroll);
    }
    let page = args.iter().position(|a| a == "--page").and_then(|i| args.get(i + 1)).cloned().unwrap_or_default();
    // The app id the desktop matches to vyoma.desktop: Vyoma's name and icon on the taskbar.
    let _ = slint::set_xdg_app_id("vyoma");
    let ui = MainWindow::new()?;
    // Another window is open: it shows the page, this one ends. No bus at all: run anyway.
    if !instance::claim(&ui, &page).unwrap_or(true) {
        return Ok(());
    }
    ui.set_setup(args.iter().any(|a| a == "--setup") || !setup_done());
    wire(&ui);
    qr::wire(&ui);
    watch(ui.as_weak());
    if !page.is_empty() {
        open_page(&ui, &page);
    }
    ui.run()?;
    Ok(())
}

/// Shows [page] (see `instance.rs`), leaving setup if it's up: a page was asked for by name.
pub fn open_page(ui: &MainWindow, page: &str) {
    if page.is_empty() {
        return;
    }
    ui.set_setup(false);
    match page {
        COMMANDS => ui.invoke_show_commands(),
        THIS_PC => ui.invoke_show_pc(),
        ACTIVITY => ui.invoke_show_activity(),
        id => ui.invoke_select(id.into()),
    }
}

fn read_commands(ui: &slint::Weak<MainWindow>) {
    let ui = ui.clone();
    std::thread::spawn(move || {
        let list = DbusDaemon::connect().and_then(|d| d.commands());
        let _ = slint::invoke_from_event_loop(move || {
            if let (Some(win), Ok(list)) = (ui.upgrade(), list) {
                show_commands(&win, &list);
            }
        });
    });
}

/// The commands, and the suggestions whose command line isn't among them yet.
pub fn show_commands(ui: &MainWindow, list: &[CommandData]) {
    let to = |c: &CommandData| Command { key: c.key.clone().into(), name: c.name.clone().into(), command: c.command.clone().into(), shown: breakable(&c.command).into(), fingerprint: c.fingerprint, every: c.every_time };
    let suggestions: Vec<Command> = suggestions()
        .iter()
        .filter(|(_, line, _)| !list.iter().any(|c| c.command.trim() == line.as_str()))
        .map(|(name, line, fingerprint)| Command { key: "".into(), name: (*name).into(), command: line.as_str().into(), shown: breakable(line).into(), fingerprint: *fingerprint, every: false })
        .collect();
    ui.set_commands(ModelRc::new(VecModel::from(list.iter().map(to).collect::<Vec<_>>())));
    COMMANDS_SHOWN.with(|c| *c.borrow_mut() = list.to_vec());
    ui.set_suggestions(ModelRc::new(VecModel::from(suggestions)));
}

/// Where the window keeps what's its own (that setup's been through), beside the daemon's state.
fn window_file() -> Option<std::path::PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))?;
    Some(config.join("vyoma-desktop").join("window.json"))
}

fn setup_done() -> bool {
    window_file().and_then(|f| std::fs::read_to_string(f).ok()).and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()).is_some_and(|v| v["setupDone"] == true)
}

fn mark_setup_done() {
    let Some(file) = window_file() else { return };
    let _ = file.parent().map(std::fs::create_dir_all);
    let _ = std::fs::write(file, serde_json::json!({ "setupDone": true }).to_string());
}

/// The window's buttons: each call runs off the UI thread, and a failure shows as a notice.
fn wire(ui: &MainWindow) {
    ui.on_finish_setup({
        let ui = ui.as_weak();
        move || {
            mark_setup_done();
            if let Some(win) = ui.upgrade() {
                win.set_setup(false);
                read_health(&ui);
            }
        }
    });
    ui.on_select({
        let ui = ui.as_weak();
        move |id| {
            if let Some(ui) = ui.upgrade() {
                ui.set_selected(id);
                SHOWN.with(|s| show(&ui, &s.borrow()));
            }
        }
    });
    let act = |ui: &MainWindow, method: &'static str, done: Option<&'static str>| {
        let ui = ui.as_weak();
        move |id: SharedString| {
            let ui = ui.clone();
            std::thread::spawn(move || {
                let result = DbusDaemon::connect().and_then(|d| d.act(method, &id));
                match (result, done) {
                    (Err(e), _) => notify(&ui, format!("{e:#}")),
                    (Ok(()), Some(text)) => notify(&ui, text.to_owned()),
                    (Ok(()), None) => {}
                }
            });
        }
    };
    ui.on_pair(act(ui, "Pair", None));
    ui.on_accept(act(ui, "Accept", None));
    ui.on_reject(act(ui, "Reject", None));
    ui.on_unpair(act(ui, "Unpair", Some("Unpaired")));
    ui.on_ring(act(ui, "Ring", Some("Ringing the phone")));
    ui.on_browse(act(ui, "Browse", Some("Opening the phone's files…")));
    ui.on_show_pc({
        let ui = ui.as_weak();
        move || {
            if let Some(win) = ui.upgrade() {
                win.set_selected(THIS_PC.into());
            }
            read_health(&ui);
        }
    });
    ui.on_show_activity({
        let ui = ui.as_weak();
        move || {
            if let Some(win) = ui.upgrade() {
                win.set_confirm_clear(false);
                win.set_selected(ACTIVITY.into());
            }
            read_activity(&ui);
        }
    });
    ui.on_clear_activity({
        let ui = ui.as_weak();
        move || {
            let ui = ui.clone();
            std::thread::spawn(move || {
                if let Err(e) = DbusDaemon::connect().and_then(|d| d.clear_activity()) {
                    notify(&ui, format!("{e:#}"));
                }
            });
        }
    });
    ui.on_update({
        let ui = ui.as_weak();
        move || {
            if let Some(win) = ui.upgrade() {
                win.set_updating(true);
            }
            let ui = ui.clone();
            std::thread::spawn(move || match DbusDaemon::connect().and_then(|d| d.update()) {
                Ok(()) => notify(&ui, "Updating: Vyoma restarts by itself when it's done".to_owned()),
                Err(e) => {
                    notify(&ui, format!("{e:#}"));
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(win) = ui.upgrade() {
                            win.set_updating(false);
                        }
                    });
                }
            });
        }
    });
    ui.on_update_checks_toggled({
        let ui = ui.as_weak();
        move |on| {
            if let Some(win) = ui.upgrade() {
                win.set_update_checks(on);
            }
            let ui = ui.clone();
            std::thread::spawn(move || {
                if let Err(e) = DbusDaemon::connect().and_then(|d| d.set_update_checks(on)) {
                    notify(&ui, format!("{e:#}"));
                }
                read_health(&ui);
            });
        }
    });
    ui.on_open_firewall({
        let ui = ui.as_weak();
        move || {
            let ui = ui.clone();
            std::thread::spawn(move || {
                if let Err(e) = DbusDaemon::connect().and_then(|d| d.open_firewall()) {
                    notify(&ui, format!("{e:#}"));
                }
            });
        }
    });
    ui.on_install({
        let ui = ui.as_weak();
        move || {
            if let Some(win) = ui.upgrade() {
                win.set_installing(true);
            }
            let ui = ui.clone();
            std::thread::spawn(move || {
                if let Err(e) = DbusDaemon::connect().and_then(|d| d.install()) {
                    notify(&ui, format!("{e:#}"));
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(win) = ui.upgrade() {
                            win.set_installing(false);
                        }
                    });
                }
            });
        }
    });
    ui.on_set_permission({
        let ui = ui.as_weak();
        move |id, key, on| {
            let ui = ui.clone();
            std::thread::spawn(move || {
                if let Err(e) = DbusDaemon::connect().and_then(|d| d.set_permission(&id, &key, on)) {
                    notify(&ui, format!("{e:#}"));
                }
            });
        }
    });
    ui.on_show_commands({
        let ui = ui.as_weak();
        move || {
            if let Some(win) = ui.upgrade() {
                win.set_selected(COMMANDS.into());
            }
            read_commands(&ui);
        }
    });
    ui.on_save_command({
        let ui = ui.as_weak();
        move |key, name, command, fingerprint, every| {
            let ui = ui.clone();
            std::thread::spawn(move || match DbusDaemon::connect().and_then(|d| d.save_command(&key, &name, &command, fingerprint, every)) {
                Ok(_) => {
                    notify(&ui, if key.is_empty() { format!("Added \u{201c}{}\u{201d}: it's on your phones now", name.trim()) } else { "Saved".into() });
                    read_commands(&ui);
                }
                Err(e) => notify(&ui, format!("{e:#}")),
            });
        }
    });
    ui.on_remove_command({
        let ui = ui.as_weak();
        move |key| {
            let ui = ui.clone();
            let gone = COMMANDS_SHOWN.with(|c| c.borrow().iter().find(|c| c.key == key.as_str()).cloned());
            std::thread::spawn(move || match DbusDaemon::connect().and_then(|d| d.remove_command(&key)) {
                Ok(()) => {
                    read_commands(&ui);
                    let Some(gone) = gone else { return notify(&ui, "Deleted".into()) };
                    // Undo puts it back as it was (under a new key: phones just see it listed again).
                    let again = ui.clone();
                    notify_with(&ui, format!("Deleted \u{201c}{}\u{201d}", gone.name), "Undo", move || {
                        std::thread::spawn(move || match DbusDaemon::connect().and_then(|d| d.save_command("", &gone.name, &gone.command, gone.fingerprint, gone.every_time)) {
                            Ok(_) => {
                                notify(&again, format!("\u{201c}{}\u{201d} is back", gone.name));
                                read_commands(&again);
                            }
                            Err(e) => notify(&again, format!("{e:#}")),
                        });
                    });
                }
                Err(e) => notify(&ui, format!("{e:#}")),
            });
        }
    });
    ui.on_check_command(|key, name, command| COMMANDS_SHOWN.with(|c| command_warning(&key, &name, &command, &c.borrow(), &|p| on_path(p))).into());
    ui.on_notice_acted({
        let ui = ui.as_weak();
        move || {
            if let Some(win) = ui.upgrade() {
                win.set_notice_on(false);
            }
            if let Some(act) = NOTICE_ACTION.with(|a| a.borrow_mut().take()) {
                act();
            }
        }
    });
    ui.on_allow_clipboard({
        let ui = ui.as_weak();
        move |id| {
            let ui = ui.clone();
            std::thread::spawn(move || match DbusDaemon::connect().and_then(|d| d.allow_clipboard(&id)) {
                Ok(()) => notify(&ui, "Clipboard sync is on".into()),
                Err(e) => notify(&ui, format!("{e:#}")),
            });
        }
    });
    // Up and Down move through the sidebar, in its order.
    ui.on_step({
        let ui = ui.as_weak();
        move |by| {
            let Some(win) = ui.upgrade() else { return };
            let ids = |m: ModelRc<Phone>| (0..m.row_count()).filter_map(|i| m.row_data(i)).map(|p| p.id.to_string()).collect::<Vec<_>>();
            let mut order = ids(win.get_phones());
            order.extend(ids(win.get_nearby()));
            if win.get_daemon_ok() {
                order.push(THIS_PC.into());
                order.push(ACTIVITY.into());
                order.push(COMMANDS.into());
            }
            let at = order.iter().position(|id| *id == win.get_selected().as_str());
            let next = match at {
                Some(i) => (i as i32 + by).clamp(0, order.len() as i32 - 1) as usize,
                None if !order.is_empty() => 0,
                None => return,
            };
            match order[next].as_str() {
                THIS_PC => win.invoke_show_pc(),
                ACTIVITY => win.invoke_show_activity(),
                COMMANDS => win.invoke_show_commands(),
                id => win.invoke_select(id.into()),
            }
        }
    });
    // The daemon stopped (crashed past its restarts, or was stopped): start its unit again; the
    // window reconnects by itself once it's up.
    ui.on_start_daemon({
        let ui = ui.as_weak();
        move || {
            let ui = ui.clone();
            std::thread::spawn(move || {
                let started = std::process::Command::new("systemctl").args(["--user", "start", "vyoma-desktop"]).status();
                match started {
                    Ok(s) if s.success() => notify(&ui, "Starting Vyoma\u{2026}".into()),
                    _ => notify(&ui, "Vyoma couldn't start. Run vyoma-desktop in a terminal to see why".into()),
                }
            });
        }
    });
    ui.on_search({
        let ui = ui.as_weak();
        move || {
            let ui = ui.clone();
            std::thread::spawn(move || match DbusDaemon::connect().and_then(|d| d.search()) {
                Ok(()) => notify(&ui, "Looking for phones nearby".into()),
                Err(e) => notify(&ui, format!("{e:#}")),
            });
        }
    });
    ui.on_send_files({
        let ui = ui.as_weak();
        move |id| {
            let ui = ui.clone();
            std::thread::spawn(move || {
                let Some(files) = rfd::FileDialog::new().set_title("Send to your phone").pick_files() else { return };
                let paths: Vec<String> = files.iter().map(|p| p.to_string_lossy().into_owned()).collect();
                let n = paths.len();
                // "Sent" follows once the phone has them (the daemon's FilesSent).
                match DbusDaemon::connect().and_then(|d| d.send_files(&id, paths)) {
                    Ok(()) => notify(&ui, if n == 1 { "Sending 1 file…".into() } else { format!("Sending {n} files…") }),
                    Err(e) => notify(&ui, format!("{e:#}")),
                }
            });
        }
    });
}

/// Reads the daemon now, whenever it says something changed, and every [REREAD_EVERY]; shows
/// "isn't running" while it's away and picks up again when it's back.
fn watch(ui: slint::Weak<MainWindow>) {
    let (changed, changes) = mpsc::channel::<()>();
    let told = ui.clone();
    std::thread::spawn(move || {
        loop {
            if let Ok(d) = DbusDaemon::connect() {
                while let Ok(event) = d.next_event() {
                    match event {
                        Event::Changed => {
                            read_commands(&told);
                            if changed.send(()).is_err() {
                                return;
                            }
                        }
                        Event::Sent { id, count, error } => sent(&told, id, count, error),
                        Event::Activity => read_activity(&told),
                        Event::SetupDone { ok } => {
                            notify(&told, if ok { "Installed. This PC is set up".into() } else { "The install didn't finish".into() });
                            read_health(&told);
                        }
                        Event::FirewallDone { ok } => {
                            notify(&told, if ok { "Ports open. Phones on this Wi-Fi can reach this PC".into() } else { "The ports weren't opened".into() });
                            read_health(&told);
                        }
                    }
                }
            }
            std::thread::sleep(RECONNECT_EVERY);
        }
    });
    std::thread::spawn(move || {
        let mut daemon: Option<DbusDaemon> = None;
        let mut wall_read: Option<std::time::Instant> = None;
        loop {
            if daemon.is_none() {
                daemon = DbusDaemon::connect().ok();
                // Back (or there at start): This PC's state, for the sidebar's dot.
                if daemon.is_some() {
                    read_health(&ui);
                    read_activity(&ui);
                    read_commands(&ui);
                    wall_read = None;
                }
            }
            if let Some(d) = &daemon {
                if wall_read.is_none_or(|t| t.elapsed() >= WALLPAPER_EVERY) {
                    wall_read = Some(std::time::Instant::now());
                    // An older daemon, or no wallpaper to read: the accent's glow instead.
                    let wall = d.wallpaper().ok();
                    let ui2 = ui.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui2.upgrade() {
                            show_wallpaper(&ui, wall);
                        }
                    });
                }
            }
            let phones = daemon.as_ref().map(|d| d.phones());
            let phones = match phones {
                Some(Ok(p)) => Some(p),
                _ => {
                    daemon = None;
                    None
                }
            };
            let ui2 = ui.clone();
            if slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui2.upgrade() {
                    apply(&ui, phones);
                }
            })
            .is_err()
            {
                return;
            }
            let wait = if daemon.is_some() { REREAD_EVERY } else { RECONNECT_EVERY };
            let _ = changes.recv_timeout(wait);
            // A burst of changes is one reread.
            while changes.try_recv().is_ok() {}
        }
    });
}

/// The accent and canvas from [wall], redrawn only when it changed.
fn show_wallpaper(ui: &MainWindow, wall: Option<look::Wallpaper>) {
    thread_local! { static LAST: RefCell<Option<Option<look::Wallpaper>>> = const { RefCell::new(None) }; }
    if LAST.with(|l| l.borrow().as_ref() == Some(&wall)) {
        return;
    }
    look::apply(&ui.global::<Theme>(), wall.as_ref());
    LAST.with(|l| *l.borrow_mut() = Some(wall));
}

/// Reads the activity off the UI thread, then shows it.
fn read_activity(ui: &slint::Weak<MainWindow>) {
    let ui = ui.clone();
    std::thread::spawn(move || {
        let list = DbusDaemon::connect().and_then(|d| d.activity());
        let _ = slint::invoke_from_event_loop(move || {
            if let (Some(win), Ok(list)) = (ui.upgrade(), list) {
                show_activity(&win, &list, chrono::Local::now());
            }
        });
    });
}

/// [list] (newest first) by day, as of [now]; the sidebar's line says how much today.
pub fn show_activity(ui: &MainWindow, list: &[ActivityEntry], now: chrono::DateTime<chrono::Local>) {
    use chrono::{Datelike, TimeZone};
    let today = now.date_naive();
    let mut days: Vec<(chrono::NaiveDate, Vec<ActivityEntry>)> = Vec::new();
    let mut stamps = Vec::new();
    for h in list {
        let Some(at) = chrono::Local.timestamp_opt(h.time, 0).single() else { continue };
        let day = at.date_naive();
        match days.last_mut() {
            Some((d, items)) if *d == day => items.push(h.clone()),
            _ => days.push((day, vec![h.clone()])),
        }
        stamps.push(at);
    }
    let mut at = stamps.into_iter();
    let label = |d: chrono::NaiveDate| {
        let ago = (today - d).num_days();
        match ago {
            0 => "Today".to_owned(),
            1 => "Yesterday".to_owned(),
            2..=6 => d.format("%A").to_string(),
            _ if d.year() == today.year() => d.format("%-d %B").to_string(),
            _ => d.format("%-d %B %Y").to_string(),
        }
    };
    let model: Vec<Day> = days
        .iter()
        .map(|(d, items)| Day {
            label: label(*d).into(),
            items: ModelRc::new(VecModel::from(
                items
                    .iter()
                    .map(|h| {
                        let time = at.next().map(|t| t.format("%H:%M").to_string()).unwrap_or_default();
                        Happening { kind: h.kind.clone().into(), text: breakable(&h.text).into(), meta: breakable(&format!("{} \u{b7} {time}", h.name)).into() }
                    })
                    .collect::<Vec<_>>(),
            )),
        })
        .collect();
    let today_count = days.first().filter(|(d, _)| *d == today).map_or(0, |(_, i)| i.len());
    ui.set_activity_summary(
        match (today_count, list.is_empty()) {
            (0, true) => "Nothing yet".to_owned(),
            (0, false) => "Nothing today".to_owned(),
            (n, _) => format!("{n} today"),
        }
        .into(),
    );
    ui.set_days(ModelRc::new(VecModel::from(model)));
    ui.set_activity_loaded(true);
}

/// Reads This PC's setup off the UI thread, then shows it.
fn read_health(ui: &slint::Weak<MainWindow>) {
    let ui = ui.clone();
    std::thread::spawn(move || {
        let health = DbusDaemon::connect().and_then(|d| d.health());
        let _ = slint::invoke_from_event_loop(move || {
            if let (Some(win), Ok(h)) = (ui.upgrade(), health) {
                show_health(&win, &h);
            }
        });
    });
}

pub fn show_health(ui: &MainWindow, h: &Health) {
    // What needs something first, where it's seen without scrolling.
    let mut order: Vec<&daemon::Feature> = h.features.iter().collect();
    order.sort_by_key(|f| f.ready);
    let features: Vec<Feature> = order
        .into_iter()
        .map(|f| Feature {
            name: f.name.clone().into(),
            ready: f.ready,
            detail: if f.ready {
                "Ready".into()
            } else if f.needs.is_empty() {
                "Not available here".into()
            } else {
                format!("Needs {}", f.needs.join(", ")).into()
            },
        })
        .collect();
    let checks: Vec<Check> = h.checks.iter().map(|c| Check { name: c.name.clone().into(), state: c.state.clone().into(), detail: c.detail.clone().into(), action: c.action.clone().into() }).collect();
    // A firewall that may keep phones out: an away phone's page offers the same fix.
    ui.set_firewall_fix(h.checks.iter().any(|c| c.name == "Firewall" && !c.action.is_empty()));
    let notes: Vec<SharedString> = h.notes.iter().map(|n| n.clone().into()).collect();
    // Setup lists only what an install fixes.
    let missing: Vec<Feature> = features.iter().filter(|f| !f.ready && f.detail.starts_with("Needs")).cloned().collect();
    ui.set_missing(ModelRc::new(VecModel::from(missing)));
    ui.set_features(ModelRc::new(VecModel::from(features)));
    ui.set_checks(ModelRc::new(VecModel::from(checks)));
    ui.set_notes(ModelRc::new(VecModel::from(notes)));
    ui.set_installable(h.installable);
    ui.set_installing(h.installing);
    ui.set_attention(h.attention() as i32);
    let u = &h.update;
    ui.set_version(u.current.clone().into());
    ui.set_update_latest(u.latest.clone().unwrap_or_default().into());
    ui.set_update_checks(u.checks);
    ui.set_updating(u.updating);
    ui.set_can_update(u.can_update);
    ui.set_update_packaged(u.packaged);
    ui.set_health_loaded(true);
}

/// A send is over: "Sent", or why not, naming the phone.
fn sent(ui: &slint::Weak<MainWindow>, id: String, count: u32, error: String) {
    let ui2 = ui.clone();
    let _ = slint::invoke_from_event_loop(move || {
        let name = SHOWN.with(|s| s.borrow().iter().find(|p| p.id == id).map(|p| p.name.clone())).unwrap_or_else(|| "the phone".into());
        let text = match (error.is_empty(), count) {
            (false, _) => format!("Couldn't send to {name}: {error}"),
            (true, 1) => format!("Sent to {name}"),
            (true, n) => format!("Sent {n} files to {name}"),
        };
        notify(&ui2, text);
    });
}

/// New devices from the daemon (None: it's not running).
fn apply(ui: &MainWindow, phones: Option<Vec<PhoneData>>) {
    ui.set_daemon_ok(phones.is_some());
    let phones = phones.unwrap_or_default();
    let before = SHOWN.with(|s| s.replace(phones.clone()));
    // A phone that just asked to pair comes to the front.
    let asking = phones.iter().find(|p| p.asking && !p.code.is_empty() && !before.iter().any(|b| b.id == p.id && b.asking && !b.code.is_empty()));
    if let Some(p) = asking {
        ui.set_selected(p.id.clone().into());
    }
    // Paired just now (its code scanned, or accepted here): its moment, wherever the window is.
    let fresh = phones.iter().find(|p| p.paired && before.iter().any(|b| b.id == p.id && !b.paired));
    if let Some(p) = fresh {
        // With the code on screen (Add a phone, or setup before any phone) it's what was scanned:
        // the code turns into a tick first, then the moment, then (from Add) that phone's page.
        let code_up = ui.get_selected() == ADD || (ui.get_setup() && !before.iter().any(|b| b.paired));
        if code_up {
            ui.set_pair_code_done(p.name.clone().into());
            let (weak, name, id) = (ui.as_weak(), p.name.clone(), p.id.clone());
            slint::Timer::single_shot(CODE_DONE_FOR, move || {
                let Some(win) = weak.upgrade() else { return };
                moment(&win, name);
                if win.get_selected() == ADD {
                    win.set_selected(id.into());
                    SHOWN.with(|s| show(&win, &s.borrow()));
                }
                // Back to a code while the moment still covers the window.
                let weak = win.as_weak();
                slint::Timer::single_shot(MOMENT_FOR - Duration::from_millis(400), move || {
                    if let Some(win) = weak.upgrade() {
                        win.set_pair_code_done("".into());
                    }
                });
            });
        } else {
            moment(ui, p.name.clone());
        }
    }
    show(ui, &phones);
}

/// "You're connected" with [name], for a moment.
pub fn moment(ui: &MainWindow, name: String) {
    ui.set_moment(name.into());
    ui.set_moment_on(true);
    let ui = ui.as_weak();
    slint::Timer::single_shot(MOMENT_FOR, move || {
        if let Some(win) = ui.upgrade() {
            win.set_moment_on(false);
        }
    });
}

/// The sidebar's two lists and the chosen phone's page, keeping the choice while it's there.
fn show(ui: &MainWindow, phones: &[PhoneData]) {
    let (mut paired, mut nearby): (Vec<&PhoneData>, Vec<&PhoneData>) = phones.iter().partition(|p| p.paired);
    paired.sort_by_key(|p| (!p.linked, p.name.to_lowercase()));
    nearby.retain(|p| p.linked || !p.code.is_empty());
    nearby.sort_by_key(|p| p.name.to_lowercase());
    let selected = ui.get_selected().to_string();
    // A nearby phone only takes the page when it asks to pair: one that has just linked (its owner
    // scanning the code, say) must not replace the page showing the code.
    let current = paired.iter().chain(&nearby).find(|p| p.id == selected).or(paired.first()).or(nearby.iter().find(|p| !p.code.is_empty())).copied();
    ui.set_phones(ModelRc::new(VecModel::from(paired.iter().map(|p| to_ui(p)).collect::<Vec<_>>())));
    ui.set_nearby(ModelRc::new(VecModel::from(nearby.iter().map(|p| to_ui(p)).collect::<Vec<_>>())));
    // This PC's page, the activity, commands and Add a phone stay put however the phones change: the
    // Add page leaving took its code down while a phone that had just linked was about to use it.
    if selected == THIS_PC || selected == ACTIVITY || selected == COMMANDS || selected == ADD {
        return;
    }
    ui.set_has_current(current.is_some());
    if let Some(p) = current {
        if p.id != selected {
            ui.set_confirm_unpair(false);
        }
        ui.set_selected(p.id.clone().into());
        ui.set_current(to_ui(p));
    }
}

fn to_ui(p: &PhoneData) -> Phone {
    let up = p.since.map(|at| connected_for(chrono::Local::now().timestamp_millis() - at)).unwrap_or_default();
    Phone {
        id: p.id.clone().into(),
        name: breakable(&p.name).into(),
        paired: p.paired,
        linked: p.linked,
        code: p.code.clone().into(),
        asking: p.asking,
        via: p.via.clone().into(),
        battery: p.battery.map_or(-1, |b| b as i32),
        charging: p.charging,
        can_ring: p.can_ring,
        can_browse: p.can_browse,
        can_share: p.can_share,
        clip: p.clip.clone().into(),
        rtt: p.rtt.map_or(-1, |ms| ms.clamp(0, 99_999) as i32),
        up_value: up.0.into(),
        up_unit: up.1.into(),
        up_value2: up.2.into(),
        up_unit2: up.3.into(),
        permits: std::rc::Rc::new(slint::VecModel::from(
            p.permits.iter().map(|x| Permission { key: x.key.clone().into(), name: x.name.clone().into(), detail: x.detail.clone().into(), on: x.on }).collect::<Vec<_>>(),
        ))
        .into(),
    }
}

/// Shows [text] in the bubble at the top of the window for a moment.
fn notify(ui: &slint::Weak<MainWindow>, text: String) {
    show_notice(ui, text, "", None);
}

/// A notice with a button ([label], e.g. Undo) that runs [act] on the UI thread; it stays longer,
/// so there's time to reach it.
fn notify_with(ui: &slint::Weak<MainWindow>, text: String, label: &'static str, act: impl FnOnce() + Send + 'static) {
    show_notice(ui, text, label, Some(Box::new(act)));
}

fn show_notice(ui: &slint::Weak<MainWindow>, text: String, label: &'static str, act: Option<Box<dyn FnOnce() + Send>>) {
    let ui = ui.clone();
    let _ = slint::invoke_from_event_loop(move || {
        let Some(win) = ui.upgrade() else { return };
        let n = NOTICES.with(|c| {
            *c.borrow_mut() += 1;
            *c.borrow()
        });
        let lasts = if act.is_some() { NOTICE_FOR * 2 } else { NOTICE_FOR };
        NOTICE_ACTION.with(|a| *a.borrow_mut() = act.map(|f| f as Box<dyn FnOnce()>));
        win.set_notice(text.into());
        win.set_notice_action(label.into());
        win.set_notice_on(true);
        // The words stay while the bubble rises away.
        slint::Timer::single_shot(lasts, move || {
            if NOTICES.with(|c| *c.borrow()) == n {
                if let Some(win) = ui.upgrade() {
                    win.set_notice_on(false);
                }
            }
        });
    });
}

/// [text] with invisible break points in its long words (file names, paths, command lines), so it
/// wraps inside its row: a word that can't break pushed the page wider than the window and hid its
/// end. Display only: what's edited or saved keeps the plain text.
pub fn breakable(text: &str) -> String {
    const LONG: usize = 18;
    let mut out = String::with_capacity(text.len() + 8);
    for (i, word) in text.split(' ').enumerate() {
        if i > 0 {
            out.push(' ');
        }
        if word.chars().count() <= LONG {
            out.push_str(word);
            continue;
        }
        let mut run = 0;
        for c in word.chars() {
            out.push(c);
            run += 1;
            if matches!(c, '_' | '-' | '.' | '/' | '\\' | ':' | ',' | '=' | '&' | '?') || run >= LONG {
                out.push('\u{200b}');
                run = 0;
            }
        }
    }
    out
}

#[cfg(test)]
mod breakable_tests {
    #[test]
    fn long_words_get_break_points_and_short_ones_stay() {
        assert_eq!(super::breakable("Took a screenshot"), "Took a screenshot");
        let b = super::breakable("Copied \u{201c}Screen_Recording_20261108.mp4\u{201d}");
        assert_eq!(b.replace('\u{200b}', ""), "Copied \u{201c}Screen_Recording_20261108.mp4\u{201d}");
        assert!(b.split(|c: char| c == ' ' || c == '\u{200b}').all(|w| w.chars().count() <= 18));
    }
}

/// How long a phone's been connected, as the card shows it: up to two number-and-unit pairs.
pub fn connected_for(ms: i64) -> (String, &'static str, String, &'static str) {
    let mins = ms.max(0) / 60_000;
    let (h, d) = (mins / 60, mins / (60 * 24));
    let pair = |n: i64, u| if n > 0 { (n.to_string(), u) } else { (String::new(), "") };
    if mins < 1 {
        ("<1".into(), "min", String::new(), "")
    } else if h < 1 {
        (mins.to_string(), "min", String::new(), "")
    } else if d < 1 {
        let (v, u) = pair(mins % 60, "min");
        (h.to_string(), "h", v, u)
    } else {
        let (v, u) = pair(h % 24, "h");
        (d.to_string(), "d", v, u)
    }
}

#[cfg(test)]
mod connected_tests {
    use super::connected_for;
    #[test]
    fn reads_like_a_person_would_say_it() {
        assert_eq!(connected_for(20_000), ("<1".into(), "min", String::new(), ""));
        assert_eq!(connected_for(14 * 60_000), ("14".into(), "min", String::new(), ""));
        assert_eq!(connected_for((2 * 60 + 14) * 60_000), ("2".into(), "h", "14".into(), "min"));
        assert_eq!(connected_for(2 * 3_600_000), ("2".into(), "h", String::new(), ""));
        assert_eq!(connected_for((3 * 24 + 5) * 3_600_000), ("3".into(), "d", "5".into(), "h"));
    }
}

/// Whether [program] (no slash) is a file this PC can run, on PATH.
fn on_path(program: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let runs = |p: std::path::PathBuf| p.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
    if program.contains('/') {
        let path = match program.strip_prefix("~/") {
            Some(rest) => std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(rest)),
            None => Some(program.into()),
        };
        return path.is_some_and(runs);
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    // The daemon runs as a user service, which also looks in ~/.local/bin.
    let local = std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/bin"));
    std::env::split_paths(&path).chain(local).any(|dir| runs(dir.join(program)))
}

/// The program a command line starts with, as sh would read it: past `VAR=value`s, unquoted.
/// None when it starts with shell syntax (a subshell, `if`, a builtin) that has no program to find.
fn first_program(command: &str) -> Option<String> {
    let mut words = Vec::new();
    let (mut word, mut quote, mut chars) = (String::new(), None::<char>, command.trim().chars());
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => word.push(c),
            (None, '\'' | '"') => quote = Some(c),
            (None, '\\') => word.extend(chars.next()),
            (None, c) if c.is_whitespace() || ";|&<>()".contains(c) => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
                if !c.is_whitespace() {
                    words.push(c.to_string());
                }
            }
            (None, c) => word.push(c),
        }
        if words.iter().any(|w| !w.contains('=') || w.starts_with('=')) {
            break;
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    let first = words.into_iter().find(|w| !(w.contains('=') && !w.starts_with('=')))?;
    const SHELL: &[&str] = &[
        "(", "{", "!", "if", "for", "while", "until", "case", "cd", "echo", "printf", "test", "[", "export", "exec", "eval",
        "source", ".", "true", "false", "kill", "read", "set", "unset", "wait", "exit", "command", "type", "ulimit", "umask", "trap",
    ];
    (!SHELL.contains(&first.as_str()) && !first.starts_with('$') && !first.starts_with('`')).then_some(first)
}

/// What looks like a mistake in a command being edited, in words; "" if nothing does.
pub fn command_warning(key: &str, name: &str, command: &str, others: &[CommandData], runs: &dyn Fn(&str) -> bool) -> String {
    let program = first_program(command);
    match program.as_deref() {
        Some("sudo" | "doas") => {
            return "Commands run without a terminal, so sudo can't ask for your password. Use pkexec, which asks in a window, or a command that doesn't need it.".into()
        }
        Some(p) if !runs(p) => {
            return if p.contains('/') {
                format!("Nothing at {p} runs: check the path, and that the file is executable.")
            } else {
                format!("This PC has no program called \u{201c}{p}\u{201d}. Check the spelling, or give its full path.")
            }
        }
        _ => {}
    }
    let name = name.trim();
    if !name.is_empty() && others.iter().any(|c| c.key != key && c.name.trim().eq_ignore_ascii_case(name)) {
        return format!("Another command is already called \u{201c}{name}\u{201d}. A name of its own tells them apart on the phone.");
    }
    String::new()
}

#[cfg(test)]
mod command_checks {
    use super::*;

    fn cmd(key: &str, name: &str) -> CommandData {
        CommandData { key: key.into(), name: name.into(), command: "true".into(), fingerprint: false, every_time: false }
    }

    #[test]
    fn finds_the_program_as_sh_would() {
        assert_eq!(first_program("  backup --quiet"), Some("backup".into()));
        assert_eq!(first_program("LANG=C FOO='a b' ls -l"), Some("ls".into()));
        assert_eq!(first_program("'/opt/My App/run' x"), Some("/opt/My App/run".into()));
        assert_eq!(first_program("~/bin/x;y"), Some("~/bin/x".into()));
        assert_eq!(first_program("cd /tmp && make"), None);
        assert_eq!(first_program("(sleep 1; x)"), None);
        assert_eq!(first_program("$EDITOR file"), None);
        assert_eq!(first_program(""), None);
    }

    #[test]
    fn warns_about_likely_mistakes_only() {
        let have = |p: &str| ["ls", "pkexec", "systemctl"].contains(&p);
        let list = [cmd("a", "Lock"), cmd("b", "Backup")];
        assert!(command_warning("", "Files", "lsx -l", &list, &have).contains("lsx"));
        assert!(command_warning("", "Update", "sudo pacman -Syu", &list, &have).contains("pkexec"));
        assert!(command_warning("", "lock ", "ls", &list, &have).contains("already called"));
        // Editing a command doesn't clash with itself.
        assert_eq!(command_warning("a", "Lock", "ls", &list, &have), "");
        assert_eq!(command_warning("", "Restart", "pkexec systemctl reboot", &list, &have), "");
        assert_eq!(command_warning("", "Go", "cd ~ && make", &list, &have), "");
    }
}
