//! The phone as a game controller: a virtual Xbox 360 pad (uinput, VID 045e PID 028e, so Steam,
//! SDL and games map it like the real one), driven by `vyoma.gamepad.state` packets from the phone,
//! with the game's rumble sent back as `vyoma.gamepad.rumble`.
//!
//! `vyoma.gamepad.state {b, lx, ly, rx, ry, lt, rt}`: `b` a button bitmask (see [Button]), sticks
//! -32768..32767 (y down, as evdev), triggers 0..255. The phone sends it on every change and resends
//! it each second while the controller is open; the pad appears with the first one and is removed
//! (every button let go) on `vyoma.gamepad.close`, after [IDLE] without one, or when the link drops.
//! `vyoma.gamepad.rumble {strong, weak, ms}`: magnitudes 0..65535; ms 0 = until the next rumble.

use std::collections::HashMap;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::io::unix::AsyncFd;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::link::Sender;
use crate::packet;

/// Without a state (or the heartbeat) for this long the phone has gone: let everything go.
const IDLE: Duration = Duration::from_secs(4);

/// The state bitmask's bits (the phone's side of the wire format; here only the D-pad's are named in code).
#[allow(dead_code)]
#[derive(Clone, Copy)]
#[repr(u32)]
pub enum Button {
    A = 0, B, X, Y, Lb, Rb, Back, Start, Guide, Ls, Rs, Up, Down, Left, Right,
}

/// evdev key codes, in [Button] order (as the kernel's xpad driver reports a 360 pad).
const KEYS: [u16; 11] = [
    0x130, // BTN_A
    0x131, // BTN_B
    0x133, // BTN_X
    0x134, // BTN_Y
    0x136, // BTN_TL
    0x137, // BTN_TR
    0x13a, // BTN_SELECT
    0x13b, // BTN_START
    0x13c, // BTN_MODE
    0x13d, // BTN_THUMBL
    0x13e, // BTN_THUMBR
];

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;
const EV_FF: u16 = 0x15;
const EV_UINPUT: u16 = 0x0101;
const UI_FF_UPLOAD: u16 = 1;
const UI_FF_ERASE: u16 = 2;
const FF_RUMBLE: u16 = 0x50;

const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const ABS_Z: u16 = 0x02;
const ABS_RX: u16 = 0x03;
const ABS_RY: u16 = 0x04;
const ABS_RZ: u16 = 0x05;
const ABS_HAT0X: u16 = 0x10;
const ABS_HAT0Y: u16 = 0x11;

// ioctls from linux/uinput.h (x86_64 / aarch64 layouts).
const UI_DEV_CREATE: libc::c_ulong = 0x5501;
const UI_DEV_DESTROY: libc::c_ulong = 0x5502;
const UI_DEV_SETUP: libc::c_ulong = 0x405c_5503;
const UI_ABS_SETUP: libc::c_ulong = 0x401c_5504;
const UI_SET_EVBIT: libc::c_ulong = 0x4004_5564;
const UI_SET_KEYBIT: libc::c_ulong = 0x4004_5565;
const UI_SET_ABSBIT: libc::c_ulong = 0x4004_5567;
const UI_SET_FFBIT: libc::c_ulong = 0x4004_556b;
const UI_BEGIN_FF_UPLOAD: libc::c_ulong = 0xc068_55c8;
const UI_END_FF_UPLOAD: libc::c_ulong = 0x4068_55c9;
const UI_BEGIN_FF_ERASE: libc::c_ulong = 0xc00c_55ca;
const UI_END_FF_ERASE: libc::c_ulong = 0x400c_55cb;

#[repr(C)]
struct InputId {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
}

#[repr(C)]
struct UinputSetup {
    id: InputId,
    name: [u8; 80],
    ff_effects_max: u32,
}

#[repr(C)]
struct AbsSetup {
    code: u16,
    _pad: u16,
    value: i32,
    minimum: i32,
    maximum: i32,
    fuzz: i32,
    flat: i32,
    resolution: i32,
}

/// struct ff_effect: 16 bytes of header, then a 32-byte union (a rumble is its first two u16s).
#[repr(C, align(8))]
#[derive(Clone, Copy)]
struct FfEffect {
    kind: u16,
    id: i16,
    direction: u16,
    trigger: [u16; 2],
    replay_length: u16,
    replay_delay: u16,
    _pad: u16,
    u: [u16; 16],
}

#[repr(C)]
struct FfUpload {
    request_id: u32,
    retval: i32,
    effect: FfEffect,
    old: FfEffect,
}

#[repr(C)]
struct FfErase {
    request_id: u32,
    retval: i32,
    effect_id: u32,
}

#[repr(C)]
struct InputEvent {
    sec: i64,
    usec: i64,
    kind: u16,
    code: u16,
    value: i32,
}

