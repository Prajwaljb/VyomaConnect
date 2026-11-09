//! The presenter's pointer (stock `kdeconnect.presenter`): a dot over everything on screen while the
//! phone points, as kdeconnectd's overlay. `{dx, dy}` moves it (the screen is -1..1 across, dy scaled
//! by the aspect, as kdeconnectd does), `{stop: true}` hides it, and so does a second without word
//! (the phone keeps sending while it's held).
//!
//! It's a wlr-layer-shell surface on the overlay layer (KWin, wlroots and COSMIC have it): a small
//! square that takes no input, moved with its margins, so a move redraws nothing. Its own thread and
//! Wayland connection, made on first use. GNOME has no layer shell, so there Vyoma's GNOME extension
//! draws the same dot (`vyoma_companion::shell::point`).

use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde_json::Value;
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, Region};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::EventLoop;
use smithay_client_toolkit::reexports::calloop::channel;
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
use smithay_client_toolkit::reexports::client::protocol::{wl_output, wl_shm, wl_surface};
use smithay_client_toolkit::reexports::client::{Connection, QueueHandle};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure};
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{delegate_registry, registry_handlers};

pub const PRESENTER: &str = "kdeconnect.presenter";

/// The dot's square, in logical pixels: a 9 px core in a soft glow.
const SIZE: i32 = 48;
/// Gone this long without a packet: the phone stopped pointing.
const QUIET: Duration = Duration::from_secs(1);

enum Order {
    Move(f64, f64),
    Stop,
}

/// Where the phone's packets go; the overlay starts with the first.
#[derive(Default)]
pub struct Presenter {
    orders: Option<channel::Sender<Order>>,
    /// Wayland or its layer shell isn't there: said once, then quiet.
    failed: bool,
    /// On GNOME: the moves, in order, to the extension.
    gnome: Option<tokio::sync::mpsc::UnboundedSender<Order>>,
}

impl Presenter {
    pub fn on_packet(&mut self, body: &Value) {
        let order = if body["stop"].as_bool() == Some(true) {
            Order::Stop
        } else {
            let (Some(dx), Some(dy)) = (number(&body["dx"]), number(&body["dy"])) else { return };
            Order::Move(dx.clamp(-2.0, 2.0), dy.clamp(-2.0, 2.0))
        };
        if vyoma_companion::desktop::is_gnome() {
            let tx = self.gnome.get_or_insert_with(through_shell);
            let _ = tx.send(order);
            return;
        }
        if self.orders.is_none() && !self.failed && matches!(order, Order::Move(..)) {
            match start() {
                Ok(tx) => self.orders = Some(tx),
                Err(e) => {
                    tracing::warn!("no presenter pointer: {e:#}");
                    self.failed = true;
                }
            }
        }
        if let Some(tx) = &self.orders
            && tx.send(order).is_err()
        {
            // The overlay's thread ended (the compositor went away): the next packet starts another.
            self.orders = None;
        }
    }
}

/// One task passes the moves on in order; the extension hides the dot after a quiet second itself.
fn through_shell() -> tokio::sync::mpsc::UnboundedSender<Order> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut said = false;
        while let Some(order) = rx.recv().await {
            let done = match order {
                Order::Move(dx, dy) => vyoma_companion::shell::point(dx, dy).await,
                Order::Stop => vyoma_companion::shell::hide_pointer().await,
            };
            if let Err(e) = done
                && !std::mem::replace(&mut said, true)
            {
                tracing::warn!("no presenter pointer: {e:#}");
            }
        }
    });
    tx
}

/// Peers send numbers either way (see the protocol notes).
fn number(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str()?.parse().ok()).filter(|f: &f64| f.is_finite())
}

/// The dot's position, -1..1 each way, as kdeconnectd keeps it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Spot {
    x: f64,
    y: f64,
}

impl Spot {
    const CENTRE: Spot = Spot { x: 0.0, y: 0.0 };

    fn moved(self, dx: f64, dy: f64, (w, h): (i32, i32)) -> Spot {
        let aspect = if h > 0 { w as f64 / h as f64 } else { 1.0 };
        Spot { x: (self.x + dx).clamp(-1.0, 1.0), y: (self.y + dy * aspect).clamp(-1.0, 1.0) }
    }

    /// The square's top-left, in the output's logical pixels, kept on screen.
    fn corner(self, (w, h): (i32, i32)) -> (i32, i32) {
        let at = |f: f64, span: i32| (((f + 1.0) / 2.0 * span as f64).round() as i32 - SIZE / 2).clamp(0, (span - SIZE).max(0));
        (at(self.x, w), at(self.y, h))
    }
}

