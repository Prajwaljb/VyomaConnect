//! Calls on the phone (stock `kdeconnect.telephony`), as kdeconnectd's telephony and pausemusic
//! plugins do them: a ringing phone shows a notification with Mute (which sends
//! `kdeconnect.telephony.request_mute`), a missed call stays as one, and what plays here pauses
//! while the phone rings or you talk, then resumes when the call ends. Only what this paused
//! resumes. Names and numbers never go to the log.
//!
//! Vyoma's phone app also says whether this PC may answer (`vyomaAnswer`): the ringing notification
//! then has Answer and Decline (`vyoma.call.request`), and a call answered here gets a notification of
//! its own with Hang up and where its sound is: on this PC (as the phone's Bluetooth headset,
//! `hfp.rs`) unless the phone already has a headset of its own (`vyomaHeadset`), or on the phone.

use std::collections::HashMap;

pub use vyoma_core::telephony::*;

/// One phone's call, as far as this PC is concerned.
#[derive(Default)]
pub struct Line {
    /// The ringing call's notification.
    pub ringing: Option<u32>,
    /// The missed call's notification.
    pub missed: Option<u32>,
    /// A call is on (ringing or talking): what's paused stays paused.
    pub on: bool,
    /// The players this paused, by bus name, to resume.
    pub paused: Vec<String>,
    /// The notification of a call answered here.
    pub talking: Option<u32>,
    /// Its title, for when where the sound is changes.
    pub talking_title: String,
    /// Answer was pressed here: the call gets its notification here once it's on.
    pub answered: bool,
    /// Called from here: like [answered], and its sound comes here when it starts.
    pub dialled: bool,
    /// Decline was pressed here: the phone reports a missed call, which it isn't.
    pub declined: bool,
    /// The ringing call's facts, as the phone last said them.
    pub answerable: bool,
    pub headset: bool,
    pub bluetooth: Option<String>,
    /// The call's sound on this PC, while it's being taken or held: given back when the call ends.
    pub audio: Option<tokio::task::JoinHandle<Option<crate::hfp::Audio>>>,
}

/// What to do for [call] on [line]: the state's already moved on.
#[derive(Debug, Default, PartialEq)]
pub struct Steps {
    pub close: Vec<u32>,
    pub show_ringing: bool,
    pub show_missed: bool,
    /// A call answered here is on: show its notification.
    pub show_talking: bool,
    /// The call's over: give its sound back to the phone.
    pub release: bool,
    /// A call made here started: take its sound (unless the phone has a headset of its own).
    pub take_audio: bool,
    pub pause: bool,
    pub resume: bool,
}

impl Line {
    pub fn on_call(&mut self, call: &Call) -> Steps {
        let mut s = Steps::default();
        if !call.over && call.kind != Kind::Missed {
            (self.answerable, self.headset, self.bluetooth) = (call.answerable, call.headset, call.bluetooth.clone());
        }
        match (call.kind, call.over) {
            (Kind::Ringing, false) => {
                s.show_ringing = true;
                if !self.on {
                    self.declined = false;
                }
                s.pause = !self.on;
                self.on = true;
            }
            (Kind::Talking, false) => {
                s.close.extend(self.ringing.take());
                s.show_talking = (self.answered || self.dialled) && self.talking.is_none();
                s.take_audio = self.dialled && self.talking.is_none() && !call.headset;
                s.pause = !self.on;
                self.on = true;
            }
            (Kind::Ringing | Kind::Talking, true) => self.end(&mut s),
            (Kind::Missed, false) => {
                s.show_missed = !self.declined;
                self.end(&mut s);
            }
            (Kind::Missed, true) => s.close.extend(self.missed.take()),
        }
        s
    }

    fn end(&mut self, s: &mut Steps) {
        s.close.extend(self.ringing.take());
        s.close.extend(self.talking.take());
        s.release = self.audio.is_some();
        s.resume = self.on;
        // [declined] stays: the phone says "missed" after "ringing ended".
        (self.on, self.answered, self.dialled) = (false, false, false);
    }

