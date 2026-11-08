//! Vyoma's desktop window. It only shows and asks: the daemon (`vyoma-desktop`) owns the links,
//! so closing or crashing this never drops a phone. `--snapshot <png>` renders it offscreen with
//! made-up phones instead (`--dark`, `--scene phones|pairing|nearby|unpair|notice|cable|pc|pc-setup|empty|down`).

mod daemon;
mod snapshot;

use std::cell::RefCell;
use std::sync::mpsc;
use std::time::Duration;

use daemon::{Daemon, DbusDaemon, Event, Health, PhoneData};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

slint::include_modules!();

/// How long a notice stays, and how often the window rereads the daemon without being told to
/// (a battery that moved without a change signal, a daemon that restarted).
const NOTICE_FOR: Duration = Duration::from_millis(3200);
const REREAD_EVERY: Duration = Duration::from_secs(5);
const RECONNECT_EVERY: Duration = Duration::from_secs(2);
/// The sidebar's id for This PC's page.
pub const THIS_PC: &str = "this-pc";

thread_local! {
    /// The devices last read, on the UI thread.
    static SHOWN: RefCell<Vec<PhoneData>> = const { RefCell::new(Vec::new()) };
    static NOTICES: RefCell<u64> = const { RefCell::new(0) };
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if let Some(at) = args.iter().position(|a| a == "--snapshot") {
        let path = args.get(at + 1).ok_or_else(|| anyhow::anyhow!("--snapshot needs a path"))?;
        let scene = args.iter().position(|a| a == "--scene").and_then(|i| args.get(i + 1)).map(String::as_str).unwrap_or("phones");
        return snapshot::run(path, args.iter().any(|a| a == "--dark"), scene);
    }
    let ui = MainWindow::new()?;
    wire(&ui);
    watch(ui.as_weak());
    ui.run()?;
    Ok(())
}

/// The window's buttons: each call runs off the UI thread, and a failure shows as a notice.
fn wire(ui: &MainWindow) {
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
                            if changed.send(()).is_err() {
                                return;
                            }
                        }
                        Event::Sent { id, count, error } => sent(&told, id, count, error),
                        Event::SetupDone { ok } => {
                            notify(&told, if ok { "Installed. This PC is set up".into() } else { "The install didn't finish".into() });
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
        loop {
            if daemon.is_none() {
                daemon = DbusDaemon::connect().ok();
                // Back (or there at start): This PC's state, for the sidebar's dot.
                if daemon.is_some() {
                    read_health(&ui);
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
    let checks: Vec<Check> = h.checks.iter().map(|c| Check { name: c.name.clone().into(), state: c.state.clone().into(), detail: c.detail.clone().into() }).collect();
    let notes: Vec<SharedString> = h.notes.iter().map(|n| n.clone().into()).collect();
    ui.set_features(ModelRc::new(VecModel::from(features)));
    ui.set_checks(ModelRc::new(VecModel::from(checks)));
    ui.set_notes(ModelRc::new(VecModel::from(notes)));
    ui.set_installable(h.installable);
    ui.set_installing(h.installing);
    ui.set_attention(h.attention() as i32);
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
    show(ui, &phones);
}

/// The sidebar's two lists and the chosen phone's page, keeping the choice while it's there.
fn show(ui: &MainWindow, phones: &[PhoneData]) {
    let (mut paired, mut nearby): (Vec<&PhoneData>, Vec<&PhoneData>) = phones.iter().partition(|p| p.paired);
    paired.sort_by_key(|p| (!p.linked, p.name.to_lowercase()));
    nearby.retain(|p| p.linked || !p.code.is_empty());
    nearby.sort_by_key(|p| p.name.to_lowercase());
    let selected = ui.get_selected().to_string();
    let current = paired.iter().chain(&nearby).find(|p| p.id == selected).or(paired.first()).or(nearby.first()).copied();
    ui.set_phones(ModelRc::new(VecModel::from(paired.iter().map(|p| to_ui(p)).collect::<Vec<_>>())));
    ui.set_nearby(ModelRc::new(VecModel::from(nearby.iter().map(|p| to_ui(p)).collect::<Vec<_>>())));
    // This PC's page stays put however the phones change.
    if selected == THIS_PC {
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
    Phone {
        id: p.id.clone().into(),
        name: p.name.clone().into(),
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
    }
}

/// Shows [text] at the bottom of the window for a moment.
fn notify(ui: &slint::Weak<MainWindow>, text: String) {
    let ui = ui.clone();
    let _ = slint::invoke_from_event_loop(move || {
        let Some(win) = ui.upgrade() else { return };
        let n = NOTICES.with(|c| {
            *c.borrow_mut() += 1;
            *c.borrow()
        });
        win.set_notice(text.into());
        slint::Timer::single_shot(NOTICE_FOR, move || {
            if NOTICES.with(|c| *c.borrow()) == n {
                if let Some(win) = ui.upgrade() {
                    win.set_notice(SharedString::new());
                }
            }
        });
    });
}
