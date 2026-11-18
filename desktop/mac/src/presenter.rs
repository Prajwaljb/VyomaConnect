//! The presenter's pointer (stock `kdeconnect.presenter`): a red dot over everything while the phone
//! points. `{dx, dy}` moves it (the screen is -1..1 across, dy scaled by the aspect, as on Linux),
//! `{stop: true}` hides it, and so does a second without word. The dot is a small window of its own
//! process (`vyoma-mac --presenter-window`, AppKit wants the main thread), fed lines on stdin,
//! started with the first move and gone after a minute of nothing.

use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::Value;

pub const PRESENTER: &str = "kdeconnect.presenter";
pub const WINDOW_ARG: &str = "--presenter-window";
/// After a failed start, how long before trying again.
const RETRY_AFTER: Duration = Duration::from_secs(5);

/// The line for the window, for a packet's [body].
fn line(body: &Value) -> Option<String> {
    if body["stop"].as_bool() == Some(true) {
        return Some("stop".into());
    }
    let number = |v: &Value| v.as_f64().filter(|n| n.is_finite());
    let (dx, dy) = (number(&body["dx"])?, number(&body["dy"])?);
    Some(format!("move {} {}", dx.clamp(-2.0, 2.0), dy.clamp(-2.0, 2.0)))
}

/// Where the phone's packets go; a thread keeps the window fed, so a stuck one can't hold up the agent.
#[derive(Default)]
pub struct Presenter(Option<mpsc::Sender<String>>);

impl Presenter {
    pub fn on_packet(&mut self, body: &Value) {
        let Some(line) = line(body) else { return };
        let tx = self.0.get_or_insert_with(|| {
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || feed(rx));
            tx
        });
        let _ = tx.send(line);
    }

    pub fn stop(&self) {
        if let Some(tx) = &self.0 {
            let _ = tx.send("stop".into());
        }
    }
}

fn feed(lines: mpsc::Receiver<String>) {
    let mut window: Option<Child> = None;
    let mut tried: Option<Instant> = None;
    for line in lines {
        let mut wrote = write(&mut window, &line);
        // Only a move is worth a window, and it may have closed itself since the last one.
        if !wrote && line != "stop" && tried.is_none_or(|t| t.elapsed() > RETRY_AFTER) {
            tried = Some(Instant::now());
            window = open();
            wrote = write(&mut window, &line);
        }
        if !wrote && let Some(mut gone) = window.take() {
            let _ = gone.kill();
            let _ = gone.wait();
        }
    }
}

fn write(window: &mut Option<Child>, line: &str) -> bool {
    window.as_mut().and_then(|w| w.stdin.as_mut()).is_some_and(|input| writeln!(input, "{line}").and_then(|()| input.flush()).is_ok())
}

fn open() -> Option<Child> {
    let exe = std::env::current_exe().ok()?;
    match Command::new(exe).arg(WINDOW_ARG).stdin(Stdio::piped()).stdout(Stdio::null()).spawn() {
        Ok(child) => Some(child),
        Err(e) => {
            tracing::warn!("no presenter pointer: {e}");
            None
        }
    }
}

/// The dot's place, each way -1..1 from the middle of the screen, y growing downward.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq)]
struct Spot {
    x: f64,
    y: f64,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
impl Spot {
    const CENTRE: Spot = Spot { x: 0.0, y: 0.0 };

    fn moved(self, dx: f64, dy: f64, (w, h): (f64, f64)) -> Spot {
        let aspect = if h > 0.0 { w / h } else { 1.0 };
        Spot { x: (self.x + dx).clamp(-1.0, 1.0), y: (self.y + dy * aspect).clamp(-1.0, 1.0) }
    }

    /// The bottom-left corner of a [side]-point window around the spot, on a screen at [origin] of
    /// [size], in AppKit's coordinates (y growing upward), kept on the screen.
    fn corner(self, origin: (f64, f64), size: (f64, f64), side: f64) -> (f64, f64) {
        let x = (self.x + 1.0) / 2.0 * size.0 - side / 2.0;
        let y = (1.0 - self.y) / 2.0 * size.1 - side / 2.0;
        (origin.0 + x.clamp(0.0, (size.0 - side).max(0.0)), origin.1 + y.clamp(0.0, (size.1 - side).max(0.0)))
    }
}

/// The window process: lines on stdin, until they end or a minute passes without one.
pub fn window() -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    {
        view::run()
    }
    #[cfg(not(target_os = "macos"))]
    {
        anyhow::bail!("only on a Mac")
    }
}

