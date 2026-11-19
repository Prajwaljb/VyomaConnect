//! The phone's game controller as it comes in, on any PC side: `vyoma.gamepad.state` packets, and
//! the signed frames of the fast routes (UDP, the USB cable, Bluetooth). What a state then does is
//! each platform's own (on Linux a virtual pad: the companion's gamepad.rs, which has the wire format).

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use base64::Engine;
use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::mpsc;
use tracing::{info, warn};

/// The state bitmask's bits (the phone's side of the wire format).
#[derive(Clone, Copy)]
#[repr(u32)]
pub enum Button {
    A = 0, B, X, Y, Lb, Rb, Back, Start, Guide, Ls, Rs, Up, Down, Left, Right,
}

/// The pad games see.
/// The pad the phone asks for.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum Kind {
    #[default]
    Xbox,
    DualSense,
}

impl Kind {
    pub fn parse(s: &str) -> Kind {
        if s == "dualsense" { Kind::DualSense } else { Kind::Xbox }
    }
}

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

    pub fn pressed(&self, b: u32) -> bool {
        self.buttons & (1 << b) != 0
    }

    pub fn hat(&self) -> (i32, i32) {
        let axis = |neg: Button, pos: Button| (self.pressed(pos as u32) as i32) - (self.pressed(neg as u32) as i32);
        (axis(Button::Left, Button::Right), axis(Button::Up, Button::Down))
    }
}

/// The UDP side channel: where states come in, and the key they're signed with; also where the
/// wired stream's states for this controller arrive.
pub struct Fast {
    socket: UdpSocket,
    key: hmac::Key,
    wired: mpsc::Receiver<State>,
    id: u64,
}

const WIRED_ADDR: (&str, u16) = ("127.0.0.1", 1766);

/// Controllers open for a wired stream: their key, and where its states go.
static WIRED: std::sync::Mutex<Vec<(u64, hmac::Key, mpsc::Sender<State>)>> = std::sync::Mutex::new(Vec::new());
static NEXT_WIRED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl Drop for Fast {
    fn drop(&mut self) {
        WIRED.lock().unwrap().retain(|(id, _, _)| *id != self.id);
    }
}

/// Takes wired controller streams (phones plugged in, through usb.rs's adb reverse).
pub async fn serve_wired() {
    let listener = match TcpListener::bind(WIRED_ADDR).await {
        Ok(l) => l,
        Err(e) => return warn!("no wired controller channel: {e}"),
    };
    // Anything on the phone or this PC can dial it: a few at once, and each must prove itself quickly.
    let slots = Arc::new(tokio::sync::Semaphore::new(WIRED_STREAMS));
    loop {
        let (stream, _) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(200)).await; // out of file descriptors
                continue;
            }
        };
        let Ok(slot) = slots.clone().try_acquire_owned() else { continue };
        let _ = stream.set_nodelay(true);
        // Every frame: echoes stopping is how the phone knows at once that the cable was pulled.
        tokio::spawn(async move {
            read_frames(stream, "the USB cable", 1).await;
            drop(slot);
        });
    }
}

const WIRED_STREAMS: usize = 8;
const FIRST_FRAME_WAIT: Duration = Duration::from_secs(2);

/// Every 32nd state is echoed, so the phone can time the round trip on this very route.
pub const ECHO_EVERY: u32 = 32;

fn echoed(state: &State, every: u32) -> Option<[u8; 4]> {
    state.order.filter(|(_, seq)| seq % every == 0).map(|(_, seq)| seq.to_be_bytes())
}

/// One stream of signed frames (the cable, or Bluetooth); the first frame's key picks the controller.
pub async fn read_frames<S: AsyncRead + AsyncWrite + Unpin>(mut stream: S, via: &'static str, echo_every: u32) {
    let mut buf = [0u8; DATAGRAM_LEN];
    if !matches!(tokio::time::timeout(FIRST_FRAME_WAIT, stream.read_exact(&mut buf)).await, Ok(Ok(_))) {
        return;
    }
    let found = WIRED.lock().unwrap().iter().find_map(|(_, k, tx)| State::from_datagram(&buf, k).map(|s| (s, k.clone(), tx.clone())));
    let Some((first, key, tx)) = found else { return };
    info!("controller input arriving over {via}");
    if tx.send(first).await.is_err() {
        return;
    }
    while stream.read_exact(&mut buf).await.is_ok() {
        let Some(s) = State::from_datagram(&buf, &key) else { return };
        if let Some(echo) = echoed(&s, echo_every) {
            // Never let a slow echo hold up the controller.
            let _ = tokio::time::timeout(Duration::from_millis(50), stream.write_all(&echo)).await;
        }
        if tx.send(s).await.is_err() {
            return;
        }
    }
}

impl Fast {
    /// A port in the range the KDE Connect firewall rule opens (UDP 1714..1764), above kdeconnectd's own.
    pub async fn open() -> Result<(Fast, u16, String)> {
        let mut last = None;
        for port in 1750..=1764u16 {
            match UdpSocket::bind(("0.0.0.0", port)).await {
                Ok(socket) => {
                    let mut raw = [0u8; 32];
                    SystemRandom::new().fill(&mut raw).map_err(|_| anyhow::anyhow!("no randomness"))?;
                    let key = hmac::Key::new(hmac::HMAC_SHA256, &raw);
                    let (tx, wired) = mpsc::channel(256);
                    let id = NEXT_WIRED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    WIRED.lock().unwrap().push((id, key.clone(), tx));
                    return Ok((Fast { socket, key, wired, id }, port, base64::engine::general_purpose::STANDARD.encode(raw)));
                }
                Err(e) => last = Some(e),
            }
        }
        Err(last.map(anyhow::Error::from).unwrap_or_else(|| anyhow::anyhow!("no port")))
    }

    /// The next state from either side channel, and whether it came over the cable.
    pub async fn next(&mut self) -> (State, bool) {
        let mut buf = [0u8; 64];
        loop {
            tokio::select! {
                got = self.socket.recv_from(&mut buf) => {
                    if let Ok((n, from)) = got && let Some(s) = State::from_datagram(&buf[..n], &self.key) {
                        if let Some(echo) = echoed(&s, ECHO_EVERY) {
                            let _ = self.socket.send_to(&echo, from).await;
                        }
                        return (s, false);
                    }
                }
                Some(s) = self.wired.recv() => return (s, true),
            }
        }
    }
}

/// Keeps states in order: a newer stream replaces the old, an older count within one is dropped.
#[derive(Default)]
pub struct Order {
    last: Option<(u32, u32)>,
}

impl Order {
    pub fn fresh(&mut self, s: &State) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_cable_echoes_every_frame_and_the_rest_every_32nd() {
        let at = |seq| State { order: Some((1, seq)), ..Default::default() };
        assert_eq!(echoed(&at(7), 1), Some(7u32.to_be_bytes()));
        assert_eq!(echoed(&at(7), ECHO_EVERY), None);
        assert_eq!(echoed(&at(64), ECHO_EVERY), Some(64u32.to_be_bytes()));
    }

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
    fn parses_and_clamps() {
        let s = State::parse(&json!({ "b": 1 | (1 << 11) | (1 << 14), "lx": 40000, "ly": -5, "lt": 300 }));
        assert_eq!(s.lx, 32767);
        assert_eq!(s.ly, -5);
        assert_eq!(s.lt, 255);
        assert!(s.pressed(Button::A as u32));
        assert_eq!(s.hat(), (1, -1));
    }
}