    /// The phone's gone: nothing will say the call ended.
    pub fn gone(&mut self) -> Steps {
        let mut s = Steps::default();
        self.end(&mut s);
        s
    }
}

/// Which phone a pressed call button belongs to: its ringing or talking notification.
pub fn line_of(lines: &HashMap<String, Line>, notification: u32) -> Option<String> {
    lines.iter().find(|(_, l)| l.ringing == Some(notification) || l.talking == Some(notification)).map(|(id, _)| id.clone())
}

/// The ringing notification's buttons.
pub fn ringing_actions(answerable: bool) -> &'static [(&'static str, &'static str)] {
    if answerable { &[("answer", "Answer"), ("decline", "Decline"), ("mute", "Silence")] } else { &[("mute", "Silence")] }
}

/// A call answered here: its line under the title and its buttons, by where the sound is.
pub fn talking_notice(here: bool, phone: &str) -> (String, &'static [(&'static str, &'static str)]) {
    if here {
        ("Sound on this PC".into(), &[("hangup", "Hang up"), ("phone-audio", "Use phone")])
    } else {
        (format!("Sound on {phone}"), &[("hangup", "Hang up"), ("pc-audio", "Use this PC")])
    }
}

/// What a failed request says: "Couldn't answer".
pub fn failed_title(action: &str) -> &'static str {
    match action {
        "answer" => "Couldn't answer the call",
        "decline" => "Couldn't decline the call",
        "dial" => "Couldn't make the call",
        _ => "Couldn't hang up",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(event: &str, cancel: bool) -> Call {
        parse(&json!({ "type": TELEPHONY, "body": { "event": event, "contactName": "A", "phoneNumber": "1", "isCancel": cancel } })).unwrap()
    }

    #[test]
    fn reads_who_is_calling() {
        let c = parse(&json!({ "body": { "event": "ringing", "contactName": "Unknown", "phoneNumber": " 123 " } })).unwrap();
        assert_eq!((c.kind, c.over, c.who.as_deref(), c.number.as_deref()), (Kind::Ringing, false, Some("123"), None::<&str>));
        assert_eq!((c.ringing_title(), c.body("P")), ("123 is calling".into(), "On P".into()));
        let c = call("ringing", false);
        assert_eq!((c.who.as_deref(), c.number.as_deref()), (Some("A"), Some("1")));
        assert_eq!((c.ringing_title(), c.missed_title(), c.body("P")), ("A is calling".into(), "Missed call from A".into(), "1, on P".into()));
        assert!(parse(&json!({ "body": { "event": "sms" } })).is_none());
        let c = parse(&json!({ "body": { "event": "ringing", "contactName": "Unknown" } })).unwrap();
        assert_eq!(c.ringing_title(), "Unknown number is calling");
    }

    #[test]
    fn an_apps_call_names_the_app_not_an_unknown_number() {
        let c = parse(&json!({ "body": { "event": "ringing", "contactName": "Sam", "appName": "WhatsApp" } })).unwrap();
        assert_eq!((c.ringing_title(), c.body("P")), ("Sam is calling".into(), "WhatsApp call, on P".into()));
        assert_eq!(c.missed_title(), "Missed WhatsApp call from Sam");
        // The caller kept private: the phone names the app as the contact.
        let c = parse(&json!({ "body": { "event": "ringing", "contactName": "WhatsApp", "appName": "WhatsApp" } })).unwrap();
        assert_eq!((c.ringing_title(), c.missed_title(), c.body("P")), ("WhatsApp call".into(), "Missed WhatsApp call".into(), "On P".into()));
    }

    #[test]
    fn music_pauses_for_the_whole_call_and_resumes_once() {
        let mut line = Line::default();
        let s = line.on_call(&call("ringing", false));
        assert!(s.show_ringing && s.pause);
        line.ringing = Some(7);
        let s = line.on_call(&call("talking", false));
        assert_eq!((s.close, s.pause), (vec![7], false));
        let s = line.on_call(&call("talking", true));
        assert!(s.resume);
        assert!(!line.on_call(&call("talking", true)).resume);
    }

    #[test]
    fn a_missed_call_stays_and_resumes_the_music() {
        let mut line = Line::default();
        line.on_call(&call("ringing", false));
        line.ringing = Some(3);
        let s = line.on_call(&call("missedCall", false));
        assert_eq!((s.close, s.show_missed, s.resume), (vec![3], true, true));
        line.missed = Some(4);
        assert_eq!(line.on_call(&call("missedCall", true)).close, vec![4]);
    }

    #[test]
    fn a_vanished_phone_ends_its_call() {
        let mut line = Line::default();
        line.on_call(&call("ringing", false));
        line.ringing = Some(5);
        let s = line.gone();
        assert_eq!((s.close, s.resume), (vec![5], true));
        let lines = HashMap::from([("p".to_string(), Line { ringing: Some(9), talking: Some(8), ..Default::default() })]);
        assert_eq!(line_of(&lines, 9).as_deref(), Some("p"));
        assert_eq!(line_of(&lines, 8).as_deref(), Some("p"));
        assert_eq!(line_of(&lines, 1), None);
    }

    #[test]
    fn reads_what_vyoma_adds() {
        let c = parse(&json!({ "body": { "event": "ringing", "contactName": "A", "vyomaAnswer": true, "vyomaHeadset": false, "vyomaBluetooth": " Phone " } })).unwrap();
        assert_eq!((c.answerable, c.headset, c.bluetooth.as_deref()), (true, false, Some("Phone")));
        let c = call("ringing", false);
        assert_eq!((c.answerable, c.headset, c.bluetooth.as_deref()), (false, false, None));
        assert_eq!(ringing_actions(false), &[("mute", "Silence")]);
        assert_eq!(ringing_actions(true)[0], ("answer", "Answer"));
        assert_eq!(c.talking_title(), "On a call with A");
    }

    #[test]
    fn a_call_answered_here_gets_its_own_notice_until_it_ends() {
        let mut line = Line::default();
        line.on_call(&call("ringing", false));
        line.ringing = Some(1);
        line.answered = true;
        let s = line.on_call(&call("talking", false));
        assert_eq!((s.close, s.show_talking), (vec![1], true));
        line.talking = Some(2);
        assert!(!line.on_call(&call("talking", false)).show_talking);
        let s = line.on_call(&call("talking", true));
        assert_eq!((s.close, s.resume), (vec![2], true));
        assert!(!line.answered);
        // Answered on the phone: nothing new here.
        line.on_call(&call("ringing", false));
        assert!(!line.on_call(&call("talking", false)).show_talking);
    }

    #[test]
    fn a_call_made_here_takes_its_sound_once_it_starts() {
        let mut line = Line { dialled: true, ..Line::default() };
        let s = line.on_call(&call("talking", false));
        assert_eq!((s.show_talking, s.take_audio, s.pause), (true, true, true));
        line.talking = Some(3);
        assert!(!line.on_call(&call("talking", false)).take_audio, "once");
        line.on_call(&call("talking", true));
        assert!(!line.dialled, "over");
        // The phone's own headset keeps it.
        let mut line = Line { dialled: true, ..Line::default() };
        let mut with_headset = call("talking", false);
        with_headset.headset = true;
        assert!(!line.on_call(&with_headset).take_audio);
    }

    #[test]
    fn a_declined_call_isnt_missed() {
        let mut line = Line::default();
        line.on_call(&call("ringing", false));
        line.declined = true;
        line.on_call(&call("ringing", true));
        let s = line.on_call(&call("missedCall", false));
        assert!(!s.show_missed);
        line.on_call(&call("ringing", false));
        assert!(line.on_call(&call("missedCall", false)).show_missed);
    }
}