const _: () = assert!(size_of::<UinputSetup>() == 92);
const _: () = assert!(size_of::<AbsSetup>() == 28);
const _: () = assert!(size_of::<FfEffect>() == 48);
const _: () = assert!(size_of::<FfUpload>() == 104);
const _: () = assert!(size_of::<FfErase>() == 12);
const _: () = assert!(size_of::<InputEvent>() == 24);

/// What the phone's controller looks like right now.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct State {
    pub buttons: u32,
    pub lx: i32,
    pub ly: i32,
    pub rx: i32,
    pub ry: i32,
    pub lt: i32,
    pub rt: i32,
}

impl State {
    pub fn parse(body: &Value) -> State {
        let n = |k: &str, lo: i64, hi: i64| body[k].as_i64().unwrap_or(0).clamp(lo, hi) as i32;
        State {
            buttons: body["b"].as_u64().unwrap_or(0) as u32,
            lx: n("lx", -32768, 32767),
            ly: n("ly", -32768, 32767),
            rx: n("rx", -32768, 32767),
            ry: n("ry", -32768, 32767),
            lt: n("lt", 0, 255),
            rt: n("rt", 0, 255),
        }
    }

    fn pressed(&self, b: u32) -> bool {
        self.buttons & (1 << b) != 0
    }

    fn hat(&self) -> (i32, i32) {
        let axis = |neg: Button, pos: Button| (self.pressed(pos as u32) as i32) - (self.pressed(neg as u32) as i32);
        (axis(Button::Left, Button::Right), axis(Button::Up, Button::Down))
    }
}

/// The virtual pad. Dropping it removes it from the PC.
pub struct Pad {
    fd: AsyncFd<OwnedFd>,
    last: State,
    effects: HashMap<i16, (u16, u16, u16)>,
}

fn ioctl_int(fd: &OwnedFd, req: libc::c_ulong, v: libc::c_int) -> Result<()> {
    if unsafe { libc::ioctl(fd.as_raw_fd(), req, v) } < 0 {
        bail!("uinput ioctl {req:#x}: {}", std::io::Error::last_os_error());
    }
    Ok(())
}

fn ioctl_ptr<T>(fd: &OwnedFd, req: libc::c_ulong, v: *mut T) -> Result<()> {
    if unsafe { libc::ioctl(fd.as_raw_fd(), req, v) } < 0 {
        bail!("uinput ioctl {req:#x}: {}", std::io::Error::last_os_error());
    }
    Ok(())
}

impl Pad {
    pub fn create() -> Result<Pad> {
        let path = c"/dev/uinput";
        let raw = unsafe { libc::open(path.as_ptr(), libc::O_RDWR | libc::O_NONBLOCK | libc::O_CLOEXEC) };
        if raw < 0 {
            bail!("can't open /dev/uinput ({}): the user needs to be in the input group", std::io::Error::last_os_error());
        }
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        ioctl_int(&fd, UI_SET_EVBIT, EV_KEY as i32)?;
        for k in KEYS {
            ioctl_int(&fd, UI_SET_KEYBIT, k as i32)?;
        }
        ioctl_int(&fd, UI_SET_EVBIT, EV_ABS as i32)?;
        ioctl_int(&fd, UI_SET_EVBIT, EV_FF as i32)?;
        ioctl_int(&fd, UI_SET_FFBIT, FF_RUMBLE as i32)?;
        let axes: [(u16, i32, i32, i32, i32); 8] = [
            (ABS_X, -32768, 32767, 16, 128),
            (ABS_Y, -32768, 32767, 16, 128),
            (ABS_RX, -32768, 32767, 16, 128),
            (ABS_RY, -32768, 32767, 16, 128),
            (ABS_Z, 0, 255, 0, 0),
            (ABS_RZ, 0, 255, 0, 0),
            (ABS_HAT0X, -1, 1, 0, 0),
            (ABS_HAT0Y, -1, 1, 0, 0),
        ];
        for (code, minimum, maximum, fuzz, flat) in axes {
            ioctl_int(&fd, UI_SET_ABSBIT, code as i32)?;
            let mut a = AbsSetup { code, _pad: 0, value: 0, minimum, maximum, fuzz, flat, resolution: 0 };
            ioctl_ptr(&fd, UI_ABS_SETUP, &mut a)?;
        }
        let mut setup = UinputSetup {
            // A wired 360 pad: what Steam, SDL and Proton all know.
            id: InputId { bustype: 0x03, vendor: 0x045e, product: 0x028e, version: 0x0110 },
            name: [0; 80],
            ff_effects_max: 16,
        };
        let name = b"Vyoma Controller (Xbox 360)";
        setup.name[..name.len()].copy_from_slice(name);
        ioctl_ptr(&fd, UI_DEV_SETUP, &mut setup)?;
        ioctl_int(&fd, UI_DEV_CREATE, 0)?;
        // SAFETY: the AsyncFd owns the fd, so it's deregistered before the fd closes.
        let fd = unsafe { AsyncFd::register(fd) }.context("watch uinput")?;
        Ok(Pad { fd, last: State::default(), effects: HashMap::new() })
    }