fn start() -> Result<channel::Sender<Order>> {
    let conn = Connection::connect_to_env().context("no Wayland session")?;
    let (globals, mut queue) = registry_queue_init(&conn).context("reading the compositor's globals")?;
    let qh = queue.handle();
    let compositor = CompositorState::bind(&globals, &qh).context("no wl_compositor")?;
    let shell = LayerShell::bind(&globals, &qh).context("the compositor has no layer shell")?;
    let shm = Shm::bind(&globals, &qh).context("no wl_shm")?;
    let (tx, rx) = channel::channel();
    let mut overlay = Overlay {
        registry: RegistryState::new(&globals),
        outputs: OutputState::new(&globals, &qh),
        pool: SlotPool::new((SIZE * SIZE * 4) as usize, &shm).context("no shared memory")?,
        shm,
        compositor,
        shell,
        layer: None,
        size: (1920, 1080),
        scale: 1,
        spot: Spot::CENTRE,
        heard: Instant::now(),
        configured: false,
    };
    // The screens' sizes before the first dot, so it never starts on a guess: one round trip binds them,
    // the next brings what they are.
    for _ in 0..2 {
        queue.roundtrip(&mut overlay).context("asking the compositor about its screens")?;
    }
    let (ready_tx, ready_rx) = mpsc::channel();
    std::thread::Builder::new().name("presenter".into()).spawn(move || {
        let mut events: EventLoop<Overlay> = match EventLoop::try_new() {
            Ok(l) => l,
            Err(e) => return drop(ready_tx.send(Err(anyhow::anyhow!("event loop: {e}")))),
        };
        if let Err(e) = WaylandSource::new(conn, queue).insert(events.handle()) {
            return drop(ready_tx.send(Err(anyhow::anyhow!("Wayland source: {e}"))));
        }
        if let Err(e) = events.handle().insert_source(rx, |event, _, o: &mut Overlay| match event {
            channel::Event::Msg(order) => o.obey(order, &qh),
            channel::Event::Closed => o.hide(),
        }) {
            return drop(ready_tx.send(Err(anyhow::anyhow!("orders: {e}"))));
        }
        let _ = ready_tx.send(Ok(()));
        loop {
            if events.dispatch(Some(Duration::from_millis(200)), &mut overlay).is_err() {
                tracing::warn!("presenter pointer: lost the compositor");
                return;
            }
            if overlay.layer.is_some() && overlay.heard.elapsed() > QUIET {
                overlay.hide();
            }
        }
    })?;
    ready_rx.recv().context("the presenter thread ended")??;
    Ok(tx)
}

struct Overlay {
    registry: RegistryState,
    outputs: OutputState,
    shm: Shm,
    pool: SlotPool,
    compositor: CompositorState,
    shell: LayerShell,
    layer: Option<LayerSurface>,
    /// The logical size of the output the dot is on.
    size: (i32, i32),
    scale: i32,
    spot: Spot,
    heard: Instant,
    configured: bool,
}

impl Overlay {
    fn obey(&mut self, order: Order, qh: &QueueHandle<Self>) {
        match order {
            Order::Stop => self.hide(),
            Order::Move(dx, dy) => {
                self.heard = Instant::now();
                if self.layer.is_none() {
                    self.show(qh);
                }
                self.spot = self.spot.moved(dx, dy, self.size);
                self.place();
            }
        }
    }

    fn show(&mut self, qh: &QueueHandle<Self>) {
        // A new presentation starts in the middle, as kdeconnectd's.
        self.spot = Spot::CENTRE;
        self.configured = false;
        if let Some(info) = self.outputs.outputs().next().and_then(|o| self.outputs.info(&o)) {
            self.size = info.logical_size.unwrap_or(self.size);
            self.scale = info.scale_factor.max(1);
        }
        let surface = self.compositor.create_surface(qh);
        // Clicks go through to whatever's under it.
        if let Ok(none) = Region::new(&self.compositor) {
            surface.set_input_region(Some(none.wl_region()));
        }
        let layer = self.shell.create_layer_surface(qh, surface, Layer::Overlay, Some("vyoma-presenter"), None);
        layer.set_anchor(Anchor::TOP | Anchor::LEFT);
        layer.set_size(SIZE as u32, SIZE as u32);
        // Measured from the screen's edge, not the panels'.
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        self.layer = Some(layer);
        self.place();
    }

    fn hide(&mut self) {
        // Dropping it destroys the surface.
        self.layer = None;
    }

    /// Moves the square to [spot]; until the first configure that commit only asks for one.
    fn place(&mut self) {
        let Some(layer) = &self.layer else { return };
        let (left, top) = self.spot.corner(self.size);
        layer.set_margin(top, 0, 0, left);
        layer.commit();
    }

