//! The phone as a game controller: a virtual pad (uinput) that games take for the real one, an Xbox
//! 360 pad (VID 045e PID 028e, as xpad reports it) or a DualSense (054c:0ce6, as hid-playstation
//! reports it over USB, so SDL's own mapping applies), driven by `vyoma.gamepad.state` packets from
//! the phone, with the game's rumble sent back as `vyoma.gamepad.rumble`. The phone's gyro can also
//! move a virtual mouse (aim in PC games), made the first time it moves.
//!
//! `vyoma.gamepad.state {b, lx, ly, rx, ry, lt, rt, kind, mx, my, sid, seq}`: `b` a button bitmask
//! (see [Button]), sticks -32768..32767 (y down, as evdev), triggers 0..255, `kind` "xbox" or
//! "dualsense", `mx`/`my` running totals of gyro-mouse pixels (wrapping), `sid` the phone's stream,
//! `seq` its count within it (older ones are dropped: UDP and the link can reorder). The phone sends
//! it on every change and each second while the controller is open; the pad appears with the first
//! one and is removed (every button let go) on `vyoma.gamepad.close`, after [IDLE] without one, or
//! when the link drops. `vyoma.gamepad.rumble {strong, weak, ms}`: magnitudes 0..65535; ms 0 = until
//! the next rumble.
//!
//! Low latency: once the pad is up the companion offers `vyoma.gamepad.udp {port, key}` (a UDP port
//! the KDE Connect firewall range allows, a fresh base64 32-byte key). The phone then sends each state
//! as one [DATAGRAM_LEN]-byte datagram, big-endian: "VYG1", sid u32, seq u32, b u32, lx ly rx ry i16,
//! lt rt u8, mx my i32, kind u8 (0 xbox, 1 dualsense), then HMAC-SHA256 of all that cut to 16 bytes.
//! The link keeps carrying the once-a-second state, so a lost datagram heals within a second.

use std::collections::HashMap;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use base64::Engine;
use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::{Value, json};
use tokio::io::unix::AsyncFd;
use tokio::net::UdpSocket;
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

/// The pad games see.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum Kind {
    #[default]
    Xbox,
    DualSense,
}

/// evdev key codes, in [Button] order (as the kernel's xpad driver reports a 360 pad).
const XBOX_KEYS: [u16; 11] = [
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

/// The same for a DualSense, as hid-playstation reports one: triangle is BTN_NORTH (0x133), square
/// BTN_WEST (0x134), the other way round from xpad's X/Y. L2/R2 also click as buttons.
const DUALSENSE_KEYS: [u16; 11] = [0x130, 0x131, 0x134, 0x133, 0x136, 0x137, 0x13a, 0x13b, 0x13c, 0x13d, 0x13e];
const BTN_TL2: u16 = 0x138;
const BTN_TR2: u16 = 0x139;

impl Kind {
    fn keys(self) -> &'static [u16; 11] {
        match self {
            Kind::Xbox => &XBOX_KEYS,
            Kind::DualSense => &DUALSENSE_KEYS,
        }
    }

    fn parse(s: &str) -> Kind {
        if s == "dualsense" { Kind::DualSense } else { Kind::Xbox }
    }

    /// A stick value as this pad reports it (a DualSense's sticks are 0..255, 128 the middle).
    fn stick(self, v: i32) -> i32 {
        match self {
            Kind::Xbox => v,
            Kind::DualSense => (v + 32768) >> 8,
        }
    }
}

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_REL: u16 = 0x02;
const EV_ABS: u16 = 0x03;
const REL_X: u16 = 0x00;
const REL_Y: u16 = 0x01;
const BTN_LEFT: u16 = 0x110;
const BTN_RIGHT: u16 = 0x111;
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
const UI_SET_RELBIT: libc::c_ulong = 0x4004_5566;
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
    pub kind: Kind,
    /// Running totals of gyro-mouse pixels.
    pub mx: i32,
    pub my: i32,
    /// Which stream and where in it; None from a phone that doesn't number its states.
    pub order: Option<(u32, u32)>,
}

/// Bytes in one UDP state (see the module docs): 35 of state, 16 of tag.
pub const DATAGRAM_LEN: usize = 51;
const BODY_LEN: usize = 35;
const TAG_LEN: usize = 16;

