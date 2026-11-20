//! The pointer and keyboard on compositors that let a client be one (Hyprland, Sway, river,
//! Wayfire, labwc...): wlr's virtual pointer and the virtual keyboard protocol. They have no
//! RemoteDesktop portal, so this is how the phone's touchpad, keyboard and touches reach them.
//! Chosen by probing ([offered]): a compositor that offers both protocols gets them, whatever its
//! name; Plasma and GNOME don't, and keep the portal.
//!
//! The keyboard carries the desktop's own layout (xkbcommon compiles it from its names, as the
//! compositor does), so keys are the real keys that layout has, and modifiers are worked out by
//! xkbcommon from what's held. A symbol no key types goes on a keymap of its own for that one press,
//! as wtype does, then the layout comes back.

use std::ffi::{CStr, CString};
use std::io::Write;
use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::sync::Mutex;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_output::{self, WlOutput};
use wayland_client::protocol::wl_pointer::{Axis, AxisSource, ButtonState};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_region::WlRegion;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1;
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::{self, Anchor, KeyboardInteractivity, ZwlrLayerSurfaceV1};
use wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1;
use wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1;

use crate::mirror::{RuleNames, xkb_context_new, xkb_keymap_new_from_names};

const POINTER: &str = "zwlr_virtual_pointer_manager_v1";
const KEYBOARD: &str = "zwp_virtual_keyboard_manager_v1";
/// WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1.
const XKB_V1: u32 = 1;
/// xkbcommon's state components.
const MODS_DEPRESSED: libc::c_int = 1;
const MODS_LATCHED: libc::c_int = 2;
const MODS_LOCKED: libc::c_int = 4;
const LAYOUT_EFFECTIVE: libc::c_int = 0x80;
/// The key a lone symbol goes on: keycode 9 in its own keymap, evdev's 1.
const LONE_KEY: u32 = 1;

#[link(name = "xkbcommon")]
unsafe extern "C" {
    fn xkb_keymap_get_as_string(keymap: *mut libc::c_void, format: libc::c_int) -> *mut libc::c_char;
    fn xkb_state_new(keymap: *mut libc::c_void) -> *mut libc::c_void;
    fn xkb_state_update_key(state: *mut libc::c_void, key: u32, direction: libc::c_int) -> libc::c_int;
    fn xkb_state_serialize_mods(state: *mut libc::c_void, components: libc::c_int) -> u32;
    fn xkb_state_serialize_layout(state: *mut libc::c_void, components: libc::c_int) -> u32;
    fn xkb_keysym_get_name(keysym: u32, buffer: *mut libc::c_char, size: usize) -> libc::c_int;
}

/// Whether this session's compositor offers a virtual pointer and keyboard. Asked each time an
/// input session opens (cheap: one round trip), so it follows the session it's in.
pub fn offered() -> bool {
    if crate::x11::active() {
        return false;
    }
    globals().is_some_and(|names| names.iter().any(|n| n == POINTER) && names.iter().any(|n| n == KEYBOARD))
}

/// The protocols this session's Wayland compositor offers (one round trip); None off Wayland.
pub fn globals() -> Option<Vec<String>> {
    let conn = Connection::connect_to_env().ok()?;
    let (globals, _) = registry_queue_init::<State>(&conn).ok()?;
    Some(globals.contents().clone_list().into_iter().map(|g| g.interface).collect())
}

