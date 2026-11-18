//! Calls on the phone, told on this Mac: who is ringing in a notification, a missed call left as
//! one, and Music and Spotify paused for the call and resumed after it (only what this paused).
//! Answering stays on the phone: a Mac can't be its headset. One task takes the phone's packets in
//! order, so a call that ends at once can't resume before its pause. Names and numbers never go to
//! the log.

use std::collections::HashMap;

use serde_json::Value;
use tokio::sync::mpsc;
use vyoma_core::telephony::{self, Call, Kind};

use crate::{mac, media};

pub enum Order {
    Packet { id: String, phone: String, packet: Value },
    /// The phone's gone: nothing will say the call ended.
    Gone(String),
}

#[derive(Default)]
struct Line {
    /// A call is on (ringing or talking): what's paused stays paused.
    on: bool,
    paused: Vec<media::Player>,
}

#[derive(Debug, Default, PartialEq)]
struct Steps {
    ring: bool,
    stop_ringing: bool,
    missed: bool,
    clear_missed: bool,
    pause: bool,
    resume: bool,
}

impl Line {
    fn on_call(&mut self, call: &Call) -> Steps {
        let mut s = Steps::default();
        match (call.kind, call.over) {
            (Kind::Ringing, false) => {
                // The phone can say it again: one alert a call.
                s.ring = !self.on;
                s.pause = !self.on;
                self.on = true;
            }
            (Kind::Talking, false) => {
                s.stop_ringing = true;
                s.pause = !self.on;
                self.on = true;
            }
            (Kind::Ringing | Kind::Talking, true) => self.end(&mut s),
            (Kind::Missed, false) => {
                s.missed = true;
                self.end(&mut s);
            }
            (Kind::Missed, true) => s.clear_missed = true,
        }
        s
    }

    fn end(&mut self, s: &mut Steps) {
        s.stop_ringing = true;
        s.resume = self.on;
        self.on = false;
    }

    fn gone(&mut self) -> Steps {
        let mut s = Steps::default();
        self.end(&mut s);
        s
    }

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
        let mut lines: HashMap<String, Line> = HashMap::new();
        while let Some(order) = rx.recv().await {
            match order {
                Order::Packet { id, phone, packet } => {
                    let Some(call) = telephony::parse(&packet) else { continue };
                    tracing::info!("{id}: call {:?}{}", call.kind, if call.over { ", over" } else { "" });
                    let line = lines.entry(id.clone()).or_default();
                    let steps = line.on_call(&call);
                    line.carry_out(&id, steps, Some((&call, &phone))).await;
                }
                Order::Gone(id) => {
                    if let Some(line) = lines.get_mut(&id) {
                        let steps = line.gone();
                        line.carry_out(&id, steps, None).await;
                    }
                }
            }
        }
    });
    tx
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(event: &str, over: bool) -> Call {
        telephony::parse(&json!({ "type": telephony::TELEPHONY, "body": { "event": event, "contactName": "A", "isCancel": over } })).unwrap()
    }

    #[test]
    fn a_call_rings_once_and_gives_the_music_back() {
        let mut line = Line::default();
        assert_eq!(line.on_call(&call("ringing", false)), Steps { ring: true, pause: true, ..Steps::default() });
        assert_eq!(line.on_call(&call("ringing", false)), Steps::default(), "said again: no second alert");
        assert_eq!(line.on_call(&call("talking", false)), Steps { stop_ringing: true, ..Steps::default() });
        assert_eq!(line.on_call(&call("talking", true)), Steps { stop_ringing: true, resume: true, ..Steps::default() });
    }

    #[test]
    fn a_missed_call_stays_until_the_phone_clears_it() {
        let mut line = Line::default();
        line.on_call(&call("ringing", false));
        assert_eq!(line.on_call(&call("missedCall", false)), Steps { missed: true, stop_ringing: true, resume: true, ..Steps::default() });
        assert_eq!(line.on_call(&call("missedCall", true)), Steps { clear_missed: true, ..Steps::default() });
    }

    #[test]
    fn a_phone_that_goes_mid_call_gives_the_music_back() {
        let mut line = Line::default();
        line.on_call(&call("talking", false));
        assert!(line.gone().resume);
        assert!(!line.gone().resume, "only once");
    }
}
