//! X11 sessions (Cinnamon, MATE, Xfce, Budgie on X11, i3...), which have none of the Wayland ways
//! the rest of the app uses (the data-control clipboard, the RemoteDesktop and ScreenCast portals,
//! layer shell). X11 lets any app do these itself, so here they're done straight on the X server,
//! with no tools to install and no dialog to allow them:
//! - the clipboard: an invisible window owns CLIPBOARD to offer the phone's text, reads others'
//!   (TARGETS first, so a password manager's copy is never read), and XFIXES says when it changes;
//! - input: XTEST, the X server's own way to fake the keyboard and pointer; what no key types gets a
//!   spare keycode for the moment;
//! - the keyboard layout (`_XKB_RULES_NAMES`) and the primary monitor (RandR), for typing and casting.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use tokio::sync::{mpsc, oneshot};
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::Event;
use x11rb::protocol::xfixes::{self, ConnectionExt as _, SelectionEventMask};
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ClientMessageEvent, ConnectionExt as _, CreateWindowAux, EventMask, PropMode, Property, SELECTION_NOTIFY_EVENT,
    SelectionNotifyEvent, SelectionRequestEvent, Window, WindowClass,
};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;
use x11rb::{CURRENT_TIME, NONE};

/// Whether this session is X11: its own word for it, else an X display and no Wayland one.
pub fn active() -> bool {
    let set = |k: &str| std::env::var_os(k).is_some_and(|v| !v.is_empty());
    match std::env::var("XDG_SESSION_TYPE").as_deref() {
        Ok("x11") => set("DISPLAY"),
        Ok("wayland") => false,
        _ => set("DISPLAY") && !set("WAYLAND_DISPLAY"),
    }
}

fn connect() -> Result<(RustConnection, usize)> {
    x11rb::connect(None).context("can't reach the X server (DISPLAY)")
}

fn atom(conn: &RustConnection, name: &str) -> Result<Atom> {
    Ok(conn.intern_atom(false, name.as_bytes())?.reply()?.atom)
}

// ---- keyboard layout and screens ----------------------------------------------------------------

/// The X server's first keyboard layout and its variant (`setxkbmap`'s `_XKB_RULES_NAMES`).
pub fn keyboard_layout() -> Option<(String, String)> {
    let (conn, screen) = connect().ok()?;
    let root = conn.setup().roots[screen].root;
    let names = atom(&conn, "_XKB_RULES_NAMES").ok()?;
    let reply = conn.get_property(false, root, names, AtomEnum::STRING, 0, 1024).ok()?.reply().ok()?;
    parse_rules_names(&reply.value)
}

/// rules, model, layout, variant, options, each ending in a NUL; layouts and variants are lists.
fn parse_rules_names(value: &[u8]) -> Option<(String, String)> {
    let parts: Vec<String> = value.split(|b| *b == 0).map(|p| String::from_utf8_lossy(p).into_owned()).collect();
    let first = |i: usize| parts.get(i).and_then(|p| p.split(',').next()).unwrap_or("").trim().to_owned();
    let layout = first(2);
    (!layout.is_empty()).then(|| (layout, first(3)))
}

/// The primary monitor's place on the X screen (x, y, width, height); the whole screen without RandR.
pub fn primary_monitor() -> Result<(i32, i32, i32, i32)> {
    use x11rb::protocol::randr::ConnectionExt as _;
    let (conn, screen) = connect()?;
    let s = &conn.setup().roots[screen];
    let whole = (0, 0, s.width_in_pixels as i32, s.height_in_pixels as i32);
    let Ok(monitors) = conn.randr_get_monitors(s.root, true).map_err(anyhow::Error::from).and_then(|c| Ok(c.reply()?)) else { return Ok(whole) };
    let pick = monitors.monitors.iter().find(|m| m.primary).or(monitors.monitors.first());
    Ok(pick.map_or(whole, |m| (m.x as i32, m.y as i32, m.width as i32, m.height as i32)))
}

/// Turns the screens on (DPMS) and resets the screen saver's idle time, as a key press would.
pub fn wake_screens() -> Result<()> {
    use x11rb::protocol::dpms::{ConnectionExt as _, DPMSMode};
    use x11rb::protocol::xproto::ScreenSaver;
    let (conn, _) = connect()?;
    if conn.extension_information(x11rb::protocol::dpms::X11_EXTENSION_NAME)?.is_some() {
        conn.dpms_force_level(DPMSMode::ON)?;
    }
    conn.force_screen_saver(ScreenSaver::RESET)?;
    conn.flush()?;
    Ok(())
}