#[cfg(target_os = "macos")]
mod view {
    use std::cell::RefCell;
    use std::ffi::c_void;
    use std::io::BufRead;
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    use anyhow::{Result, anyhow};
    use dispatch2::DispatchQueue;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2::{MainThreadMarker, MainThreadOnly, msg_send};
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSScreen, NSView, NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask};
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    use super::Spot;

    /// The window's side, room for the glow around the dot.
    const SIDE: f64 = 48.0;
    const DOT: f64 = 18.0;
    /// Above everything, full-screen apps too (NSScreenSaverWindowLevel).
    const LEVEL: isize = 1000;
    const QUIET: Duration = Duration::from_secs(1);
    const CLOSE_AFTER: Duration = Duration::from_secs(60);

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGColorCreateSRGB(r: f64, g: f64, b: f64, a: f64) -> *const c_void;
    }

    struct Dot {
        window: Retained<NSWindow>,
        spot: Spot,
        shown: bool,
    }

    thread_local! {
        static DOT_WINDOW: RefCell<Option<Dot>> = const { RefCell::new(None) };
    }

    static START: OnceLock<Instant> = OnceLock::new();
    static HEARD_MS: AtomicU64 = AtomicU64::new(0);

    fn now_ms() -> u64 {
        START.get_or_init(Instant::now).elapsed().as_millis() as u64
    }

    fn hide() {
        DOT_WINDOW.with_borrow_mut(|d| {
            if let Some(d) = d.as_mut().filter(|d| d.shown) {
                d.window.orderOut(None);
                (d.shown, d.spot) = (false, Spot::CENTRE);
            }
        });
    }

    fn moved(dx: f64, dy: f64) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let Some(screen) = NSScreen::mainScreen(mtm) else { return };
        let frame = screen.frame();
        let size = (frame.size.width, frame.size.height);
        DOT_WINDOW.with_borrow_mut(|d| {
            let Some(d) = d.as_mut() else { return };
            d.spot = d.spot.moved(dx, dy, size);
            let (x, y) = d.spot.corner((frame.origin.x, frame.origin.y), size, SIDE);
            d.window.setFrameOrigin(NSPoint::new(x, y));
            if !d.shown {
                d.window.orderFrontRegardless();
                d.shown = true;
            }
        });
    }

    fn read_lines() {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            HEARD_MS.store(now_ms(), Ordering::Relaxed);
            let mut words = line.split(' ');
            match words.next() {
                Some("stop") => DispatchQueue::main().exec_async(hide),
                Some("move") => {
                    let mut number = || words.next().and_then(|w| w.parse::<f64>().ok());
                    if let (Some(dx), Some(dy)) = (number(), number()) {
                        DispatchQueue::main().exec_async(move || moved(dx, dy));
                    }
                }
                _ => {}
            }
        }
        std::process::exit(0);
    }

    fn watch_the_quiet() {
        loop {
            std::thread::sleep(Duration::from_millis(200));
            let quiet = Duration::from_millis(now_ms().saturating_sub(HEARD_MS.load(Ordering::Relaxed)));
            if quiet > CLOSE_AFTER {
                std::process::exit(0);
            }
            if quiet > QUIET {
                DispatchQueue::main().exec_async(hide);
            }
        }
    }

    pub fn run() -> Result<()> {
        let mtm = MainThreadMarker::new().ok_or_else(|| anyhow!("not on the main thread"))?;
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(SIDE, SIDE));
        // SAFETY: a window made and used on the main thread; kept until the process ends.
        let window = unsafe { NSWindow::initWithContentRect_styleMask_backing_defer(NSWindow::alloc(mtm), rect, NSWindowStyleMask::Borderless, NSBackingStoreType::Buffered, false) };
        let view = NSView::initWithFrame(NSView::alloc(mtm), rect);
        view.setWantsLayer(true);
        let colors = AnyClass::get(c"NSColor").ok_or_else(|| anyhow!("no NSColor"))?;
        let layers = AnyClass::get(c"CALayer").ok_or_else(|| anyhow!("no CALayer"))?;
        // SAFETY: AppKit's and Core Animation's documented methods with their declared types; the
        // colours are never released, as they live as long as this small process.
        unsafe {
            window.setReleasedWhenClosed(false);
            let clear: *mut AnyObject = msg_send![colors, clearColor];
            let _: () = msg_send![&*window, setBackgroundColor: clear];
            let root: *mut AnyObject = msg_send![&*view, layer];
            let dot: Retained<AnyObject> = msg_send![layers, new];
            let red = CGColorCreateSRGB(0.94, 0.2, 0.2, 1.0);
            let at = (SIDE - DOT) / 2.0;
            let _: () = msg_send![&*dot, setFrame: NSRect::new(NSPoint::new(at, at), NSSize::new(DOT, DOT))];
            let _: () = msg_send![&*dot, setCornerRadius: DOT / 2.0];
            let _: () = msg_send![&*dot, setBackgroundColor: red];
            let _: () = msg_send![&*dot, setBorderWidth: 2.0f64];
            let _: () = msg_send![&*dot, setBorderColor: CGColorCreateSRGB(1.0, 1.0, 1.0, 1.0)];
            let _: () = msg_send![&*dot, setShadowColor: red];
            let _: () = msg_send![&*dot, setShadowRadius: 10.0f64];
            let _: () = msg_send![&*dot, setShadowOpacity: 0.8f32];
            let _: () = msg_send![&*dot, setShadowOffset: NSSize::new(0.0, 0.0)];
            let _: () = msg_send![root, addSublayer: &*dot];
        }
        window.setOpaque(false);
        window.setHasShadow(false);
        window.setIgnoresMouseEvents(true);
        window.setLevel(LEVEL);
        window.setCollectionBehavior(NSWindowCollectionBehavior::CanJoinAllSpaces | NSWindowCollectionBehavior::Stationary | NSWindowCollectionBehavior::FullScreenAuxiliary | NSWindowCollectionBehavior::IgnoresCycle);
        window.setContentView(Some(&view));
        DOT_WINDOW.with_borrow_mut(|d| *d = Some(Dot { window, spot: Spot::CENTRE, shown: false }));
        HEARD_MS.store(now_ms(), Ordering::Relaxed);
        std::thread::Builder::new().name("lines".into()).spawn(read_lines)?;
        std::thread::Builder::new().name("quiet".into()).spawn(watch_the_quiet)?;
        app.run();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn packets_become_lines() {
        assert_eq!(line(&json!({ "dx": 0.5, "dy": -0.25 })).unwrap(), "move 0.5 -0.25");
        assert_eq!(line(&json!({ "dx": 9.0, "dy": 0.0 })).unwrap(), "move 2 0");
        assert_eq!(line(&json!({ "stop": true })).unwrap(), "stop");
        assert_eq!(line(&json!({ "dx": 1.0 })), None);
    }

    #[test]
    fn the_dot_starts_in_the_middle_and_stays_on_screen() {
        let screen = (1600.0, 1000.0);
        assert_eq!(Spot::CENTRE.corner((0.0, 0.0), screen, 48.0), (776.0, 476.0));
        let top_left = Spot::CENTRE.moved(-5.0, -5.0, screen);
        assert_eq!(top_left.corner((0.0, 0.0), screen, 48.0), (0.0, 952.0), "AppKit's y grows upward");
        let right = Spot::CENTRE.moved(5.0, 0.0, screen);
        assert_eq!(right.corner((100.0, 50.0), screen, 48.0), (1652.0, 526.0), "on a second screen's origin");
    }

    #[test]
    fn up_and_down_are_scaled_by_the_screens_shape() {
        assert_eq!(Spot::CENTRE.moved(0.0, 0.1, (2000.0, 1000.0)).y, 0.2);
    }
}
