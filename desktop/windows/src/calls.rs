//! Calls on the phone, told on this PC: who is ringing in a toast with Silence (the phone's
//! ringer, the stock request) and Decline (where the phone lets this PC answer for it), a missed
//! call left as a toast until the phone clears it, and what's playing paused for the call and
//! resumed after it (only what this paused). Answering stays on the phone: without a hands-free
//! link the call's sound couldn't come here. One task takes the phone's packets in order (core's
//! [Line]), so a call that ends at once can't resume before its pause. Names and numbers never go
//! to the log.

use std::collections::HashMap;

use serde_json::{Value, json};
use tokio::sync::mpsc;
use vyoma_core::link::packet;
use vyoma_core::telephony::{self, Call, Line, Steps};

use crate::toast::{self, Button, Shown, Toast};
use crate::{media, tray};

pub enum Order {
    Packet { id: String, phone: String, packet: Value },
    /// The phone's gone: nothing will say the call ended.
    Gone(String),
}

const RINGING: &str = "call";
const MISSED: &str = "missed-call";

/// The ringing toast for a call on phone [id] (named [phone]).
pub fn ringing_toast(id: &str, phone: &str, call: &Call) -> Toast {
    let button = |label: &str, what: &str| Button { label: label.into(), args: toast::args(&[("do", "call"), ("dev", id), ("what", what)]), with_reply: false };
    let mut buttons = vec![button("Silence", "mute")];
    if call.answerable {
        buttons.push(button("Decline", "decline"));
    }
    Toast {
        tag: RINGING.into(),
        group: toast::short(id),
        title: call.ringing_title(),
        body: call.body(phone),
        launch: toast::args(&[("do", "open")]),
        buttons,
        urgent: true,
        ..Default::default()
    }
}

pub fn missed_toast(id: &str, phone: &str, call: &Call) -> Toast {
    Toast { tag: MISSED.into(), group: toast::short(id), title: call.missed_title(), body: call.body(phone), launch: toast::args(&[("do", "open")]), ..Default::default() }
}

/// What a ringing toast's button asks the phone.
pub fn request(what: &str) -> Option<Value> {
    match what {
        "mute" => Some(packet(telephony::MUTE, json!({}))),
        "decline" => Some(packet(telephony::REQUEST, json!({ "action": "decline" }))),
        _ => None,
    }
}

/// A toast, or a balloon where there are none (not when toasts are only switched off).
fn tell(t: &Toast) {
    if toast::show(t) == Shown::Unavailable {
        tray::balloon(&t.title, &t.body);
    }
}

/// One phone's calls (core's [Line]) and what this PC paused for them.
#[derive(Default)]
struct Phone {
    line: Line,
    paused: Vec<String>,
}

impl Phone {
    async fn carry_out(&mut self, id: &str, steps: Steps, call: Option<(&Call, &str)>) {
        let group = toast::short(id);
        if steps.stop_ringing {
            toast::remove(RINGING, &group);
        }
        if steps.clear_missed {
            toast::remove(MISSED, &group);
        }
        if let Some((call, phone)) = call {
            if steps.ring {
                tell(&ringing_toast(id, phone, call));
            }
            if steps.missed {
                tell(&missed_toast(id, phone, call));
            }
        }
        if steps.pause {
            self.paused = tokio::task::spawn_blocking(media::pause_playing).await.unwrap_or_default();
        }
        if steps.resume {
            let paused = std::mem::take(&mut self.paused);
            if !paused.is_empty() {
                let _ = tokio::task::spawn_blocking(move || media::resume(&paused)).await;
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn call(answerable: bool) -> Call {
        telephony::parse(&json!({ "type": telephony::TELEPHONY, "body": { "event": "ringing", "contactName": "Sam", "phoneNumber": "123", "vyomaAnswer": answerable } })).unwrap()
    }

    #[test]
    fn the_ringing_toast_offers_what_the_phone_allows() {
        let t = ringing_toast("phone1", "Pixel", &call(false));
        assert_eq!((t.title.as_str(), t.body.as_str(), t.urgent), ("Sam is calling", "123, on Pixel", true));
        assert_eq!(t.buttons.iter().map(|b| b.label.as_str()).collect::<Vec<_>>(), ["Silence"]);
        let t = ringing_toast("phone1", "Pixel", &call(true));
        assert_eq!(t.buttons.iter().map(|b| b.label.as_str()).collect::<Vec<_>>(), ["Silence", "Decline"]);
        let args = toast::parse_args(&t.buttons[1].args);
        assert_eq!((args["do"].as_str(), args["dev"].as_str(), args["what"].as_str()), ("call", "phone1", "decline"));
    }

    #[test]
    fn buttons_become_the_phones_requests() {
        assert_eq!(request("mute").unwrap()["type"], telephony::MUTE);
        let decline = request("decline").unwrap();
        assert_eq!((decline["type"].as_str(), decline["body"]["action"].as_str()), (Some(telephony::REQUEST), Some("decline")));
        assert!(request("answer").is_none(), "answering stays on the phone");
    }
}