/// The next workspace to the right (or left), asked of the window manager the EWMH way, wrapping.
pub fn switch_workspace(right: bool) -> Result<()> {
    let (conn, screen) = connect()?;
    let root = conn.setup().roots[screen].root;
    let read = |name: &str| -> Result<u32> {
        let a = atom(&conn, name)?;
        let r = conn.get_property(false, root, a, AtomEnum::CARDINAL, 0, 1)?.reply()?;
        r.value32().and_then(|mut v| v.next()).with_context(|| format!("the window manager doesn't say {name}"))
    };
    let (now, count) = (read("_NET_CURRENT_DESKTOP")?, read("_NET_NUMBER_OF_DESKTOPS")?.max(1));
    let next = if right { (now + 1) % count } else { (now + count - 1) % count };
    let current = atom(&conn, "_NET_CURRENT_DESKTOP")?;
    let event = ClientMessageEvent::new(32, root, current, [next, CURRENT_TIME, 0, 0, 0]);
    conn.send_event(false, root, EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY, event)?;
    conn.flush()?;
    Ok(())
}

// ---- input ----------------------------------------------------------------------------------------

const KEY_PRESS: u8 = 2;
const KEY_RELEASE: u8 = 3;
const BUTTON_PRESS: u8 = 4;
const BUTTON_RELEASE: u8 = 5;
const MOTION: u8 = 6;
/// A wheel click's worth of the portal's smooth scrolling (libinput's 15 degrees).
const SCROLL_STEP: f64 = 15.0;
/// X keycodes are evdev's plus 8 (the evdev rules every X server uses now).
const EVDEV_OFFSET: i32 = 8;

/// The keyboard and pointer, through XTEST. Moves keep their fractions; scrolls add up to clicks.
pub struct Input {
    conn: RustConnection,
    root: Window,
    state: Mutex<InputState>,
}

#[derive(Default)]
struct InputState {
    rest: (f64, f64),
    scroll: (f64, f64),
    /// Spare keycodes lent to keysyms no key types, and which is next.
    lent: HashMap<u32, u8>,
    spare: Vec<u8>,
    next_spare: usize,
}

impl Input {
    pub fn open() -> Result<Self> {
        let (conn, screen) = connect()?;
        conn.extension_information(x11rb::protocol::xtest::X11_EXTENSION_NAME)?.context("the X server has no XTEST")?;
        conn.xtest_get_version(2, 2)?.reply()?;
        let root = conn.setup().roots[screen].root;
        Ok(Self { conn, root, state: Mutex::new(InputState::default()) })
    }

    fn fake(&self, kind: u8, detail: u8, x: i16, y: i16) -> Result<()> {
        self.conn.xtest_fake_input(kind, detail, CURRENT_TIME, self.root, x, y, 0)?;
        self.conn.flush()?;
        Ok(())
    }

    /// The touchpad's moves: by a distance.
    pub fn relative(&self, dx: f64, dy: f64) -> Result<()> {
        let (x, y) = {
            let mut s = self.state.lock().unwrap();
            let (x, y) = (s.rest.0 + dx, s.rest.1 + dy);
            let (ix, iy) = (x.trunc(), y.trunc());
            s.rest = (x - ix, y - iy);
            (ix as i16, iy as i16)
        };
        if x == 0 && y == 0 {
            return Ok(());
        }
        self.fake(MOTION, 1, x, y)
    }

    /// To a place on the X screen.
    pub fn absolute(&self, x: f64, y: f64) -> Result<()> {
        self.fake(MOTION, 0, x.round() as i16, y.round() as i16)
    }

    /// An evdev button (BTN_LEFT 0x110, BTN_RIGHT, BTN_MIDDLE).
    pub fn button(&self, evdev: i32, down: bool) -> Result<()> {
        let button = match evdev {
            0x111 => 3,
            0x112 => 2,
            _ => 1,
        };
        self.fake(if down { BUTTON_PRESS } else { BUTTON_RELEASE }, button, 0, 0)
    }

