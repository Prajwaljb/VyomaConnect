//! Which part of a PC side a phone's packet goes to, once for every side: each side's loop matches
//! on [Route] (exhaustively, so a plugin added here can't go unhandled on one OS) and keeps only
//! how its own computer does the thing. And the touchpad's gate ([Touchpad]), the one stock plugin
//! a per-phone switch covers: refused while Control is off, said in Activity once a stretch of use.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use vyoma_core::activity::Activity;
use vyoma_core::permits::{self, Permit, Permits};
use vyoma_core::{messages, presenter};

use crate::{calls, commands, media, mousepad, notifications, stock};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// The texts lock opened or closed on the phone (`vyoma.texts.*`).
    TextsLock,
    /// The phone did (or couldn't do) what a call request asked.
    CallDone,
    /// Every other `vyoma.*` packet: the phone's session, which checks the switches itself.
    Session,
    /// MPRIS: the phone's media controls.
    Media,
    /// The volume page.
    Volume,
    /// Touchpad and keyboard ([Touchpad] first).
    Mousepad,
    Commands,
    Notification,
    /// A call on the phone.
    Call,
    /// Texts and contacts the phone sent.
    Texts,
    /// The pointer dot.
    Presenter,
    /// Ping, battery, find, lock, the clipboard and shares ([stock::on_packet]).
    Stock,
    /// Not the engine's: the side's own, or nothing.
    Other,
}

/// Where a packet of type [kind] goes.
pub fn route(kind: &str) -> Route {
    match kind {
        messages::UNLOCKED | messages::LOCKED => Route::TextsLock,
        calls::DONE => Route::CallDone,
        k if k.starts_with("vyoma.") => Route::Session,
        media::MPRIS_REQUEST => Route::Media,
        media::VOLUME_REQUEST => Route::Volume,
        mousepad::MOUSEPAD_REQUEST => Route::Mousepad,
        commands::RUNCOMMAND_REQUEST => Route::Commands,
        notifications::NOTIFICATION => Route::Notification,
        calls::TELEPHONY => Route::Call,
        messages::MESSAGES | messages::CONTACTS_UIDS_REPLY | messages::CONTACTS_VCARDS_REPLY => Route::Texts,
        presenter::PRESENTER => Route::Presenter,
        k if stock::handles(k) => Route::Stock,
        _ => Route::Other,
    }
}

/// The log's line for a call request the phone answered (never who was called).
pub fn call_done(body: &Value) -> String {
    match body["error"].as_str() {
        Some(e) => format!("the phone couldn't {}: {e}", body["action"].as_str().unwrap_or("do that")),
        None => format!("call {}", body["action"].as_str().unwrap_or("done")),
    }
}

/// Use after a pause this long is a new stretch, said again in Activity.
pub const TOUCHPAD_STRETCH: Duration = Duration::from_secs(10 * 60);
/// A refusal is said again after this (moves stream).
const REFUSED_EVERY: Duration = Duration::from_secs(2);
/// Input the computer won't take is said again after this.
const STUCK_EVERY: Duration = Duration::from_secs(15);

/// The touchpad's gate, for every phone.
#[derive(Default)]
pub struct Touchpad {
    refused: HashMap<String, Instant>,
    used: HashMap<String, Instant>,
    stuck: Option<Instant>,
}

/// What a touchpad packet comes to.
#[derive(Debug, PartialEq)]
pub enum Touch {
    /// Control is off for this phone; the refusal to send, when it's time to say it again.
    Refused(Option<Value>),
    /// Do [acts]; [echo] answers the phone's keyboard.
    Use { acts: Vec<mousepad::Act>, echo: Option<Value> },
}

/// Input this computer wouldn't take, said to the phone.
#[derive(Debug, PartialEq)]
pub struct Stuck {
    pub refusal: Value,
    /// The first time this run.
    pub first: bool,
}

impl Touchpad {
    /// Packet [p] from phone [id] (shown as [name]); [switch] is how the side names the Control
    /// switch, for Activity.
    pub fn on_packet(&mut self, permits: &Permits, activity: &Activity, id: &str, name: &str, switch: &str, p: &Value) -> Touch {
        match self.gate(permits, activity, id, name, switch, p) {
            Ok(()) => Touch::Use { acts: mousepad::acts(&p["body"]), echo: mousepad::echo(&p["body"]) },
            Err(refusal) => Touch::Refused(refusal),
        }
    }

