//! Calls on the phone, told on this PC (the engine's call task): who is ringing in a toast with
//! Silence (the phone's ringer, the stock request) and Decline (where the phone lets this PC
//! answer for it), a missed call left as a toast until the phone clears it, and what's playing
//! paused for the call and resumed after it (only what this paused). Answering stays on the
//! phone: without a hands-free link the call's sound couldn't come here.

use serde_json::Value;
use tokio::sync::mpsc;
pub use vyoma_engine::calls::Order;
use vyoma_engine::calls::{self, Call, Ringer};

use crate::toast::{self, Button, Shown, Toast};
use crate::{media, tray};

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
        "mute" => Some(calls::mute_packet()),
        "decline" => Some(calls::request_packet("decline")),
        _ => None,
    }
}

/// A toast, or a balloon where there are none (not when toasts are only switched off).
fn tell(t: &Toast) {
    if toast::show(t) == Shown::Unavailable {
        tray::balloon(&t.title, &t.body);
    }
}

struct WinRinger;

impl Ringer for WinRinger {
    /// The paused players' app ids.
    type Paused = Vec<String>;

    async fn ring(&mut self, id: &str, phone: &str, call: &Call) {
        tell(&ringing_toast(id, phone, call));
    }

    async fn stop_ringing(&mut self, id: &str) {
        toast::remove(RINGING, &toast::short(id));
    }

    async fn missed(&mut self, id: &str, phone: &str, call: &Call) {
        tell(&missed_toast(id, phone, call));
    }

    async fn clear_missed(&mut self, id: &str) {
        toast::remove(MISSED, &toast::short(id));
    }

    async fn pause(&mut self) -> Vec<String> {
        tokio::task::spawn_blocking(media::pause_playing).await.unwrap_or_default()
    }

    async fn resume(&mut self, paused: Vec<String>) {
        if !paused.is_empty() {
            let _ = tokio::task::spawn_blocking(move || media::resume(&paused)).await;
        }
    }
}

/// Starts the task that handles calls; it ends when the sender is dropped.
pub fn start() -> mpsc::UnboundedSender<Order> {
    calls::start(WinRinger)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use vyoma_core::telephony;

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