    fn draw(&mut self) {
        let Some(layer) = &self.layer else { return };
        let px = SIZE * self.scale;
        let Ok((buffer, canvas)) = self.pool.create_buffer(px, px, px * 4, wl_shm::Format::Argb8888) else { return };
        paint_dot(canvas, px as usize);
        let surface = layer.wl_surface();
        surface.set_buffer_scale(self.scale);
        surface.damage_buffer(0, 0, px, px);
        if buffer.attach_to(surface).is_ok() {
            layer.commit();
        }
    }
}

/// A red dot with a white rim in a soft red glow, premultiplied ARGB, [px] square.
fn paint_dot(canvas: &mut [u8], px: usize) {
    let c = px as f32 / 2.0;
    let unit = px as f32 / SIZE as f32;
    let (core, rim, glow) = (4.5 * unit, 6.0 * unit, c);
    for (i, p) in canvas.chunks_exact_mut(4).enumerate() {
        let (x, y) = ((i % px) as f32 + 0.5 - c, (i / px) as f32 + 0.5 - c);
        let d = (x * x + y * y).sqrt();
        // Edge coverage over one pixel, for smooth circles.
        let inside = |r: f32| (r - d + 0.5).clamp(0.0, 1.0);
        // Apple's system red, the colour a presenter's laser is expected to be.
        let (red, green, blue) = (1.0, 0.231, 0.188);
        let halo = ((1.0 - (d - rim).max(0.0) / (glow - rim)).clamp(0.0, 1.0)).powi(2) * 0.35;
        let (mut a, mut r, mut g, mut b) = (halo, red * halo, green * halo, blue * halo);
        let ring = inside(rim);
        (a, r, g, b) = (ring + a * (1.0 - ring), ring + r * (1.0 - ring), ring + g * (1.0 - ring), ring + b * (1.0 - ring));
        let dot = inside(core);
        (a, r, g, b) = (dot + a * (1.0 - dot), red * dot + r * (1.0 - dot), green * dot + g * (1.0 - dot), blue * dot + b * (1.0 - dot));
        let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        p.copy_from_slice(&[byte(b), byte(g), byte(r), byte(a)]);
    }
}

impl LayerShellHandler for Overlay {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.hide();
    }

    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface, _: LayerSurfaceConfigure, _: u32) {
        if !self.configured {
            self.configured = true;
            self.draw();
        }
    }
}

impl CompositorHandler for Overlay {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, factor: i32) {
        if factor.max(1) != self.scale {
            self.scale = factor.max(1);
            self.draw();
        }
    }

    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}

    /// The compositor put it on [output]: that screen's size is what -1..1 spans.
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, output: &wl_output::WlOutput) {
        if let Some(size) = self.outputs.info(output).and_then(|i| i.logical_size)
            && size != self.size
        {
            self.size = size;
            self.place();
        }
    }

    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for Overlay {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ShmHandler for Overlay {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_registry!(Overlay);

impl ProvidesRegistryState for Overlay {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState];
}

smithay_client_toolkit::delegate_dispatch2!(Overlay);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_like_kdeconnectd() {
        let screen = (1920, 1080);
        let s = Spot::CENTRE.moved(0.5, 0.1, screen);
        assert_eq!(s.x, 0.5);
        assert!((s.y - 0.1 * 1920.0 / 1080.0).abs() < 1e-9);
        assert_eq!(Spot::CENTRE.moved(5.0, -5.0, screen), Spot { x: 1.0, y: -1.0 });
        assert_eq!(Spot::CENTRE.corner(screen), (960 - SIZE / 2, 540 - SIZE / 2));
        assert_eq!(Spot { x: 1.0, y: -1.0 }.corner(screen), (1920 - SIZE, 0));
    }

    /// Shows the dot on this desktop for two seconds, drifting right: `cargo test -- --ignored shows_on`.
    #[test]
    #[ignore]
    fn shows_on_this_desktop() {
        let tx = start().expect("overlay");
        for _ in 0..40 {
            tx.send(Order::Move(0.01, 0.0)).unwrap();
            std::thread::sleep(Duration::from_millis(50));
        }
        tx.send(Order::Stop).unwrap();
        std::thread::sleep(Duration::from_millis(200));
    }

    #[test]
    fn the_dot_is_opaque_in_the_middle_and_clear_at_the_corners() {
        let px = SIZE as usize;
        let mut canvas = vec![0u8; px * px * 4];
        paint_dot(&mut canvas, px);
        let at = |x: usize, y: usize| &canvas[(y * px + x) * 4..][..4];
        assert_eq!(at(px / 2, px / 2)[3], 255);
        assert_eq!(at(0, 0)[3], 0);
        assert_eq!(number(&serde_json::json!("0.25")), Some(0.25));
    }
}
