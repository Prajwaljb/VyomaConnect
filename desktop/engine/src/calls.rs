//! Calls on the phone, told on the PC (`kdeconnect.telephony`): one task takes every phone's call
//! packets in order (core's [Line]), so a call that ends at once can't resume the music before
//! it was paused. Who is ringing is said once a call; the alert comes down when it's answered or
//! over; a missed call stays until the phone clears it; what's playing pauses for the call and
//! only what was paused resumes. How each is shown, and what pausing means, is the side's
//! ([Ringer]). Also the packets a PC sends about a call. Names and numbers never go to the log.
//!
//! Linux's daemon keeps a fuller line of its own (answering on the PC, the call's sound over
//! Bluetooth) on the same core parsing.

use std::collections::HashMap;
use std::future::Future;

use serde_json::{Value, json};
use tokio::sync::mpsc;
use vyoma_core::link::packet;
pub use vyoma_core::telephony::{Call, DIAL, DONE, Kind, Line, MUTE, REQUEST, Steps, TELEPHONY, parse};

/// What the call task is handed.
pub enum Order {
    Packet { id: String, phone: String, packet: Value },
    /// The phone's gone: nothing will say the call ended.
    Gone(String),
}

/// How a PC side tells of calls. [id] is the phone's, [phone] its name.
pub trait Ringer: Send + 'static {
    /// What a pause stopped, to start again.
    type Paused: Default + Send;
    fn ring(&mut self, id: &str, phone: &str, call: &Call) -> impl Future<Output = ()> + Send;
    fn stop_ringing(&mut self, id: &str) -> impl Future<Output = ()> + Send;
    fn missed(&mut self, id: &str, phone: &str, call: &Call) -> impl Future<Output = ()> + Send;
    fn clear_missed(&mut self, id: &str) -> impl Future<Output = ()> + Send;
    /// Pauses what's playing; what that was.
    fn pause(&mut self) -> impl Future<Output = Self::Paused> + Send;
    fn resume(&mut self, paused: Self::Paused) -> impl Future<Output = ()> + Send;
}

/// One phone's line and what was paused for it.
struct Phone<P> {
    line: Line,
    paused: P,
}

async fn carry_out<R: Ringer>(r: &mut R, p: &mut Phone<R::Paused>, id: &str, steps: Steps, call: Option<(&Call, &str)>) {
    if steps.stop_ringing {
        r.stop_ringing(id).await;
    }
    if steps.clear_missed {
        r.clear_missed(id).await;
    }
    if let Some((call, phone)) = call {
        if steps.ring {
            r.ring(id, phone, call).await;
        }
        if steps.missed {
            r.missed(id, phone, call).await;
        }
    }
    if steps.pause {
        p.paused = r.pause().await;
    }
    if steps.resume {
        r.resume(std::mem::take(&mut p.paused)).await;
    }
}

/// Starts the task that handles calls; it ends when the sender is dropped.
pub fn start<R: Ringer>(mut ringer: R) -> mpsc::UnboundedSender<Order> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut phones: HashMap<String, Phone<R::Paused>> = HashMap::new();
        while let Some(order) = rx.recv().await {
            match order {
                Order::Packet { id, phone, packet } => {
                    let Some(call) = parse(&packet) else { continue };
                    tracing::info!("{id}: call {:?}{}", call.kind, if call.over { ", over" } else { "" });
                    let p = phones.entry(id.clone()).or_insert_with(|| Phone { line: Line::default(), paused: R::Paused::default() });
                    let steps = p.line.on_call(&call);
                    carry_out(&mut ringer, p, &id, steps, Some((&call, &phone))).await;
                }
                Order::Gone(id) => {
                    if let Some(p) = phones.get_mut(&id) {
                        let steps = p.line.gone();
                        carry_out(&mut ringer, p, &id, steps, None).await;
                    }
                }
            }
        }
    });
    tx
}

/// Silences the phone's ringer (stock).
pub fn mute_packet() -> Value {
    packet(MUTE, json!({}))
}

/// Asks Vyoma's phone to answer, decline or hang up (`answer`, `decline`, `hangup`), which it does
/// only where its owner lets this PC.
pub fn request_packet(action: &str) -> Value {
    packet(REQUEST, json!({ "action": action }))
}

/// Asks the phone to call [number].
pub fn dial_packet(number: &str) -> Value {
    packet(DIAL, json!({ "number": number }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Log(Arc<Mutex<Vec<String>>>);

    impl Log {
        fn say(&self, s: impl Into<String>) {
            self.0.lock().unwrap().push(s.into());
        }
    }

    impl Ringer for Log {
        type Paused = Vec<String>;
        async fn ring(&mut self, id: &str, phone: &str, call: &Call) {
            self.say(format!("ring {id} {} / {}", call.ringing_title(), call.body(phone)));
        }
        async fn stop_ringing(&mut self, id: &str) {
            self.say(format!("stop {id}"));
        }
        async fn missed(&mut self, id: &str, _: &str, call: &Call) {
            self.say(format!("missed {id} {}", call.missed_title()));
        }
        async fn clear_missed(&mut self, id: &str) {
            self.say(format!("clear {id}"));
        }
        async fn pause(&mut self) -> Vec<String> {
            self.say("pause");
            vec!["player".into()]
        }
        async fn resume(&mut self, paused: Vec<String>) {
            self.say(format!("resume {}", paused.join(",")));
        }
    }

    fn call(event: &str, over: bool) -> Order {
        Order::Packet { id: "p1".into(), phone: "Pixel".into(), packet: packet(TELEPHONY, json!({ "event": event, "contactName": "Sam", "phoneNumber": "123", "isCancel": over })) }
    }

    #[tokio::test]
    async fn rings_once_pauses_and_gives_back() {
        let log = Log::default();
        let tx = start(log.clone());
        for o in [call("ringing", false), call("ringing", false), call("missedCall", false), call("missedCall", true), call("talking", false), Order::Gone("p1".into()), Order::Gone("p2".into())] {
            tx.send(o).unwrap();
        }
        drop(tx);
        for _ in 0..100 {
            if log.0.lock().unwrap().len() >= 10 {
                break;
            }
            tokio::task::yield_now().await;
        }
        let said = log.0.lock().unwrap().clone();
        assert_eq!(
            said,
            [
                "ring p1 Sam is calling / 123, on Pixel",
                "pause",
                "stop p1",
                "missed p1 Missed call from Sam",
                "resume player",
                "clear p1",
                "stop p1",
                "pause",
                "stop p1",
                "resume player",
            ]
        );
        assert_eq!((request_packet("decline")["body"]["action"].as_str(), mute_packet()["type"].as_str()), (Some("decline"), Some(MUTE)));
        assert_eq!(dial_packet("5")["body"]["number"], "5");
    }
}
