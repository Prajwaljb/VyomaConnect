//! Calls on the phone, told on this Mac (the engine's call task): who is ringing in a
//! notification, a missed call left as one, and Music and Spotify paused for the call and resumed
//! after it (only what this paused). Answering stays on the phone: a Mac can't be its headset.

use tokio::sync::mpsc;
pub use vyoma_engine::calls::Order;
use vyoma_engine::calls::{self, Call, Ringer};

use crate::{mac, media};

struct MacRinger;

impl Ringer for MacRinger {
    type Paused = Vec<media::Player>;

    async fn ring(&mut self, id: &str, phone: &str, call: &Call) {
        mac::phone_notification(&format!("call-{id}"), "", &call.ringing_title(), &call.body(phone)).await;
    }

    async fn stop_ringing(&mut self, id: &str) {
        mac::withdraw(&format!("call-{id}"));
    }

    async fn missed(&mut self, id: &str, phone: &str, call: &Call) {
        mac::phone_notification(&format!("missed-call-{id}"), "", &call.missed_title(), &call.body(phone)).await;
    }

    async fn clear_missed(&mut self, id: &str) {
        mac::withdraw(&format!("missed-call-{id}"));
    }

    async fn pause(&mut self) -> Vec<media::Player> {
        media::pause_playing().await
    }

    async fn resume(&mut self, paused: Vec<media::Player>) {
        media::resume(&paused).await;
    }
}

/// Starts the task that handles calls; it ends when the sender is dropped.
pub fn start() -> mpsc::UnboundedSender<Order> {
    calls::start(MacRinger)
}