    /// Smooth scrolling, as the portal takes it (positive: down, right), as wheel clicks.
    pub fn scroll(&self, dx: f64, dy: f64) -> Result<()> {
        let clicks = {
            let mut s = self.state.lock().unwrap();
            let (x, y) = (s.scroll.0 + dx, s.scroll.1 + dy);
            let (cx, cy) = ((x / SCROLL_STEP).trunc(), (y / SCROLL_STEP).trunc());
            s.scroll = (x - cx * SCROLL_STEP, y - cy * SCROLL_STEP);
            (cx as i32, cy as i32)
        };
        let (cx, cy) = clicks;
        for (n, plus, minus) in [(cy, 5u8, 4u8), (cx, 7, 6)] {
            let button = if n > 0 { plus } else { minus };
            for _ in 0..n.unsigned_abs().min(20) {
                self.fake(BUTTON_PRESS, button, 0, 0)?;
                self.fake(BUTTON_RELEASE, button, 0, 0)?;
            }
        }
        Ok(())
    }

    /// The fingers lifted: the next scroll starts from nothing.
    pub fn scroll_end(&self) {
        self.state.lock().unwrap().scroll = (0.0, 0.0);
    }

    /// A key by its evdev code.
    pub fn key(&self, evdev: i32, down: bool) -> Result<()> {
        let code = u8::try_from(evdev + EVDEV_OFFSET).context("no such key")?;
        self.fake(if down { KEY_PRESS } else { KEY_RELEASE }, code, 0, 0)
    }

    /// A keysym no key in the layout types (✓, emoji): a spare keycode is made to type it. Spares
    /// keep what they were lent until needed again, so an app still reading its keymap after the
    /// press can't see it change under it; they go back to nothing when this closes.
    pub fn keysym(&self, sym: u32, down: bool) -> Result<()> {
        let code = {
            let mut s = self.state.lock().unwrap();
            match s.lent.get(&sym) {
                Some(c) => *c,
                None => {
                    if !down {
                        return Ok(());
                    }
                    if s.spare.is_empty() {
                        s.spare = self.spare_keycodes()?;
                    }
                    if s.spare.is_empty() {
                        bail!("no spare key to type it with");
                    }
                    let code = s.spare[s.next_spare % s.spare.len()];
                    s.next_spare += 1;
                    s.lent.retain(|_, c| *c != code);
                    s.lent.insert(sym, code);
                    self.conn.change_keyboard_mapping(1, code, 2, &[sym, sym])?;
                    // Every app hears of the new mapping before the press that follows.
                    self.conn.get_input_focus()?.reply()?;
                    code
                }
            }
        };
        self.fake(if down { KEY_PRESS } else { KEY_RELEASE }, code, 0, 0)
    }

    /// Keycodes with nothing on them, from the top down (a handful is plenty).
    fn spare_keycodes(&self) -> Result<Vec<u8>> {
        let setup = self.conn.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let map = self.conn.get_keyboard_mapping(min, max - min + 1)?.reply()?;
        let per = map.keysyms_per_keycode as usize;
        if per == 0 {
            return Ok(Vec::new());
        }
        Ok(map
            .keysyms
            .chunks(per)
            .enumerate()
            .filter(|(_, syms)| syms.iter().all(|s| *s == 0))
            .map(|(i, _)| min + i as u8)
            .rev()
            .take(8)
            .collect())
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        let lent: Vec<u8> = self.state.lock().unwrap().lent.values().copied().collect();
        for code in lent {
            let _ = self.conn.change_keyboard_mapping(1, code, 2, &[0, 0]);
        }
        // Waits until the server has done it.
        let _ = self.conn.get_input_focus().map(|c| c.reply());
    }
}

// ---- clipboard ----------------------------------------------------------------------------------

/// Password managers (KeePassXC, KWallet) mark what they copy with this target; it's never read.
const SECRET_HINT: &str = "x-kde-passwordManagerHint";
/// A copy whose owner hasn't answered in this long is given up on (a frozen app).
const READ_WAIT: Duration = Duration::from_secs(2);

enum Order {
    Set(Option<String>, oneshot::Sender<bool>),
    Get(oneshot::Sender<Option<String>>),
    Follow(mpsc::UnboundedSender<()>),
}

struct Clip {
    conn: Arc<RustConnection>,
    window: Window,
    wake: Atom,
    orders: Arc<Mutex<VecDeque<Order>>>,
}

