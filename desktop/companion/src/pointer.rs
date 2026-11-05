//! The touchpad over Bluetooth, for Wi-Fi that stalls: movement, clicks and scrolling beside the link,
//! which stays where it is because it also carries what Bluetooth can't (files). The phone asks over
//! the link and gets a key; the radio only needs a bonded device, so frames must be signed to count.
//!
//! Moves go through the same portal calls as the link's mousepad packets do in kdeconnectd, so the
//! cursor feels the same either way. Plasma asks once to allow control, then a token reopens it.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use base64::Engine;
use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::json;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::link::Sender;
use crate::{packet, screen};

const BODY_LEN: usize = 17;
const TAG_LEN: usize = 16;
const FRAME_LEN: usize = BODY_LEN + TAG_LEN;
/// Every 32nd frame is echoed, so the phone can time the round trip.
const ECHO_EVERY: u32 = 32;
/// BTN_LEFT, BTN_RIGHT, BTN_MIDDLE.
const BUTTONS: [i32; 3] = [0x110, 0x111, 0x112];

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Frame {
    /// Nothing to do: it keeps the radio awake between strokes, and carries the echo.
    KeepAwake,
    Move(f64, f64),
    Scroll(f64, f64),
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
            if button >= BUTTONS.len() {
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

struct Registered(u64);

impl Drop for Registered {
    fn drop(&mut self) {
        OPEN.lock().unwrap().retain(|(id, _, _)| *id != self.0);
    }
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

/// One touchpad: opens the portal, hands over the key, and performs the frames until [Frame::Close].
pub async fn serve(sender: Sender, name: String, tx: mpsc::Sender<Frame>, mut rx: mpsc::Receiver<Frame>) {
    let session = match screen::pointer_session().await {
        Ok(s) => s,
        Err(e) => {
            warn!("{name}: no touchpad over Bluetooth: {e:#}");
            let _ = sender.send(&packet("vyoma.pointer.ready", json!({ "error": format!("{e:#}") }))).await;
            return;
        }
    };
    let mut raw = [0u8; 32];
    if SystemRandom::new().fill(&mut raw).is_err() {
        return;
    }
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    OPEN.lock().unwrap().push((id, hmac::Key::new(hmac::HMAC_SHA256, &raw), tx));
    let _registered = Registered(id);
    let key = base64::engine::general_purpose::STANDARD.encode(raw);
    if sender.send(&packet("vyoma.pointer.ready", json!({ "key": key }))).await.is_err() {
        return;
    }
    let mut held = [false; BUTTONS.len()];
    let mut moved = false;
    while let Some(frame) = rx.recv().await {
        let done = match frame {
            Frame::KeepAwake => Ok(()),
            Frame::Move(dx, dy) => {
                moved = true;
                session.relative(dx, dy).await
            }
            Frame::Scroll(dx, dy) => session.axis(dx, dy).await,
            Frame::Button(b, down) => {
                // The portal handles calls concurrently: a click mustn't land before its move.
                if std::mem::take(&mut moved) {
                    let _ = session.settle().await;
                }
                held[b] = down;
                session.button(BUTTONS[b], down).await
            }
            Frame::StreamEnded | Frame::Close => {
                for (b, h) in held.iter_mut().enumerate() {
                    if std::mem::take(h) {
                        let _ = session.button(BUTTONS[b], false).await;
                    }
                }
                if frame == Frame::Close {
                    return;
                }
                Ok(())
            }
        };
        if let Err(e) = done {
            tracing::debug!("touchpad: {e:#}");
        }
    }
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
}
