//! The presenter's dot on X11 (Cinnamon, MATE, Xfce), which has no layer shell: a small
//! override-redirect window kept on top, that clicks pass through (an empty input shape). With a
//! compositor it's the same soft dot as on Wayland (a 32-bit window); without one, a plain dot cut
//! to its circle by the Shape extension. Its own thread and X connection, made on first use.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::Event;
use x11rb::protocol::shape::{self, ConnectionExt as _};
use x11rb::protocol::xfixes::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{
    ClipOrdering, ColormapAlloc, ConfigureWindowAux, ConnectionExt as _, CreateGCAux, CreateWindowAux, EventMask, ImageFormat, Rectangle, StackMode, VisualClass, WindowClass,
};
use x11rb::rust_connection::RustConnection;

use crate::presenter::{Order, QUIET, SIZE, Spot, paint_dot};

/// Starts the dot's thread; its orders go through the returned sender.
pub fn start() -> Result<mpsc::Sender<Order>> {
    let (conn, screen) = x11rb::connect(None).context("can't reach the X server")?;
    let monitor = vyoma_companion::x11::primary_monitor().unwrap_or((0, 0, 1920, 1080));
    let mut dot = Dot::new(conn, screen, monitor)?;
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new().name("presenter-x11".into()).spawn(move || {
        let mut heard = Instant::now();
        loop {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(Order::Move(dx, dy)) => {
                    heard = Instant::now();
                    if let Err(e) = dot.moved(dx, dy) {
                        tracing::warn!("presenter pointer: {e:#}");
                        return;
                    }
                }
                Ok(Order::Stop) => dot.hide(),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if dot.shown && heard.elapsed() > QUIET {
                        dot.hide();
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return dot.hide(),
            }
            // A window manager or a screen change may have uncovered it.
            while let Ok(Some(event)) = dot.conn.poll_for_event() {
                if let Event::Expose(_) = event {
                    let _ = dot.draw();
                }
            }
        }
    })?;
    Ok(tx)
}

struct Dot {
    conn: RustConnection,
    window: u32,
    gc: u32,
    depth: u8,
    /// The monitor it's on: x, y, width, height on the X screen.
    monitor: (i32, i32, i32, i32),
    spot: Spot,
    shown: bool,
    pixels: Vec<u8>,
}

impl Dot {
    fn new(conn: RustConnection, screen: usize, monitor: (i32, i32, i32, i32)) -> Result<Self> {
        let s = conn.setup().roots[screen].clone();
        // A compositor owns _NET_WM_CM_S<n>: then a 32-bit window blends in, glow and all.
        let cm = conn.intern_atom(false, format!("_NET_WM_CM_S{screen}").as_bytes())?.reply()?.atom;
        let composited = conn.get_selection_owner(cm)?.reply()?.owner != x11rb::NONE;
        let argb = composited
            .then(|| s.allowed_depths.iter().find(|d| d.depth == 32).and_then(|d| d.visuals.iter().find(|v| v.class == VisualClass::TRUE_COLOR)).map(|v| v.visual_id))
            .flatten();
        let window = conn.generate_id()?;
        let (depth, visual, mut aux) = match argb {
            Some(visual) => {
                let colormap = conn.generate_id()?;
                conn.create_colormap(ColormapAlloc::NONE, colormap, s.root, visual)?;
                (32, visual, CreateWindowAux::new().colormap(colormap).background_pixel(0).border_pixel(0))
            }
            None => (s.root_depth, s.root_visual, CreateWindowAux::new().background_pixel(s.black_pixel)),
        };
        aux = aux.override_redirect(1).event_mask(EventMask::EXPOSURE);
        conn.create_window(depth, window, s.root, 0, 0, SIZE as u16, SIZE as u16, 0, WindowClass::INPUT_OUTPUT, visual, &aux)?;
        // Clicks go through to whatever's under it.
        if conn.extension_information(xfixes::X11_EXTENSION_NAME)?.is_some() && conn.extension_information(shape::X11_EXTENSION_NAME)?.is_some() {
            conn.xfixes_query_version(5, 0)?.reply()?;
            let none = conn.generate_id()?;
            conn.xfixes_create_region(none, &[])?;
            conn.xfixes_set_window_shape_region(window, shape::SK::INPUT, 0, 0, none)?;
            conn.xfixes_destroy_region(none)?;
            if argb.is_none() {
                // No compositor: cut the window to the dot's white rim.
                conn.shape_rectangles(shape::SO::SET, shape::SK::BOUNDING, ClipOrdering::UNSORTED, window, 0, 0, &circle(SIZE, 6))?;
            }
        }
        let gc = conn.generate_id()?;
        conn.create_gc(gc, window, &CreateGCAux::new())?;
        let mut pixels = vec![0u8; (SIZE * SIZE * 4) as usize];
        paint_dot(&mut pixels, SIZE as usize);
        conn.flush()?;
        Ok(Self { conn, window, gc, depth, monitor, spot: Spot::CENTRE, shown: false, pixels })
    }