    fn write(&self, events: &[(u16, u16, i32)]) -> Result<()> {
        let mut buf: Vec<InputEvent> = events.iter().map(|&(kind, code, value)| InputEvent { sec: 0, usec: 0, kind, code, value }).collect();
        buf.push(InputEvent { sec: 0, usec: 0, kind: EV_SYN, code: 0, value: 0 });
        let bytes = buf.len() * size_of::<InputEvent>();
        let n = unsafe { libc::write(self.fd.get_ref().as_raw_fd(), buf.as_ptr().cast(), bytes) };
        if n != bytes as isize {
            bail!("uinput write: {}", std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// Sends what changed since the last state, as one report.
    pub fn apply(&mut self, s: State) -> Result<()> {
        let old = self.last;
        let mut ev = Vec::new();
        for (i, &key) in KEYS.iter().enumerate() {
            let (was, now) = (old.pressed(i as u32), s.pressed(i as u32));
            if was != now {
                ev.push((EV_KEY, key, now as i32));
            }
        }
        let (hx, hy) = s.hat();
        let (ox, oy) = old.hat();
        for (code, was, now) in [
            (ABS_X, old.lx, s.lx), (ABS_Y, old.ly, s.ly), (ABS_RX, old.rx, s.rx), (ABS_RY, old.ry, s.ry),
            (ABS_Z, old.lt, s.lt), (ABS_RZ, old.rt, s.rt), (ABS_HAT0X, ox, hx), (ABS_HAT0Y, oy, hy),
        ] {
            if was != now {
                ev.push((EV_ABS, code, now));
            }
        }
        if !ev.is_empty() {
            self.write(&ev)?;
        }
        self.last = s;
        Ok(())
    }

    /// Waits for the game to play (or stop) a rumble; handles effect uploads on the way.
    pub async fn rumble(&mut self) -> Result<(u16, u16, u16)> {
        loop {
            let mut guard = self.fd.readable().await?;
            let mut ev = InputEvent { sec: 0, usec: 0, kind: 0, code: 0, value: 0 };
            let n = unsafe { libc::read(guard.get_inner().as_raw_fd(), (&mut ev as *mut InputEvent).cast(), size_of::<InputEvent>()) };
            if n < 0 {
                let e = std::io::Error::last_os_error();
                if e.kind() == std::io::ErrorKind::WouldBlock {
                    guard.clear_ready();
                    continue;
                }
                bail!("uinput read: {e}");
            }
            drop(guard);
            match (ev.kind, ev.code) {
                (EV_UINPUT, UI_FF_UPLOAD) => {
                    let mut up: FfUpload = unsafe { std::mem::zeroed() };
                    up.request_id = ev.value as u32;
                    ioctl_ptr(self.fd.get_ref(), UI_BEGIN_FF_UPLOAD, &mut up)?;
                    if up.effect.kind == FF_RUMBLE {
                        self.effects.insert(up.effect.id, (up.effect.u[0], up.effect.u[1], up.effect.replay_length));
                    }
                    up.retval = 0;
                    ioctl_ptr(self.fd.get_ref(), UI_END_FF_UPLOAD, &mut up)?;
                }
                (EV_UINPUT, UI_FF_ERASE) => {
                    let mut er = FfErase { request_id: ev.value as u32, retval: 0, effect_id: 0 };
                    ioctl_ptr(self.fd.get_ref(), UI_BEGIN_FF_ERASE, &mut er)?;
                    self.effects.remove(&(er.effect_id as i16));
                    er.retval = 0;
                    ioctl_ptr(self.fd.get_ref(), UI_END_FF_ERASE, &mut er)?;
                }
                (EV_FF, id) => {
                    if ev.value > 0 {
                        if let Some(&e) = self.effects.get(&(id as i16)) {
                            return Ok(e);
                        }
                    } else {
                        return Ok((0, 0, 0));
                    }
                }
                _ => {}
            }
        }
    }
}

impl Drop for Pad {
    fn drop(&mut self) {
        // Let go of everything first, so nothing stays held in a game.
        let _ = self.apply(State::default());
        unsafe { libc::ioctl(self.fd.get_ref().as_raw_fd(), UI_DEV_DESTROY) };
    }
}

pub enum Input {
    State(State),
    Close,
}

/// Runs the pad for one phone while it sends states; returns when it closes or goes quiet.
pub async fn run(mut input: mpsc::Receiver<Input>, sender: Sender, name: String) {
    let mut pad: Option<Pad> = None;
    loop {
        let next = async {
            match pad.as_mut() {
                Some(p) => p.rumble().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            got = tokio::time::timeout(IDLE, input.recv()) => match got {
                Ok(Some(Input::State(s))) => {
                    if pad.is_none() {
                        match Pad::create() {
                            Ok(p) => {
                                info!("{name}: controller connected");
                                pad = Some(p);
                            }
                            Err(e) => {
                                warn!("{name}: no virtual controller: {e:#}");
                                let _ = sender.send(&packet("vyoma.gamepad.rumble", json!({ "error": format!("{e:#}") }))).await;
                                return;
                            }
                        }
                    }
                    if let Some(p) = pad.as_mut() && let Err(e) = p.apply(s) {
                        warn!("{name}: controller input failed: {e:#}");
                    }
                }
                Ok(Some(Input::Close)) | Ok(None) | Err(_) => {
                    if pad.take().is_some() {
                        info!("{name}: controller disconnected");
                    }
                    return;
                }
            },
            r = next => match r {
                Ok((strong, weak, ms)) => {
                    let _ = sender.send(&packet("vyoma.gamepad.rumble", json!({ "strong": strong, "weak": weak, "ms": ms }))).await;
                }
                Err(e) => {
                    warn!("{name}: controller feedback failed: {e:#}");
                    return;
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_clamps() {
        let s = State::parse(&json!({ "b": 1 | (1 << 11) | (1 << 14), "lx": 40000, "ly": -5, "lt": 300 }));
        assert_eq!(s.lx, 32767);
        assert_eq!(s.ly, -5);
        assert_eq!(s.lt, 255);
        assert!(s.pressed(Button::A as u32));
        assert_eq!(s.hat(), (1, -1));
    }

    /// A game uploads a rumble and plays it (as SDL does, through the pad's event node): the pad
    /// hands back its strength and length. Needs /dev/uinput and the event node to be writable.
    #[tokio::test]
    #[ignore]
    async fn passes_rumble_back() {
        let mut p = Pad::create().unwrap();
        // This pad's own event node (another test may have a pad of the same name up): UI_GET_SYSNAME.
        let mut sys = [0u8; 64];
        assert!(unsafe { libc::ioctl(p.fd.get_ref().as_raw_fd(), 0x8040_552c as libc::c_ulong, sys.as_mut_ptr()) } >= 0);
        let sys = std::ffi::CStr::from_bytes_until_nul(&sys).unwrap().to_str().unwrap().to_owned();
        let node = (0..20)
            .find_map(|_| {
                std::thread::sleep(Duration::from_millis(50));
                std::fs::read_dir(format!("/sys/class/input/{sys}")).ok()?.flatten().find_map(|e| {
                    let n = e.file_name().into_string().ok()?;
                    n.starts_with("event").then(|| format!("/dev/input/{n}"))
                })
            })
            .expect("event node");
        let game = std::thread::spawn(move || {
            let path = std::ffi::CString::new(node).unwrap();
            let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDWR) };
            assert!(fd >= 0, "open event node: {}", std::io::Error::last_os_error());
            let mut e: FfEffect = unsafe { std::mem::zeroed() };
            e.kind = FF_RUMBLE;
            e.id = -1;
            e.replay_length = 250;
            e.u[0] = 40000;
            e.u[1] = 12000;
            // EVIOCSFF: blocks until the pad's side accepts the upload.
            assert!(unsafe { libc::ioctl(fd, 0x4030_4580 as libc::c_ulong, &mut e) } >= 0, "upload: {}", std::io::Error::last_os_error());
            let play = InputEvent { sec: 0, usec: 0, kind: EV_FF, code: e.id as u16, value: 1 };
            unsafe { libc::write(fd, (&play as *const InputEvent).cast(), size_of::<InputEvent>()) };
            std::thread::sleep(Duration::from_millis(300));
            unsafe { libc::close(fd) };
        });
        let got = tokio::time::timeout(Duration::from_secs(3), p.rumble()).await.expect("rumble in time").unwrap();
        assert_eq!(got, (40000, 12000, 250));
        // Closing the node erases the effect, which waits for the pad to answer: keep answering.
        while !game.is_finished() {
            let _ = tokio::time::timeout(Duration::from_millis(100), p.rumble()).await;
        }
        game.join().unwrap();
    }

    /// Makes a real virtual pad, presses A, lets go. Needs write access to /dev/uinput.
    #[tokio::test]
    #[ignore]
    async fn creates_a_pad() {
        let mut p = Pad::create().unwrap();
        p.apply(State { buttons: 1, lx: 1000, ..Default::default() }).unwrap();
        tokio::time::sleep(Duration::from_millis(std::env::var("PAD_HOLD_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(200))).await;
        drop(p);
    }
}