    /// [on_packet]'s check alone, for a side that reads the packet itself: Err while Control is
    /// off, with the refusal when it's time to say it again.
    pub fn gate(&mut self, permits: &Permits, activity: &Activity, id: &str, name: &str, switch: &str, p: &Value) -> Result<(), Option<Value>> {
        let now = Instant::now();
        if !permits.allows(id, Permit::Control) {
            let last = self.refused.insert(id.to_owned(), now);
            if last.is_some_and(|t| now.duration_since(t) <= REFUSED_EVERY) {
                return Err(None);
            }
            tracing::info!("{id}: touchpad refused, {} is off for this phone", Permit::Control.key());
            activity.add(id, name, "refused", format!("Refused: \u{201c}{switch}\u{201d} is off"));
            return Err(Some(permits::denied(Permit::Control, p)));
        }
        if self.used.insert(id.to_owned(), now).is_none_or(|t| now.duration_since(t) > TOUCHPAD_STRETCH) {
            activity.add(id, name, "touchpad", "Used as touchpad");
        }
        Ok(())
    }

    /// The computer wouldn't take packet [p]'s input ([why], as the phone shows it): the refusal,
    /// once in a while.
    pub fn stuck(&mut self, p: &Value, why: &str) -> Option<Stuck> {
        let now = Instant::now();
        if self.stuck.is_some_and(|t| now.duration_since(t) <= STUCK_EVERY) {
            return None;
        }
        let first = self.stuck.replace(now).is_none();
        let mut refusal = permits::denied(Permit::Control, p);
        refusal["body"]["reason"] = json!(why);
        Some(Stuck { refusal, first })
    }

    /// Phone [id] is gone: its next use is a new stretch.
    pub fn forget(&mut self, id: &str) {
        self.refused.remove(id);
        self.used.remove(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vyoma_core::link::packet;

    #[test]
    fn every_stock_packet_has_its_place() {
        assert_eq!(route(messages::UNLOCKED), Route::TextsLock);
        assert_eq!(route(calls::DONE), Route::CallDone);
        assert_eq!(route("vyoma.stats.request"), Route::Session);
        assert_eq!(route(media::MPRIS_REQUEST), Route::Media);
        assert_eq!(route(media::VOLUME_REQUEST), Route::Volume);
        assert_eq!(route(mousepad::MOUSEPAD_REQUEST), Route::Mousepad);
        assert_eq!(route(commands::RUNCOMMAND_REQUEST), Route::Commands);
        assert_eq!(route(notifications::NOTIFICATION), Route::Notification);
        assert_eq!(route(calls::TELEPHONY), Route::Call);
        assert_eq!(route(messages::MESSAGES), Route::Texts);
        assert_eq!(route(presenter::PRESENTER), Route::Presenter);
        for k in [stock::PING, stock::BATTERY, stock::FIND, stock::LOCK_REQUEST, stock::CLIPBOARD, stock::SHARE, stock::SHARE_UPDATE] {
            assert_eq!(route(k), Route::Stock, "{k}");
        }
        assert_eq!(route("kdeconnect.sftp"), Route::Other);
        assert_eq!(call_done(&json!({ "action": "answer" })), "call answer");
        assert_eq!(call_done(&json!({ "action": "decline", "error": "not allowed" })), "the phone couldn't decline: not allowed");
    }

    #[test]
    fn the_touchpad_says_once_and_refuses_while_off() {
        let (permits, activity, mut pad) = (Permits::default(), Activity::default(), Touchpad::default());
        let p = packet(mousepad::MOUSEPAD_REQUEST, json!({ "dx": 3.0, "dy": 0.0 }));
        for _ in 0..3 {
            assert!(matches!(pad.on_packet(&permits, &activity, "p1", "Pixel", "Touchpad", &p), Touch::Use { ref acts, echo: None } if !acts.is_empty()));
        }
        assert_eq!(activity.recent().len(), 1, "one stretch, one line");

        permits.set("p1", Permit::Control, false).unwrap();
        let Touch::Refused(Some(no)) = pad.on_packet(&permits, &activity, "p1", "Pixel", "Touchpad", &p) else { panic!("refused, said") };
        assert_eq!(no["type"], permits::DENIED);
        assert_eq!(pad.on_packet(&permits, &activity, "p1", "Pixel", "Touchpad", &p), Touch::Refused(None), "not again straight away");

        let first = pad.stuck(&p, "Not now").unwrap();
        assert!(first.first && first.refusal["body"]["reason"] == "Not now");
        assert!(pad.stuck(&p, "Not now").is_none());
    }
}