impl Clip {
    fn send(&self, order: Order) -> Result<()> {
        self.orders.lock().unwrap().push_back(order);
        // Wakes the clipboard's thread, which waits on X events.
        let event = ClientMessageEvent::new(32, self.window, self.wake, [0u32; 5]);
        self.conn.send_event(false, self.window, EventMask::NO_EVENT, event)?;
        self.conn.flush()?;
        Ok(())
    }
}

fn clip() -> Result<&'static Clip> {
    static CLIP: OnceLock<std::result::Result<Clip, String>> = OnceLock::new();
    CLIP.get_or_init(|| start_clip().map_err(|e| format!("{e:#}"))).as_ref().map_err(|e| anyhow::anyhow!("{e}"))
}

struct Atoms {
    clipboard: Atom,
    targets: Atom,
    utf8: Atom,
    text: Atom,
    plain_utf8: Atom,
    plain: Atom,
    incr: Atom,
    secret: Atom,
    prop: Atom,
    wake: Atom,
}

fn start_clip() -> Result<Clip> {
    let (conn, screen) = connect()?;
    let conn = Arc::new(conn);
    let root = conn.setup().roots[screen].root;
    let window = conn.generate_id()?;
    conn.create_window(0, window, root, -10, -10, 1, 1, 0, WindowClass::INPUT_ONLY, 0, &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE))?;
    conn.extension_information(xfixes::X11_EXTENSION_NAME)?.context("the X server has no XFIXES")?;
    conn.xfixes_query_version(5, 0)?.reply()?;
    let a = |n: &str| atom(&conn, n);
    let atoms = Atoms {
        clipboard: a("CLIPBOARD")?,
        targets: a("TARGETS")?,
        utf8: a("UTF8_STRING")?,
        text: a("TEXT")?,
        plain_utf8: a("text/plain;charset=utf-8")?,
        plain: a("text/plain")?,
        incr: a("INCR")?,
        secret: a(SECRET_HINT)?,
        prop: a("VYOMA_CLIPBOARD")?,
        wake: a("VYOMA_WAKE")?,
    };
    conn.xfixes_select_selection_input(window, atoms.clipboard, SelectionEventMask::SET_SELECTION_OWNER)?;
    conn.flush()?;
    let orders = Arc::new(Mutex::new(VecDeque::new()));
    let clip = Clip { conn: conn.clone(), window, wake: atoms.wake, orders: orders.clone() };
    let thread_conn = conn.clone();
    std::thread::Builder::new().name("x11-clipboard".into()).spawn(move || {
        let mut owner = Owner { conn: thread_conn, window, atoms, orders, held: None, reading: None, waiting: Vec::new(), followers: Vec::new() };
        if let Err(e) = owner.run() {
            tracing::warn!("X11 clipboard stopped: {e:#}");
        }
    })?;
    Ok(clip)
}

/// A copy being read from its owner: first its targets, then its text (in INCR chunks if large).
enum Reading {
    Targets(Instant),
    Text(Instant),
    Chunks(Instant, Vec<u8>),
}

impl Reading {
    fn since(&self) -> Instant {
        match self {
            Reading::Targets(t) | Reading::Text(t) | Reading::Chunks(t, _) => *t,
        }
    }
}

struct Owner {
    conn: Arc<RustConnection>,
    window: Window,
    atoms: Atoms,
    orders: Arc<Mutex<VecDeque<Order>>>,
    /// What this PC offers as the clipboard, while it owns it.
    held: Option<String>,
    reading: Option<Reading>,
    waiting: Vec<oneshot::Sender<Option<String>>>,
    followers: Vec<mpsc::UnboundedSender<()>>,
}

impl Owner {
    fn run(&mut self) -> Result<()> {
        loop {
            let event = self.conn.wait_for_event()?;
            match event {
                Event::ClientMessage(m) if m.window == self.window && m.type_ == self.atoms.wake => self.obey()?,
                Event::SelectionRequest(r) => self.answer(&r)?,
                Event::SelectionClear(c) if c.selection == self.atoms.clipboard => self.held = None,
                Event::SelectionNotify(n) if n.requestor == self.window => self.got(&n)?,
                Event::PropertyNotify(p) if p.window == self.window && p.atom == self.atoms.prop && p.state == Property::NEW_VALUE => self.chunk()?,
                Event::XfixesSelectionNotify(e) if e.selection == self.atoms.clipboard && e.owner != self.window && e.owner != NONE => {
                    self.followers.retain(|f| f.send(()).is_ok());
                }
                _ => {}
            }
        }
    }