/// What the event queue keeps: the outputs' sizes, for touches on one screen.
#[derive(Default)]
struct State {
    outputs: Vec<(WlOutput, Option<(i32, i32)>)>,
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &WlRegistry, _: wayland_client::protocol::wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WlOutput, ()> for State {
    fn event(state: &mut Self, output: &WlOutput, event: wl_output::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_output::Event::Mode { flags, width, height, .. } = event
            && flags.into_result().is_ok_and(|f| f.contains(wl_output::Mode::Current))
            && let Some(o) = state.outputs.iter_mut().find(|(o, _)| o == output)
        {
            o.1 = Some((width, height));
        }
    }
}

macro_rules! ignore_events {
    ($($t:ty),*) => {$(
        impl Dispatch<$t, ()> for State {
            fn event(_: &mut Self, _: &$t, _: <$t as wayland_client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
        }
    )*};
}
ignore_events!(WlSeat, ZwlrVirtualPointerManagerV1, ZwlrVirtualPointerV1, ZwpVirtualKeyboardManagerV1, ZwpVirtualKeyboardV1);

/// The layout's keymap, compiled, and the state of what's held on it.
struct Keys {
    /// The keymap as text with its NUL, as the compositor reads it.
    text: Vec<u8>,
    state: *mut libc::c_void,
    /// Sent last: depressed, latched, locked, layout.
    mods: (u32, u32, u32, u32),
}

// SAFETY: the state is only touched behind Input's mutex.
unsafe impl Send for Keys {}

/// A virtual pointer and keyboard on this compositor.
pub struct Input {
    conn: Connection,
    pointer: ZwlrVirtualPointerV1,
    /// One whose absolute moves span a single output (the screen being cast), if one fits.
    on_output: Option<ZwlrVirtualPointerV1>,
    keyboard: ZwpVirtualKeyboardV1,
    keys: Mutex<Keys>,
    start: Instant,
}

impl Input {
    /// A pointer and keyboard; touches with [Input::absolute] land on the output [size] pixels
    /// big (the cast's), else across the whole desktop.
    pub fn open(size: Option<(i32, i32)>) -> Result<Self> {
        let conn = Connection::connect_to_env().context("no Wayland display")?;
        let (globals, mut queue) = registry_queue_init::<State>(&conn).context("the compositor didn't answer")?;
        let qh = queue.handle();
        let seat: WlSeat = globals.bind(&qh, 1..=7, ()).context("the compositor has no seat")?;
        let pointers: ZwlrVirtualPointerManagerV1 = globals.bind(&qh, 1..=2, ()).context("the compositor offers no virtual pointer")?;
        let keyboards: ZwpVirtualKeyboardManagerV1 = globals.bind(&qh, 1..=1, ()).context("the compositor offers no virtual keyboard")?;
        let mut state = State::default();
        for g in globals.contents().clone_list().into_iter().filter(|g| g.interface == "wl_output") {
            let output = globals.registry().bind::<WlOutput, _, _>(g.name, g.version.min(4), &qh, ());
            state.outputs.push((output, None));
        }
        queue.roundtrip(&mut state).context("the compositor didn't answer")?;
        let pointer = pointers.create_virtual_pointer(Some(&seat), &qh, ());
        let fits: Vec<&WlOutput> = state.outputs.iter().filter(|(_, s)| size.is_some() && *s == size).map(|(o, _)| o).collect();
        let only = (state.outputs.len() == 1).then(|| &state.outputs[0].0);
        let target = if fits.len() == 1 { Some(fits[0]) } else { only };
        let on_output = match target {
            Some(o) if pointers.version() >= 2 && size.is_some() => Some(pointers.create_virtual_pointer_with_output(Some(&seat), Some(o), &qh, ())),
            _ => None,
        };
        let keyboard = keyboards.create_virtual_keyboard(&seat, &qh, ());
        let keys = layout_keys()?;
        keyboard.keymap(XKB_V1, memfd(&keys.text)?.as_fd(), keys.text.len() as u32);
        conn.flush().context("the compositor went away")?;
        // Nothing comes back that matters; read it anyway so the socket never backs up.
        std::thread::Builder::new().name("vyoma-wlr".into()).spawn(move || while queue.blocking_dispatch(&mut state).is_ok() {})?;
        Ok(Self { conn, pointer, on_output, keyboard, keys: Mutex::new(keys), start: Instant::now() })
    }

    fn time(&self) -> u32 {
        self.start.elapsed().as_millis() as u32
    }

    fn flush(&self) -> Result<()> {
        self.conn.flush().context("the compositor went away")
    }

    pub fn relative(&self, dx: f64, dy: f64) -> Result<()> {
        self.pointer.motion(self.time(), dx, dy);
        self.pointer.frame();
        self.flush()
    }

    /// To ([x], [y]) of a [w]×[h] picture of the screen.
    pub fn absolute(&self, x: f64, y: f64, w: i32, h: i32) -> Result<()> {
        let p = self.on_output.as_ref().unwrap_or(&self.pointer);
        let (w, h) = (w.max(1) as u32, h.max(1) as u32);
        p.motion_absolute(self.time(), (x.max(0.0) as u32).min(w), (y.max(0.0) as u32).min(h), w, h);
        p.frame();
        self.flush()
    }

    pub fn button(&self, evdev: i32, down: bool) -> Result<()> {
        // Clicks go where the last move was, on whichever pointer made it.
        for p in std::iter::once(&self.pointer).chain(self.on_output.as_ref()) {
            p.button(self.time(), evdev as u32, if down { ButtonState::Pressed } else { ButtonState::Released });
            p.frame();
        }
        self.flush()
    }

    /// Smooth scrolling by fingers (positive: down, right), until [Input::scroll_end].
    pub fn scroll(&self, dx: f64, dy: f64) -> Result<()> {
        let t = self.time();
        self.pointer.axis_source(AxisSource::Finger);
        if dy != 0.0 {
            self.pointer.axis(t, Axis::VerticalScroll, dy);
        }
        if dx != 0.0 {
            self.pointer.axis(t, Axis::HorizontalScroll, dx);
        }
        self.pointer.frame();
        self.flush()
    }

    /// The fingers lifted: kinetic scrolling may follow in the app.
    pub fn scroll_end(&self) {
        let t = self.time();
        self.pointer.axis_source(AxisSource::Finger);
        self.pointer.axis_stop(t, Axis::VerticalScroll);
        self.pointer.axis_stop(t, Axis::HorizontalScroll);
        self.pointer.frame();
        let _ = self.flush();
    }

    /// A key of the layout, by its evdev code.
    pub fn key(&self, evdev: i32, down: bool) -> Result<()> {
        let mut keys = self.keys.lock().unwrap();
        self.keyboard.key(self.time(), evdev as u32, down as u32);
        // SAFETY: the state belongs to [keys], which this holds.
        let mods = unsafe {
            xkb_state_update_key(keys.state, evdev as u32 + 8, down as libc::c_int);
            (
                xkb_state_serialize_mods(keys.state, MODS_DEPRESSED),
                xkb_state_serialize_mods(keys.state, MODS_LATCHED),
                xkb_state_serialize_mods(keys.state, MODS_LOCKED),
                xkb_state_serialize_layout(keys.state, LAYOUT_EFFECTIVE),
            )
        };
        if mods != keys.mods {
            keys.mods = mods;
            self.keyboard.modifiers(mods.0, mods.1, mods.2, mods.3);
        }
        self.flush()
    }

    /// A symbol no key of the layout types: pressed on a keymap with only it, which goes again on release.
    pub fn keysym(&self, sym: u32, down: bool) -> Result<()> {
        let keys = self.keys.lock().unwrap();
        if down {
            let text = lone_keymap(sym)?;
            self.keyboard.keymap(XKB_V1, memfd(&text)?.as_fd(), text.len() as u32);
            self.keyboard.modifiers(0, 0, 0, 0);
            self.keyboard.key(self.time(), LONE_KEY, 1);
        } else {
            self.keyboard.key(self.time(), LONE_KEY, 0);
            self.keyboard.keymap(XKB_V1, memfd(&keys.text)?.as_fd(), keys.text.len() as u32);
            let m = keys.mods;
            self.keyboard.modifiers(m.0, m.1, m.2, m.3);
        }
        self.flush()
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        // Nothing stays held when the phone goes.
        self.keyboard.modifiers(0, 0, 0, 0);
        self.pointer.destroy();
        if let Some(p) = &self.on_output {
            p.destroy();
        }
        self.keyboard.destroy();
        let _ = self.conn.flush();
    }
}

/// The desktop's layout ([crate::desktop::keyboard_layout]) compiled by xkbcommon, as text.
fn layout_keys() -> Result<Keys> {
    let (name, variant) = crate::desktop::keyboard_layout();
    let (name, variant) = (CString::new(name)?, CString::new(variant)?);
    // SAFETY: xkbcommon's documented calls; the context and keymap live as long as this process
    // (one per input session, a few at most).
    unsafe {
        let ctx = xkb_context_new(0);
        if ctx.is_null() {
            bail!("xkbcommon has no context");
        }
        let names = RuleNames { rules: std::ptr::null(), model: std::ptr::null(), layout: name.as_ptr(), variant: variant.as_ptr(), options: std::ptr::null() };
        let km = xkb_keymap_new_from_names(ctx, &names, 0);
        if km.is_null() {
            bail!("xkbcommon couldn't compile the keyboard layout");
        }
        let raw = xkb_keymap_get_as_string(km, XKB_V1 as libc::c_int);
        if raw.is_null() {
            bail!("xkbcommon couldn't write the keyboard layout");
        }
        let text = CStr::from_ptr(raw).to_bytes_with_nul().to_vec();
        libc::free(raw.cast());
        let state = xkb_state_new(km);
        if state.is_null() {
            bail!("xkbcommon has no keyboard state");
        }
        Ok(Keys { text, state, mods: (0, 0, 0, 0) })
    }
}

/// A keymap with one key, which types [sym].
fn lone_keymap(sym: u32) -> Result<Vec<u8>> {
    let mut buf = [0 as libc::c_char; 64];
    // SAFETY: the buffer is as long as said.
    let n = unsafe { xkb_keysym_get_name(sym, buf.as_mut_ptr(), buf.len()) };
    if n <= 0 {
        bail!("no keysym {sym:#x}");
    }
    // SAFETY: xkbcommon wrote a NUL-terminated name.
    let name = unsafe { CStr::from_ptr(buf.as_ptr()) }.to_str()?.to_owned();
    Ok(lone_keymap_text(&name))
}

fn lone_keymap_text(name: &str) -> Vec<u8> {
    let text = format!(
        "xkb_keymap {{\n\
         xkb_keycodes \"vyoma\" {{ minimum = 8; maximum = 255; <K1> = 9; }};\n\
         xkb_types \"vyoma\" {{ include \"complete\" }};\n\
         xkb_compatibility \"vyoma\" {{ include \"complete\" }};\n\
         xkb_symbols \"vyoma\" {{ key <K1> {{ [ {name} ] }}; }};\n\
         }};\n"
    );
    let mut bytes = text.into_bytes();
    bytes.push(0);
    bytes
}

/// [bytes] in an anonymous file, as a keymap is handed over.
fn memfd(bytes: &[u8]) -> Result<OwnedFd> {
    // SAFETY: a new fd we own from here on.
    let fd = unsafe { libc::memfd_create(c"vyoma-keymap".as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        bail!("no memfd: {}", std::io::Error::last_os_error());
    }
    // SAFETY: just made, ours alone.
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };
    let mut file = std::fs::File::from(owned.try_clone()?);
    file.write_all(bytes)?;
    Ok(owned)
}

/// Marks every screen as changed without showing anything: a 1×1 transparent overlay that takes
/// no clicks, mapped for a moment on each output. Hyprland's ScreenCast portal (as wlr screencopy's
/// copy_with_damage) sends a picture only once part of the screen changes, so the phone's viewer
/// of a still desktop would stay empty until someone touched it.
pub fn touch_screens() -> Result<()> {
    let conn = Connection::connect_to_env().context("no Wayland display")?;
    let (globals, mut queue) = registry_queue_init::<Touched>(&conn)?;
    let qh = queue.handle();
    let compositor: WlCompositor = globals.bind(&qh, 1..=4, ()).context("no wl_compositor")?;
    let shm: WlShm = globals.bind(&qh, 1..=1, ()).context("no wl_shm")?;
    let layers: ZwlrLayerShellV1 = globals.bind(&qh, 1..=4, ()).context("no layer-shell")?;
    let outputs: Vec<WlOutput> = globals
        .contents()
        .clone_list()
        .into_iter()
        .filter(|g| g.interface == WlOutput::interface().name)
        .map(|g| globals.registry().bind(g.name, g.version.min(4), &qh, ()))
        .collect();
    let file = memfd(&[0; 4])?;
    let pool = shm.create_pool(file.as_fd(), 4, &qh, ());
    let buffer = pool.create_buffer(0, 1, 1, 4, wl_shm::Format::Argb8888, &qh, ());
    let nowhere = compositor.create_region(&qh, ());
    let mut state = Touched { buffer: Some(buffer.clone()), ..Default::default() };
    // No outputs listed: one overlay where the compositor puts it.
    let targets: Vec<Option<&WlOutput>> = if outputs.is_empty() { vec![None] } else { outputs.iter().map(Some).collect() };
    let mut overlays = Vec::new();
    for (i, output) in targets.into_iter().enumerate() {
        let surface = compositor.create_surface(&qh, ());
        surface.set_input_region(Some(&nowhere));
        let layer = layers.get_layer_surface(&surface, output, Layer::Overlay, "vyoma".into(), &qh, i);
        layer.set_size(1, 1);
        layer.set_anchor(Anchor::Top | Anchor::Left);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        surface.commit();
        state.surfaces.push(surface);
        overlays.push(layer);
    }
    // The configure comes back, the buffer goes on (Touched's dispatch), then the compositor has it.
    queue.roundtrip(&mut state)?;
    queue.roundtrip(&mut state)?;
    // Long enough for a frame to be drawn with it; taking it away again is a change too.
    std::thread::sleep(std::time::Duration::from_millis(50));
    for layer in overlays {
        layer.destroy();
    }
    for surface in &state.surfaces {
        surface.destroy();
    }
    buffer.destroy();
    pool.destroy();
    nowhere.destroy();
    queue.roundtrip(&mut state)?;
    Ok(())
}

/// [touch_screens]' event queue: the overlays, which take the buffer once configured.
#[derive(Default)]
struct Touched {
    surfaces: Vec<WlSurface>,
    buffer: Option<WlBuffer>,
}

impl Dispatch<ZwlrLayerSurfaceV1, usize> for Touched {
    fn event(state: &mut Self, layer: &ZwlrLayerSurfaceV1, event: zwlr_layer_surface_v1::Event, i: &usize, _: &Connection, _: &QueueHandle<Self>) {
        if let zwlr_layer_surface_v1::Event::Configure { serial, .. } = event
            && let Some(surface) = state.surfaces.get(*i)
        {
            layer.ack_configure(serial);
            surface.attach(state.buffer.as_ref(), 0, 0);
            surface.damage_buffer(0, 0, 1, 1);
            surface.commit();
        }
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for Touched {
    fn event(_: &mut Self, _: &WlRegistry, _: wayland_client::protocol::wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

macro_rules! touched_ignores {
    ($($t:ty),*) => {$(
        impl Dispatch<$t, ()> for Touched {
            fn event(_: &mut Self, _: &$t, _: <$t as wayland_client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
        }
    )*};
}
touched_ignores!(WlCompositor, WlShm, WlShmPool, WlBuffer, WlSurface, WlRegion, WlOutput, ZwlrLayerShellV1);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lone_symbol_gets_a_keymap_of_its_own() {
        let text = String::from_utf8(lone_keymap_text("U1F600")).unwrap();
        assert!(text.contains("key <K1> { [ U1F600 ] };"));
        assert!(text.ends_with('\0'));
    }

    /// Touches on the cast land where they're drawn: on the one output its size matches.
    #[test]
    fn touches_land_on_the_cast_screen() {
        let Ok(size) = std::env::var("VYOMA_WLR_OUTPUT") else { return };
        let (w, h) = size.split_once('x').map(|(w, h)| (w.parse().unwrap(), h.parse().unwrap())).unwrap();
        let input = Input::open(Some((w, h))).unwrap();
        assert!(input.on_output.is_some(), "a pointer for that output");
        input.absolute(w as f64 / 4.0, h as f64 / 2.0, w, h).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
    }

    /// Against a running compositor that offers both (a nested Hyprland in tests):
    /// `VYOMA_WLR_TEST=1 WAYLAND_DISPLAY=<its socket> cargo test --release --lib wlr::`.
    #[test]
    fn moves_the_pointer_and_types() {
        if std::env::var_os("VYOMA_WLR_TEST").is_none() {
            return;
        }
        assert!(offered(), "the compositor offers a virtual pointer and keyboard");
        let input = Input::open(None).unwrap();
        input.relative(40.0, 30.0).unwrap();
        input.key(30, true).unwrap(); // a
        input.key(30, false).unwrap();
        input.keysym(0x100_263a, true).unwrap(); // ☺, on no key
        input.keysym(0x100_263a, false).unwrap();
        input.key(42, true).unwrap(); // Shift+b: the modifier reaches the app
        input.key(48, true).unwrap();
        input.key(48, false).unwrap();
        input.key(42, false).unwrap();
        input.key(28, true).unwrap(); // Enter
        input.key(28, false).unwrap();
        input.scroll(0.0, 10.0).unwrap();
        input.scroll_end();
        // The compositor reads them in its own time; the devices go with [input].
        std::thread::sleep(std::time::Duration::from_millis(std::env::var("VYOMA_WLR_HOLD_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(500)));
    }
}