impl State {
    pub fn parse(body: &Value) -> State {
        let n = |k: &str, lo: i64, hi: i64| body[k].as_i64().unwrap_or(0).clamp(lo, hi) as i32;
        let order = match (body["sid"].as_u64(), body["seq"].as_u64()) {
            (Some(sid), Some(seq)) => Some((sid as u32, seq as u32)),
            _ => None,
        };
        State {
            buttons: body["b"].as_u64().unwrap_or(0) as u32,
            lx: n("lx", -32768, 32767),
            ly: n("ly", -32768, 32767),
            rx: n("rx", -32768, 32767),
            ry: n("ry", -32768, 32767),
            lt: n("lt", 0, 255),
            rt: n("rt", 0, 255),
            kind: Kind::parse(body["kind"].as_str().unwrap_or("xbox")),
            mx: body["mx"].as_i64().unwrap_or(0) as i32,
            my: body["my"].as_i64().unwrap_or(0) as i32,
            order,
        }
    }

    /// A UDP state, if [key] signed it.
    pub fn from_datagram(d: &[u8], key: &hmac::Key) -> Option<State> {
        if d.len() != DATAGRAM_LEN || &d[..4] != b"VYG1" {
            return None;
        }
        let tag = hmac::sign(key, &d[..BODY_LEN]);
        // Constant time, so the tag can't be guessed a byte at a time.
        if tag.as_ref()[..TAG_LEN].iter().zip(&d[BODY_LEN..]).fold(0u8, |acc, (a, b)| acc | (a ^ b)) != 0 {
            return None;
        }
        let u32_at = |i: usize| u32::from_be_bytes(d[i..i + 4].try_into().unwrap());
        let i16_at = |i: usize| i16::from_be_bytes(d[i..i + 2].try_into().unwrap()) as i32;
        Some(State {
            order: Some((u32_at(4), u32_at(8))),
            buttons: u32_at(12),
            lx: i16_at(16),
            ly: i16_at(18),
            rx: i16_at(20),
            ry: i16_at(22),
            lt: d[24] as i32,
            rt: d[25] as i32,
            mx: u32_at(26) as i32,
            my: u32_at(30) as i32,
            kind: if d[34] == 1 { Kind::DualSense } else { Kind::Xbox },
        })
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
    kind: Kind,
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

fn open_uinput() -> Result<OwnedFd> {
    let path = c"/dev/uinput";
    let raw = unsafe { libc::open(path.as_ptr(), libc::O_RDWR | libc::O_NONBLOCK | libc::O_CLOEXEC) };
    if raw < 0 {
        bail!("can't open /dev/uinput ({}): the user needs to be in the input group", std::io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

fn finish_setup(fd: &OwnedFd, id: InputId, name: &[u8], ff_effects_max: u32) -> Result<()> {
    let mut setup = UinputSetup { id, name: [0; 80], ff_effects_max };
    setup.name[..name.len()].copy_from_slice(name);
    ioctl_ptr(fd, UI_DEV_SETUP, &mut setup)?;
    ioctl_int(fd, UI_DEV_CREATE, 0)
}

fn write_events(fd: &OwnedFd, events: &[(u16, u16, i32)]) -> Result<()> {
    let mut buf: Vec<InputEvent> = events.iter().map(|&(kind, code, value)| InputEvent { sec: 0, usec: 0, kind, code, value }).collect();
    buf.push(InputEvent { sec: 0, usec: 0, kind: EV_SYN, code: 0, value: 0 });
    let bytes = buf.len() * size_of::<InputEvent>();
    let n = unsafe { libc::write(fd.as_raw_fd(), buf.as_ptr().cast(), bytes) };
    if n != bytes as isize {
        bail!("uinput write: {}", std::io::Error::last_os_error());
    }
    Ok(())
}

impl Pad {
    pub fn create(kind: Kind) -> Result<Pad> {
        let fd = open_uinput()?;
        ioctl_int(&fd, UI_SET_EVBIT, EV_KEY as i32)?;
        for &k in kind.keys() {
            ioctl_int(&fd, UI_SET_KEYBIT, k as i32)?;
        }
        if kind == Kind::DualSense {
            ioctl_int(&fd, UI_SET_KEYBIT, BTN_TL2 as i32)?;
            ioctl_int(&fd, UI_SET_KEYBIT, BTN_TR2 as i32)?;
        }
        ioctl_int(&fd, UI_SET_EVBIT, EV_ABS as i32)?;
        ioctl_int(&fd, UI_SET_EVBIT, EV_FF as i32)?;
        ioctl_int(&fd, UI_SET_FFBIT, FF_RUMBLE as i32)?;
        let (lo, hi, mid, fuzz, flat) = match kind {
            Kind::Xbox => (-32768, 32767, 0, 16, 128),
            Kind::DualSense => (0, 255, 128, 0, 0),
        };
        let axes: [(u16, i32, i32, i32, i32, i32); 8] = [
            (ABS_X, lo, hi, mid, fuzz, flat),
            (ABS_Y, lo, hi, mid, fuzz, flat),
            (ABS_RX, lo, hi, mid, fuzz, flat),
            (ABS_RY, lo, hi, mid, fuzz, flat),
            (ABS_Z, 0, 255, 0, 0, 0),
            (ABS_RZ, 0, 255, 0, 0, 0),
            (ABS_HAT0X, -1, 1, 0, 0, 0),
            (ABS_HAT0Y, -1, 1, 0, 0, 0),
        ];
        for (code, minimum, maximum, value, fuzz, flat) in axes {
            ioctl_int(&fd, UI_SET_ABSBIT, code as i32)?;
            let mut a = AbsSetup { code, _pad: 0, value, minimum, maximum, fuzz, flat, resolution: 0 };
            ioctl_ptr(&fd, UI_ABS_SETUP, &mut a)?;
        }
        match kind {
            // A wired 360 pad: what Steam, SDL and Proton all know.
            Kind::Xbox => finish_setup(&fd, InputId { bustype: 0x03, vendor: 0x045e, product: 0x028e, version: 0x0110 }, b"Vyoma Controller (Xbox 360)", 16)?,
            // A USB DualSense under hid-playstation (version 0x8111: SDL's mapping for exactly that).
            Kind::DualSense => finish_setup(&fd, InputId { bustype: 0x03, vendor: 0x054c, product: 0x0ce6, version: 0x8111 }, b"Vyoma Controller (DualSense)", 16)?,
        }
        // SAFETY: the AsyncFd owns the fd, so it's deregistered before the fd closes.
        let fd = unsafe { AsyncFd::register(fd) }.context("watch uinput")?;
        // Resting sticks at their middle (0..255 pads would otherwise start pushed hard up-left).
        let pad = Pad { fd, kind, last: State { kind, ..Default::default() }, effects: HashMap::new() };
        if kind == Kind::DualSense {
            write_events(pad.fd.get_ref(), &[(EV_ABS, ABS_X, 128), (EV_ABS, ABS_Y, 128), (EV_ABS, ABS_RX, 128), (EV_ABS, ABS_RY, 128)])?;
        }
        Ok(pad)
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// Sends what changed since the last state, as one report.
    pub fn apply(&mut self, s: State) -> Result<()> {
        let old = self.last;
        let k = self.kind;
        let mut ev = Vec::new();
        for (i, &key) in k.keys().iter().enumerate() {
            let (was, now) = (old.pressed(i as u32), s.pressed(i as u32));
            if was != now {
                ev.push((EV_KEY, key, now as i32));
            }
        }
        let (hx, hy) = s.hat();
        let (ox, oy) = old.hat();
        if k == Kind::DualSense {
            for (key, was, now) in [(BTN_TL2, old.lt > 0, s.lt > 0), (BTN_TR2, old.rt > 0, s.rt > 0)] {
                if was != now {
                    ev.push((EV_KEY, key, now as i32));
                }
            }
        }
        for (code, was, now) in [
            (ABS_X, k.stick(old.lx), k.stick(s.lx)), (ABS_Y, k.stick(old.ly), k.stick(s.ly)),
            (ABS_RX, k.stick(old.rx), k.stick(s.rx)), (ABS_RY, k.stick(old.ry), k.stick(s.ry)),
            (ABS_Z, old.lt, s.lt), (ABS_RZ, old.rt, s.rt), (ABS_HAT0X, ox, hx), (ABS_HAT0Y, oy, hy),
        ] {
            if was != now {
                ev.push((EV_ABS, code, now));
            }
        }
        if !ev.is_empty() {
            write_events(self.fd.get_ref(), &ev)?;
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
        let _ = self.apply(State { kind: self.kind, ..Default::default() });
        unsafe { libc::ioctl(self.fd.get_ref().as_raw_fd(), UI_DEV_DESTROY) };
    }
}

/// The gyro's virtual mouse (aiming in PC games, which take a mouse over a stick).
struct Mouse {
    fd: OwnedFd,
}

impl Mouse {
    fn create() -> Result<Mouse> {
        let fd = open_uinput()?;
        ioctl_int(&fd, UI_SET_EVBIT, EV_REL as i32)?;
        ioctl_int(&fd, UI_SET_RELBIT, REL_X as i32)?;
        ioctl_int(&fd, UI_SET_RELBIT, REL_Y as i32)?;
        // Buttons make libinput take it for a mouse; they're never pressed.
        ioctl_int(&fd, UI_SET_EVBIT, EV_KEY as i32)?;
        ioctl_int(&fd, UI_SET_KEYBIT, BTN_LEFT as i32)?;
        ioctl_int(&fd, UI_SET_KEYBIT, BTN_RIGHT as i32)?;
        finish_setup(&fd, InputId { bustype: 0x06, vendor: 0, product: 0, version: 1 }, b"Vyoma Controller (gyro mouse)", 0)?;
        Ok(Mouse { fd })
    }

    fn moved(&self, dx: i32, dy: i32) -> Result<()> {
        let mut ev = Vec::with_capacity(2);
        if dx != 0 {
            ev.push((EV_REL, REL_X, dx));
        }
        if dy != 0 {
            ev.push((EV_REL, REL_Y, dy));
        }
        if ev.is_empty() { Ok(()) } else { write_events(&self.fd, &ev) }
    }
}

impl Drop for Mouse {
    fn drop(&mut self) {
        unsafe { libc::ioctl(self.fd.as_raw_fd(), UI_DEV_DESTROY) };
    }
}

/// The UDP side channel: where states come in, and the key they're signed with.
struct Fast {
    socket: UdpSocket,
    key: hmac::Key,
}

impl Fast {
    /// A port in the range the KDE Connect firewall rule opens (UDP 1714..1764), above kdeconnectd's own.
    async fn open() -> Result<(Fast, u16, String)> {
        let mut last = None;
        for port in 1750..=1764u16 {
            match UdpSocket::bind(("0.0.0.0", port)).await {
                Ok(socket) => {
                    let mut raw = [0u8; 32];
                    SystemRandom::new().fill(&mut raw).map_err(|_| anyhow::anyhow!("no randomness"))?;
                    let key = hmac::Key::new(hmac::HMAC_SHA256, &raw);
                    return Ok((Fast { socket, key }, port, base64::engine::general_purpose::STANDARD.encode(raw)));
                }
                Err(e) => last = Some(e),
            }
        }
        Err(last.map(anyhow::Error::from).unwrap_or_else(|| anyhow::anyhow!("no port")))
    }

    async fn next(&self) -> State {
        let mut buf = [0u8; 64];
        loop {
            if let Ok(n) = self.socket.recv(&mut buf).await
                && let Some(s) = State::from_datagram(&buf[..n], &self.key)
            {
                return s;
            }
        }
    }
}

/// Keeps states in order: a newer stream replaces the old, an older count within one is dropped.
#[derive(Default)]
struct Order {
    last: Option<(u32, u32)>,
}

impl Order {
    fn fresh(&mut self, s: &State) -> bool {
        let Some((sid, seq)) = s.order else { return true };
        match self.last {
            Some((ls, lq)) if ls == sid && (seq.wrapping_sub(lq) as i32) <= 0 => false,
            _ => {
                self.last = Some((sid, seq));
                true
            }
        }
    }
}

pub enum Input {
    State(State),
    Close,
}

/// Runs the pad for one phone while it sends states; returns when it closes or goes quiet.
pub async fn run(mut input: mpsc::Receiver<Input>, sender: Sender, name: String) {
    let mut pad: Option<Pad> = None;
    let mut mouse: Option<Mouse> = None;
    let mut fast: Option<Fast> = None;
    let mut order = Order::default();
    // The gyro mouse's totals last seen (a fresh stream starts its own from anywhere).
    let mut moved: Option<(u32, i32, i32)> = None;
    loop {
        let rumble = async {
            match pad.as_mut() {
                Some(p) => p.rumble().await,
                None => std::future::pending().await,
            }
        };
        let datagram = async {
            match fast.as_ref() {
                Some(f) => f.next().await,
                None => std::future::pending().await,
            }
        };
        let state = tokio::select! {
            got = tokio::time::timeout(IDLE, input.recv()) => match got {
                Ok(Some(Input::State(s))) => s,
                Ok(Some(Input::Close)) | Ok(None) | Err(_) => {
                    if pad.take().is_some() {
                        info!("{name}: controller disconnected");
                    }
                    return;
                }
            },
            s = datagram => s,
            r = rumble => {
                match r {
                    Ok((strong, weak, ms)) => {
                        let _ = sender.send(&packet("vyoma.gamepad.rumble", json!({ "strong": strong, "weak": weak, "ms": ms }))).await;
                    }
                    Err(e) => {
                        warn!("{name}: controller feedback failed: {e:#}");
                        return;
                    }
                }
                continue;
            }
        };
        if !order.fresh(&state) {
            continue;
        }
        // A different pad asked for: the old one goes (letting go of everything) and the new one comes.
        if pad.as_ref().is_some_and(|p| p.kind() != state.kind) {
            pad = None;
        }
        if pad.is_none() {
            match Pad::create(state.kind) {
                Ok(p) => {
                    info!("{name}: controller connected ({:?})", state.kind);
                    pad = Some(p);
                }
                Err(e) => {
                    warn!("{name}: no virtual controller: {e:#}");
                    let _ = sender.send(&packet("vyoma.gamepad.rumble", json!({ "error": format!("{e:#}") }))).await;
                    return;
                }
            }
            if fast.is_none() {
                match Fast::open().await {
                    Ok((f, port, key)) => {
                        fast = Some(f);
                        let _ = sender.send(&packet("vyoma.gamepad.udp", json!({ "port": port, "key": key }))).await;
                    }
                    Err(e) => warn!("{name}: controller stays on the link (no UDP port: {e:#})"),
                }
            }
        }
        if let Some(p) = pad.as_mut() && let Err(e) = p.apply(state) {
            warn!("{name}: controller input failed: {e:#}");
        }
        let sid = state.order.map_or(0, |o| o.0);
        let (dx, dy) = match moved {
            Some((s, x, y)) if s == sid => (state.mx.wrapping_sub(x), state.my.wrapping_sub(y)),
            _ => (0, 0),
        };
        moved = Some((sid, state.mx, state.my));
        if dx != 0 || dy != 0 {
            if mouse.is_none() {
                match Mouse::create() {
                    Ok(m) => mouse = Some(m),
                    Err(e) => warn!("{name}: no gyro mouse: {e:#}"),
                }
            }
            if let Some(m) = &mouse && let Err(e) = m.moved(dx, dy) {
                warn!("{name}: gyro mouse failed: {e:#}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_signed_datagrams_only() {
        let key = hmac::Key::new(hmac::HMAC_SHA256, &[7u8; 32]);
        let mut d = Vec::new();
        d.extend_from_slice(b"VYG1");
        d.extend_from_slice(&5u32.to_be_bytes());
        d.extend_from_slice(&9u32.to_be_bytes());
        d.extend_from_slice(&3u32.to_be_bytes());
        for v in [-32768i16, 100, 0, 32767] {
            d.extend_from_slice(&v.to_be_bytes());
        }
        d.extend_from_slice(&[200, 0]);
        d.extend_from_slice(&(-4i32).to_be_bytes());
        d.extend_from_slice(&7i32.to_be_bytes());
        d.push(1);
        let tag = hmac::sign(&key, &d);
        d.extend_from_slice(&tag.as_ref()[..TAG_LEN]);
        let s = State::from_datagram(&d, &key).unwrap();
        assert_eq!((s.order, s.buttons, s.lx, s.ry, s.lt, s.mx, s.my, s.kind), (Some((5, 9)), 3, -32768, 32767, 200, -4, 7, Kind::DualSense));
        d[20] ^= 1;
        assert!(State::from_datagram(&d, &key).is_none());
    }

    #[test]
    fn drops_stale_states() {
        let mut o = Order::default();
        let at = |sid, seq| State { order: Some((sid, seq)), ..Default::default() };
        assert!(o.fresh(&at(1, 5)));
        assert!(!o.fresh(&at(1, 4)));
        assert!(!o.fresh(&at(1, 5)));
        assert!(o.fresh(&at(1, 6)));
        assert!(o.fresh(&at(2, 1)));
        assert!(o.fresh(&State::default()));
    }

    #[test]
    fn dualsense_sticks_rest_in_the_middle() {
        assert_eq!(Kind::DualSense.stick(0), 128);
        assert_eq!(Kind::DualSense.stick(-32768), 0);
        assert_eq!(Kind::DualSense.stick(32767), 255);
    }

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
        let mut p = Pad::create(Kind::Xbox).unwrap();
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
        let kind = Kind::parse(&std::env::var("PAD_KIND").unwrap_or_default());
        let mut p = Pad::create(kind).unwrap();
        // A (cross), left stick full right; a second later (SDL takes a trigger's first value as its
        // rest, so it has to move to count) the right trigger half in.
        p.apply(State { kind, buttons: 1, lx: 32767, ..Default::default() }).unwrap();
        tokio::time::sleep(Duration::from_millis(1500)).await;
        p.apply(State { kind, buttons: 1, lx: 32767, rt: 128, ..Default::default() }).unwrap();
        tokio::time::sleep(Duration::from_millis(std::env::var("PAD_HOLD_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(200))).await;
        drop(p);
    }
}
