//! The touchpad over Bluetooth, for Wi-Fi that stalls: movement, clicks and scrolling beside the link,
//! which stays where it is because it also carries what Bluetooth can't (files). The phone asks over
//! the link and gets a key; the radio only needs a bonded device, so frames must be signed to count.
//!
//! Moves go through the same portal calls as the link's mousepad packets do in kdeconnectd, so the
//! cursor feels the same either way. Plasma asks once to allow control, then a token reopens it.

use std::time::Duration;

use serde_json::json;
use tokio::sync::mpsc;
use tracing::warn;
use vyoma_core::pointer::{self as radio, BUTTONS};

pub use vyoma_core::pointer::{Frame, read_frames};

use crate::link::Sender;
use crate::{packet, screen};

/// No scroll for this long: the fingers lifted.
const SCROLL_LIFT: Duration = Duration::from_millis(150);
/// BTN_LEFT, BTN_RIGHT, BTN_MIDDLE.
const CODES: [i32; BUTTONS] = [0x110, 0x111, 0x112];

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
    let Some((_registered, key)) = radio::open(tx) else { return };
    if sender.send(&packet("vyoma.pointer.ready", json!({ "key": key }))).await.is_err() {
        return;
    }
    let mut held = [false; BUTTONS];
    let mut moved = false;
    // As in the daemon's input.rs: a pause in scrolling tells the desktop the fingers lifted.
    let mut scrolling = false;
    let mut lock = crate::scroll::ScrollLock::default();
    loop {
        let frame = if scrolling {
            match tokio::time::timeout(SCROLL_LIFT, rx.recv()).await {
                Ok(f) => f,
                Err(_) => {
                    scrolling = false;
                    lock.reset();
                    let _ = session.axis_end().await;
                    continue;
                }
            }
        } else {
            rx.recv().await
        };
        let Some(frame) = frame else { break };
        let done = match frame {
            Frame::KeepAwake => Ok(()),
            Frame::Move(dx, dy) => {
                moved = true;
                session.relative(dx, dy).await
            }
            Frame::Scroll(dx, dy) => {
                scrolling = crate::desktop::scroll_needs_finish();
                match if scrolling { lock.feed(dx, dy) } else { Some((dx, dy)) } {
                    Some((dx, dy)) => session.axis(dx, dy).await,
                    None => Ok(()),
                }
            }
            Frame::Button(b, down) => {
                // The portal handles calls concurrently: a click mustn't land before its move.
                if std::mem::take(&mut moved) {
                    let _ = session.settle().await;
                }
                held[b] = down;
                session.button(CODES[b], down).await
            }
            Frame::StreamEnded | Frame::Close => {
                for (b, h) in held.iter_mut().enumerate() {
                    if std::mem::take(h) {
                        let _ = session.button(CODES[b], false).await;
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
