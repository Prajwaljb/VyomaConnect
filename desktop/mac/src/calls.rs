//! Calls on the phone, told on this Mac: who is ringing in a notification, a missed call left as
//! one, and Music and Spotify paused for the call and resumed after it (only what this paused).
//! Answering stays on the phone: a Mac can't be its headset. One task takes the phone's packets in
//! order, so a call that ends at once can't resume before its pause. Names and numbers never go to
//! the log.

use std::collections::HashMap;

use serde_json::Value;
use tokio::sync::mpsc;
use vyoma_core::telephony::{self, Call, Line, Steps};

use crate::{mac, media};

pub enum Order {
    Packet { id: String, phone: String, packet: Value },
    /// The phone's gone: nothing will say the call ended.
    Gone(String),
}

/// One phone's calls (core's [Line]) and what this Mac paused for them.
#[derive(Default)]
struct Phone {
    line: Line,
    paused: Vec<media::Player>,
}

impl Phone {
    async fn carry_out(&mut self, id: &str, steps: Steps, call: Option<(&Call, &str)>) {
        let (ringing, missed) = (format!("call-{id}"), format!("missed-call-{id}"));
        if steps.stop_ringing {
            mac::withdraw(&ringing);
        }
        if steps.clear_missed {
            mac::withdraw(&missed);
        }
        if let Some((call, phone)) = call {
            if steps.ring {
                mac::phone_notification(&ringing, "", &call.ringing_title(), &call.body(phone)).await;
            }
            if steps.missed {
                mac::phone_notification(&missed, "", &call.missed_title(), &call.body(phone)).await;
            }
        }
        if steps.pause {
            self.paused = media::pause_playing().await;
        }
        if steps.resume {
            media::resume(&std::mem::take(&mut self.paused)).await;
        }
    }
}

/// Starts the task that handles calls; it ends when the sender is dropped.
pub fn start() -> mpsc::UnboundedSender<Order> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut phones: HashMap<String, Phone> = HashMap::new();
        while let Some(order) = rx.recv().await {
            match order {
                Order::Packet { id, phone, packet } => {
                    let Some(call) = telephony::parse(&packet) else { continue };
                    tracing::info!("{id}: call {:?}{}", call.kind, if call.over { ", over" } else { "" });
                    let p = phones.entry(id.clone()).or_default();
                    let steps = p.line.on_call(&call);
                    p.carry_out(&id, steps, Some((&call, &phone))).await;
                }
                Order::Gone(id) => {
                    if let Some(p) = phones.get_mut(&id) {
                        let steps = p.line.gone();
                        p.carry_out(&id, steps, None).await;
                    }
                }
            }
        }
    });
    tx
}
