//! The touchpad over Bluetooth, for every PC side: the phone asks over the link and gets a key,
//! then streams signed frames over the radio beside it (movement, clicks, scrolling). The radio only
//! needs a bonded device, so frames count only when signed with a key a touchpad handed out. Each
//! side performs the frames its own way (Linux: the portal, Windows: SendInput).

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use base64::Engine;
use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use tracing::info;

const BODY_LEN: usize = 17;
const TAG_LEN: usize = 16;
const FRAME_LEN: usize = BODY_LEN + TAG_LEN;
/// Every 32nd frame is echoed, so the phone can time the round trip.
const ECHO_EVERY: u32 = 32;
/// Left, right, middle.
pub const BUTTONS: usize = 3;
/// Must match BluetoothPointerChannel.SERVICE on the phone.
pub const SERVICE: &str = "6f1d3e52-8a47-4c1b-9d06-2be7a5c4f804";

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Frame {
    /// Nothing to do: it keeps the radio awake between strokes, and carries the echo.
    KeepAwake,
    Move(f64, f64),
    Scroll(f64, f64),
    /// 0 left, 1 right, 2 middle; pressed or let go.
    Button(usize, bool),
    /// The radio dropped: let go of what's held, and wait for it to come back.
    StreamEnded,
    /// The touchpad closed, or the link did.
    Close,
}

/// "VYP1", seq, kind, dx, dy (big-endian), then the HMAC-SHA256 of those cut to 16 bytes.
fn parse(d: &[u8; FRAME_LEN], key: &hmac::Key) -> Option<(u32, Frame)> {
    if &d[..4] != b"VYP1" {
        return None;
    }
    let tag = hmac::sign(key, &d[..BODY_LEN]);
    // Constant time, so the tag can't be guessed a byte at a time.
    if tag.as_ref()[..TAG_LEN].iter().zip(&d[BODY_LEN..]).fold(0u8, |acc, (a, b)| acc | (a ^ b)) != 0 {
        return None;
    }
    let seq = u32::from_be_bytes(d[4..8].try_into().ok()?);
    let dx = f32::from_be_bytes(d[9..13].try_into().ok()?) as f64;
    let dy = f32::from_be_bytes(d[13..17].try_into().ok()?) as f64;
    let frame = match d[8] {
        0 => Frame::KeepAwake,
        1 => Frame::Move(dx, dy),
        2 => Frame::Scroll(dx, dy),
        k @ (3 | 4) => {
            let button = dx as usize;
            if button >= BUTTONS {
                return None;
            }
            Frame::Button(button, k == 3)
        }
        _ => return None,
    };
    Some((seq, frame))
}

/// Touchpads open for a radio stream: their key, and where its frames go.
static OPEN: Mutex<Vec<(u64, hmac::Key, mpsc::Sender<Frame>)>> = Mutex::new(Vec::new());
static NEXT: AtomicU64 = AtomicU64::new(1);

/// A touchpad's place in the list radio streams look in; gone when dropped.
pub struct Registered(u64);

impl Drop for Registered {
    fn drop(&mut self) {
        OPEN.lock().unwrap().retain(|(id, _, _)| *id != self.0);
    }
}

/// Opens a touchpad whose frames go to [tx]: the key for the phone (base64), kept while the
/// returned guard lives.
pub fn open(tx: mpsc::Sender<Frame>) -> Option<(Registered, String)> {
    let mut raw = [0u8; 32];
    SystemRandom::new().fill(&mut raw).ok()?;
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    OPEN.lock().unwrap().push((id, hmac::Key::new(hmac::HMAC_SHA256, &raw), tx));
    Some((Registered(id), base64::engine::general_purpose::STANDARD.encode(raw)))
}

/// One radio stream; its first frame's key picks the touchpad.
pub async fn read_frames<S: AsyncRead + AsyncWrite + Unpin>(mut stream: S) {
    let mut buf = [0u8; FRAME_LEN];
    if stream.read_exact(&mut buf).await.is_err() {
        return;
    }
    let found = OPEN.lock().unwrap().iter().find_map(|(_, k, tx)| parse(&buf, k).map(|f| (f, k.clone(), tx.clone())));
    let Some((mut first, key, tx)) = found else { return };
    info!("touchpad input arriving over Bluetooth");
    let mut last = 0;
    loop {
        let (seq, frame) = first;
        if seq <= last {
            break;
        }
        last = seq;
        if seq % ECHO_EVERY == 0 {
            let _ = tokio::time::timeout(Duration::from_millis(50), stream.write_all(&seq.to_be_bytes())).await;
        }
        if tx.send(frame).await.is_err() {
            return;
        }
        if stream.read_exact(&mut buf).await.is_err() {
            break;
        }
        let Some(next) = parse(&buf, &key) else { break };
        first = next;
    }
    let _ = tx.send(Frame::StreamEnded).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> hmac::Key {
        hmac::Key::new(hmac::HMAC_SHA256, &[7u8; 32])
    }

    /// The same frame ProtocolTest builds on the phone's side.
    const SAMPLE: &str = "5659503100000007013fc00000c000000040121d619c28d79b0856686ed5a767b0";

    fn sample() -> [u8; FRAME_LEN] {
        let hex: Vec<u8> = (0..SAMPLE.len()).step_by(2).map(|i| u8::from_str_radix(&SAMPLE[i..i + 2], 16).unwrap()).collect();
        hex.try_into().unwrap()
    }

    #[test]
    fn reads_the_phones_frames() {
        assert_eq!(parse(&sample(), &key()), Some((7, Frame::Move(1.5, -2.0))));
    }

    #[test]
    fn refuses_anything_unsigned() {
        let mut bad = sample();
        bad[10] ^= 1;
        assert_eq!(parse(&bad, &key()), None);
        assert_eq!(parse(&sample(), &hmac::Key::new(hmac::HMAC_SHA256, &[8u8; 32])), None);
    }

    #[tokio::test]
    async fn a_stream_signed_with_an_open_key_reaches_its_touchpad() {
        let (tx, mut rx) = mpsc::channel(8);
        let (_open, key_b64) = open(tx).unwrap();
        let raw = base64::engine::general_purpose::STANDARD.decode(key_b64).unwrap();
        let key = hmac::Key::new(hmac::HMAC_SHA256, &raw);
        let frame = |seq: u32, kind: u8, dx: f32, dy: f32| {
            let mut f = Vec::from(*b"VYP1");
            f.extend(seq.to_be_bytes());
            f.push(kind);
            f.extend(dx.to_be_bytes());
            f.extend(dy.to_be_bytes());
            let tag = hmac::sign(&key, &f);
            f.extend(&tag.as_ref()[..TAG_LEN]);
            f
        };
        let (mut phone, pc) = tokio::io::duplex(1024);
        let reader = tokio::spawn(read_frames(pc));
        phone.write_all(&frame(1, 1, 3.0, 4.0)).await.unwrap();
        phone.write_all(&frame(2, 3, 1.0, 0.0)).await.unwrap();
        // A replayed frame ends the stream.
        phone.write_all(&frame(2, 4, 1.0, 0.0)).await.unwrap();
        assert_eq!(rx.recv().await, Some(Frame::Move(3.0, 4.0)));
        assert_eq!(rx.recv().await, Some(Frame::Button(1, true)));
        assert_eq!(rx.recv().await, Some(Frame::StreamEnded));
        reader.await.unwrap();
    }
}