    fn obey(&mut self) -> Result<()> {
        let orders: Vec<Order> = self.orders.lock().unwrap().drain(..).collect();
        for order in orders {
            match order {
                Order::Set(text, done) => {
                    let ok = self.take(text)?;
                    let _ = done.send(ok);
                }
                Order::Get(reply) => {
                    if let Some(text) = &self.held {
                        let _ = reply.send(Some(text.clone()));
                        continue;
                    }
                    self.waiting.push(reply);
                    if self.reading.as_ref().is_some_and(|r| r.since().elapsed() < READ_WAIT) {
                        continue;
                    }
                    self.reading = Some(Reading::Targets(Instant::now()));
                    self.conn.convert_selection(self.window, self.atoms.clipboard, self.atoms.targets, self.atoms.prop, CURRENT_TIME)?;
                    self.conn.flush()?;
                }
                Order::Follow(tx) => self.followers.push(tx),
            }
        }
        Ok(())
    }

    /// Owns the clipboard with [text], or lets it go (None).
    fn take(&mut self, text: Option<String>) -> Result<bool> {
        let owner = if text.is_some() { self.window } else { NONE };
        self.held = text;
        self.conn.set_selection_owner(owner, self.atoms.clipboard, CURRENT_TIME)?;
        let now = self.conn.get_selection_owner(self.atoms.clipboard)?.reply()?.owner;
        Ok(now == owner)
    }

    /// Another app asks for what this PC holds.
    fn answer(&mut self, r: &SelectionRequestEvent) -> Result<()> {
        let a = &self.atoms;
        let property = if r.property == NONE { r.target } else { r.property };
        let mut given = NONE;
        if let (true, Some(text)) = (r.selection == a.clipboard, &self.held) {
            if r.target == a.targets {
                let list = [a.targets, a.utf8, a.plain_utf8, a.plain, a.text, AtomEnum::STRING.into()];
                self.conn.change_property32(PropMode::REPLACE, r.requestor, property, AtomEnum::ATOM, &list)?;
                given = property;
            } else if [a.utf8, a.plain_utf8, a.plain, a.text, AtomEnum::STRING.into()].contains(&r.target)
                && text.len() + 1024 < self.conn.maximum_request_bytes()
            {
                let kind = if r.target == a.text { a.utf8 } else { r.target };
                self.conn.change_property8(PropMode::REPLACE, r.requestor, property, kind, text.as_bytes())?;
                given = property;
            }
        }
        let notify = SelectionNotifyEvent { response_type: SELECTION_NOTIFY_EVENT, sequence: 0, time: r.time, requestor: r.requestor, selection: r.selection, target: r.target, property: given };
        self.conn.send_event(false, r.requestor, EventMask::NO_EVENT, notify)?;
        self.conn.flush()?;
        Ok(())
    }

    /// The owner answered one of this window's requests.
    fn got(&mut self, n: &SelectionNotifyEvent) -> Result<()> {
        let Some(reading) = self.reading.take() else { return Ok(()) };
        if n.property == NONE {
            return self.done(None);
        }
        let reply = self.conn.get_property(true, self.window, self.atoms.prop, AtomEnum::ANY, 0, u32::MAX / 4)?.reply()?;
        match reading {
            Reading::Targets(_) => {
                let offered: Vec<Atom> = reply.value32().map(|v| v.collect()).unwrap_or_default();
                if offered.contains(&self.atoms.secret) {
                    return self.done(None);
                }
                let a = &self.atoms;
                let Some(kind) = [a.utf8, a.plain_utf8, AtomEnum::STRING.into(), a.text, a.plain].into_iter().find(|k| offered.contains(k)) else {
                    return self.done(None);
                };
                self.reading = Some(Reading::Text(Instant::now()));
                self.conn.convert_selection(self.window, self.atoms.clipboard, kind, self.atoms.prop, CURRENT_TIME)?;
                self.conn.flush()?;
                Ok(())
            }
            Reading::Text(t) | Reading::Chunks(t, _) => {
                if reply.type_ == self.atoms.incr {
                    // Large: it comes in pieces, each announced by a new value on the property.
                    self.reading = Some(Reading::Chunks(t, Vec::new()));
                    return Ok(());
                }
                self.done(Some(String::from_utf8_lossy(&reply.value).into_owned()))
            }
        }
    }