    fn moved(&mut self, dx: f64, dy: f64) -> Result<()> {
        if !self.shown {
            // A new presentation starts in the middle, as kdeconnectd's.
            self.spot = Spot::CENTRE;
            self.conn.map_window(self.window)?;
            self.shown = true;
        }
        let (mx, my, mw, mh) = self.monitor;
        self.spot = self.spot.moved(dx, dy, (mw, mh));
        let (x, y) = self.spot.corner((mw, mh));
        self.conn.configure_window(self.window, &ConfigureWindowAux::new().x(mx + x).y(my + y).stack_mode(StackMode::ABOVE))?;
        self.draw()
    }

    fn draw(&self) -> Result<()> {
        if !self.shown {
            return Ok(());
        }
        self.conn.put_image(ImageFormat::Z_PIXMAP, self.window, self.gc, SIZE as u16, SIZE as u16, 0, 0, 0, self.depth, &self.pixels)?;
        self.conn.flush()?;
        Ok(())
    }

    fn hide(&mut self) {
        if std::mem::take(&mut self.shown) {
            let _ = self.conn.unmap_window(self.window);
            let _ = self.conn.flush();
        }
    }
}

/// A disc of [radius] in the middle of a [size] square, as one rectangle per row.
fn circle(size: i32, radius: i32) -> Vec<Rectangle> {
    let c = size / 2;
    (-radius..radius)
        .map(|dy| {
            let y = dy as f64 + 0.5;
            let half = ((radius * radius) as f64 - y * y).max(0.0).sqrt().round() as i32;
            Rectangle { x: (c - half) as i16, y: (c + dy) as i16, width: (2 * half) as u16, height: 1 }
        })
        .filter(|r| r.width > 0)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cut_disc_is_round_and_centred() {
        let rows = circle(48, 6);
        assert_eq!(rows.len(), 12);
        assert_eq!(rows[0].width, rows[11].width, "top and bottom alike");
        assert!(rows[5].width >= 11 && rows[5].width <= 12, "widest across the middle");
        assert!(rows.iter().all(|r| r.x as i32 + r.width as i32 / 2 == 24), "centred");
    }

    /// Against a headless X server: VYOMA_X11_TEST=1 with DISPLAY set to an Xvfb.
    #[test]
    fn the_dot_shows_moves_and_hides() {
        if std::env::var_os("VYOMA_X11_TEST").is_none() {
            return;
        }
        let (conn, screen) = x11rb::connect(None).unwrap();
        let root = conn.setup().roots[screen].root;
        let (_, _, w, h) = vyoma_companion::x11::primary_monitor().unwrap();
        let pixel = |x: i32, y: i32| {
            let img = conn.get_image(ImageFormat::Z_PIXMAP, root, x as i16, y as i16, 1, 1, !0).unwrap().reply().unwrap();
            (img.data[2], img.data[1], img.data[0]) // BGRx → RGB
        };
        let dot = start().unwrap();
        dot.send(Order::Move(0.0, 0.0)).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let (r, g, b) = pixel(w / 2, h / 2);
        assert!(r > 200 && g < 100 && b < 100, "red in the middle: {:?}", (r, g, b));
        dot.send(Order::Move(0.5, 0.0)).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        assert_ne!(pixel(w / 2, h / 2).0, r, "it moved away");
        let (r2, _, _) = pixel(w * 3 / 4, h / 2);
        assert!(r2 > 200, "and is a quarter across");
        std::thread::sleep(QUIET + Duration::from_millis(400));
        assert!(pixel(w * 3 / 4, h / 2).0 < 50, "hidden after a quiet second");

        // With a compositor (here only its selection, claimed): the 32-bit window, red in the middle.
        let owner = conn.generate_id().unwrap();
        conn.create_window(0, owner, root, 0, 0, 1, 1, 0, WindowClass::INPUT_ONLY, 0, &CreateWindowAux::new()).unwrap();
        let cm = conn.intern_atom(false, format!("_NET_WM_CM_S{screen}").as_bytes()).unwrap().reply().unwrap().atom;
        conn.set_selection_owner(owner, cm, x11rb::CURRENT_TIME).unwrap();
        conn.flush().unwrap();
        let soft = start().unwrap();
        soft.send(Order::Move(0.0, 0.0)).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let (r, g, b) = pixel(w / 2, h / 2);
        assert!(r > 200 && g < 100 && b < 100, "red in the middle with a compositor too: {:?}", (r, g, b));
    }
}