    fn chunk(&mut self) -> Result<()> {
        if !matches!(self.reading, Some(Reading::Chunks(..))) {
            return Ok(());
        }
        let Some(Reading::Chunks(t, mut buf)) = self.reading.take() else { return Ok(()) };
        let reply = self.conn.get_property(true, self.window, self.atoms.prop, AtomEnum::ANY, 0, u32::MAX / 4)?.reply()?;
        self.conn.flush()?;
        if reply.value.is_empty() {
            return self.done(Some(String::from_utf8_lossy(&buf).into_owned()));
        }
        buf.extend_from_slice(&reply.value);
        self.reading = Some(Reading::Chunks(t, buf));
        Ok(())
    }

    fn done(&mut self, text: Option<String>) -> Result<()> {
        self.reading = None;
        for reply in self.waiting.drain(..) {
            let _ = reply.send(text.clone());
        }
        Ok(())
    }
}

/// The clipboard's text, unless it holds none or a password manager's copy.
pub async fn text() -> Option<String> {
    let (tx, rx) = oneshot::channel();
    clip().ok()?.send(Order::Get(tx)).ok()?;
    tokio::time::timeout(READ_WAIT, rx).await.ok()?.ok().flatten().filter(|t| !t.is_empty())
}

/// Puts [text] on the clipboard (None empties it); whether it took.
pub async fn set(text: Option<&str>) -> bool {
    let (tx, rx) = oneshot::channel();
    let sent = clip().and_then(|c| c.send(Order::Set(text.map(str::to_owned), tx)));
    if let Err(e) = sent {
        tracing::warn!("can't set the clipboard: {e:#}");
        return false;
    }
    matches!(tokio::time::timeout(READ_WAIT, rx).await, Ok(Ok(true)))
}

/// Calls [changed] on each copy made by another app, for as long as it can follow them.
pub async fn follow(mut changed: impl FnMut()) -> Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    clip()?.send(Order::Follow(tx))?;
    while rx.recv().await.is_some() {
        changed();
    }
    bail!("the X11 clipboard stopped")
}

#[cfg(test)]
mod tests {
    use super::*;
    use x11rb::protocol::xproto::ChangeWindowAttributesAux;

    #[test]
    fn layout_from_rules_names() {
        assert_eq!(parse_rules_names(b"evdev\0pc105\0de,us\0nodeadkeys,\0grp:alt_shift_toggle\0"), Some(("de".into(), "nodeadkeys".into())));
        assert_eq!(parse_rules_names(b"evdev\0pc105\0us\0\0\0"), Some(("us".into(), String::new())));
        assert_eq!(parse_rules_names(b""), None);
    }

    /// Against a headless X server: VYOMA_X11_TEST=1 with DISPLAY set to an Xvfb.
    #[tokio::test]
    async fn clipboard_round_trip() {
        if std::env::var_os("VYOMA_X11_TEST").is_none() {
            return;
        }
        assert!(set(Some("from the phone ✓")).await, "owns the clipboard");
        assert_eq!(text().await.as_deref(), Some("from the phone ✓"), "reads its own copy");
        assert!(set(None).await, "lets it go");
        assert_eq!(text().await, None);

        // Another app copies: it's heard, and its text read through the X server.
        let (seen_tx, mut seen) = mpsc::unbounded_channel();
        tokio::spawn(follow(move || {
            let _ = seen_tx.send(());
        }));
        tokio::time::sleep(Duration::from_millis(200)).await;
        let other = std::thread::spawn(|| other_app_copies("copied elsewhere", false, 2));
        tokio::time::timeout(Duration::from_secs(2), seen.recv()).await.expect("the copy is heard");
        assert_eq!(text().await.as_deref(), Some("copied elsewhere"));
        other.join().unwrap();

        // A password manager's copy: its targets give it away, and its text is never asked for.
        let other = std::thread::spawn(|| other_app_copies("hunter2", true, 1));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(text().await, None);
        other.join().unwrap();

        // A large copy comes in INCR pieces.
        let big: String = (0..40_000).map(|i| char::from(b'a' + (i % 26) as u8)).collect();
        let sent = big.clone();
        let other = std::thread::spawn(move || other_app_copies_in_pieces(&sent, 4096));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(text().await.as_deref(), Some(big.as_str()), "pieced together");
        other.join().unwrap();

        // Another app pastes what the phone sent.
        assert!(set(Some("phone text")).await);
        let pasted = tokio::task::spawn_blocking(other_app_pastes).await.unwrap();
        assert_eq!(pasted, "phone text");
    }

    /// XTEST against the headless X server: what a window under the pointer receives.
    #[test]
    fn input_reaches_a_window() {
        if std::env::var_os("VYOMA_X11_TEST").is_none() {
            return;
        }
        use x11rb::protocol::xproto::InputFocus;
        let (conn, screen) = connect().unwrap();
        let s = conn.setup().roots[screen].clone();
        let w = conn.generate_id().unwrap();
        let mask = EventMask::KEY_PRESS | EventMask::BUTTON_PRESS | EventMask::POINTER_MOTION;
        conn.create_window(0, w, s.root, 0, 0, s.width_in_pixels, s.height_in_pixels, 0, WindowClass::INPUT_OUTPUT, 0, &CreateWindowAux::new().event_mask(mask)).unwrap();
        conn.map_window(w).unwrap();
        conn.set_input_focus(InputFocus::POINTER_ROOT, w, CURRENT_TIME).unwrap();
        conn.get_input_focus().unwrap().reply().unwrap();

        let input = Input::open().unwrap();
        input.absolute(100.0, 100.0).unwrap();
        input.relative(10.5, 5.0).unwrap();
        input.relative(10.5, 5.0).unwrap();
        let at = conn.query_pointer(s.root).unwrap().reply().unwrap();
        assert_eq!((at.root_x, at.root_y), (121, 110), "moves keep their fractions");

        input.button(0x111, true).unwrap();
        input.button(0x111, false).unwrap();
        input.scroll(0.0, 31.0).unwrap(); // two clicks down, the rest kept
        input.key(30, true).unwrap(); // evdev KEY_A
        input.key(30, false).unwrap();
        input.keysym(0x0100_2713, true).unwrap(); // ✓: no key has it
        input.keysym(0x0100_2713, false).unwrap();

        let (mut buttons, mut keys) = (Vec::new(), Vec::new());
        let until = Instant::now() + Duration::from_secs(2);
        while Instant::now() < until && keys.len() < 2 {
            match conn.poll_for_event().unwrap() {
                Some(Event::ButtonPress(b)) => buttons.push(b.detail),
                Some(Event::KeyPress(k)) => keys.push(k.detail),
                Some(_) => {}
                None => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        assert_eq!(buttons, [3, 5, 5], "right click, then two wheel clicks down");
        assert_eq!(keys[0], 38, "evdev 30 is X keycode 38");
        let map = conn.get_keyboard_mapping(keys[1], 1).unwrap().reply().unwrap();
        assert_eq!(map.keysyms[0], 0x0100_2713, "the spare key types ✓");
        drop(input);
        let map = conn.get_keyboard_mapping(keys[1], 1).unwrap().reply().unwrap();
        assert!(map.keysyms.iter().all(|k| *k == 0), "and is given back");
    }

    /// An app pasting: asks the owner for UTF8_STRING and reads it.
    fn other_app_pastes() -> String {
        let (conn, screen) = connect().unwrap();
        let root = conn.setup().roots[screen].root;
        let w = conn.generate_id().unwrap();
        conn.create_window(0, w, root, 0, 0, 1, 1, 0, WindowClass::INPUT_ONLY, 0, &CreateWindowAux::new()).unwrap();
        let (clipboard, utf8, prop) = (atom(&conn, "CLIPBOARD").unwrap(), atom(&conn, "UTF8_STRING").unwrap(), atom(&conn, "PASTED").unwrap());
        conn.convert_selection(w, clipboard, utf8, prop, CURRENT_TIME).unwrap();
        conn.flush().unwrap();
        loop {
            if let Event::SelectionNotify(n) = conn.wait_for_event().unwrap() {
                assert_ne!(n.property, NONE, "the owner answered");
                let r = conn.get_property(true, w, prop, AtomEnum::ANY, 0, 1 << 20).unwrap().reply().unwrap();
                return String::from_utf8(r.value).unwrap();
            }
        }
    }

    /// An app that owns the clipboard with [text] and hands it over INCR, [piece] bytes at a time.
    fn other_app_copies_in_pieces(text: &str, piece: usize) {
        let (conn, screen) = connect().unwrap();
        let root = conn.setup().roots[screen].root;
        let w = conn.generate_id().unwrap();
        conn.create_window(0, w, root, 0, 0, 1, 1, 0, WindowClass::INPUT_ONLY, 0, &CreateWindowAux::new()).unwrap();
        let (clipboard, targets, utf8, incr) = (atom(&conn, "CLIPBOARD").unwrap(), atom(&conn, "TARGETS").unwrap(), atom(&conn, "UTF8_STRING").unwrap(), atom(&conn, "INCR").unwrap());
        conn.set_selection_owner(w, clipboard, CURRENT_TIME).unwrap();
        conn.flush().unwrap();
        let mut pieces = text.as_bytes().chunks(piece);
        let mut sending: Option<(Window, Atom)> = None;
        loop {
            match conn.wait_for_event().unwrap() {
                Event::SelectionRequest(r) => {
                    if r.target == targets {
                        conn.change_property32(PropMode::REPLACE, r.requestor, r.property, AtomEnum::ATOM, &[targets, utf8]).unwrap();
                    } else {
                        // Announces the size, then sends a piece each time the reader deletes the last.
                        conn.change_window_attributes(r.requestor, &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE)).unwrap();
                        conn.change_property32(PropMode::REPLACE, r.requestor, r.property, incr, &[text.len() as u32]).unwrap();
                        sending = Some((r.requestor, r.property));
                    }
                    let n = SelectionNotifyEvent { response_type: SELECTION_NOTIFY_EVENT, sequence: 0, time: r.time, requestor: r.requestor, selection: r.selection, target: r.target, property: r.property };
                    conn.send_event(false, r.requestor, EventMask::NO_EVENT, n).unwrap();
                    conn.flush().unwrap();
                }
                Event::PropertyNotify(p) if p.state == Property::DELETE && sending == Some((p.window, p.atom)) => {
                    let next = pieces.next().unwrap_or(&[]);
                    conn.change_property8(PropMode::REPLACE, p.window, p.atom, utf8, next).unwrap();
                    conn.flush().unwrap();
                    if next.is_empty() {
                        conn.get_input_focus().unwrap().reply().unwrap();
                        return;
                    }
                }
                _ => {}
            }
        }
    }

    /// An app that owns the clipboard with [text] (marked as a password manager's if [secret]) and
    /// answers [answers] requests for it.
    fn other_app_copies(text: &str, secret: bool, answers: usize) {
        let (conn, screen) = connect().unwrap();
        let root = conn.setup().roots[screen].root;
        let w = conn.generate_id().unwrap();
        conn.create_window(0, w, root, 0, 0, 1, 1, 0, WindowClass::INPUT_ONLY, 0, &CreateWindowAux::new()).unwrap();
        let (clipboard, targets, utf8) = (atom(&conn, "CLIPBOARD").unwrap(), atom(&conn, "TARGETS").unwrap(), atom(&conn, "UTF8_STRING").unwrap());
        let hint = atom(&conn, SECRET_HINT).unwrap();
        conn.set_selection_owner(w, clipboard, CURRENT_TIME).unwrap();
        conn.flush().unwrap();
        let mut left = answers;
        while left > 0 {
            let Event::SelectionRequest(r) = conn.wait_for_event().unwrap() else { continue };
            if r.target == targets {
                let offered: &[Atom] = if secret { &[targets, utf8, hint] } else { &[targets, utf8] };
                conn.change_property32(PropMode::REPLACE, r.requestor, r.property, AtomEnum::ATOM, offered).unwrap();
            } else {
                assert!(!secret, "a password manager's text was asked for");
                conn.change_property8(PropMode::REPLACE, r.requestor, r.property, utf8, text.as_bytes()).unwrap();
            }
            let n = SelectionNotifyEvent { response_type: SELECTION_NOTIFY_EVENT, sequence: 0, time: r.time, requestor: r.requestor, selection: r.selection, target: r.target, property: r.property };
            conn.send_event(false, r.requestor, EventMask::NO_EVENT, n).unwrap();
            conn.flush().unwrap();
            left -= 1;
        }
        // Until the server has it all: a client that closes with events unread is reset, and the
        // server may drop what it hadn't read yet (a real app stays open while it owns the copy).
        conn.get_input_focus().unwrap().reply().unwrap();
    }
}
