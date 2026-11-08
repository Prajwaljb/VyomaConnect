//! Calls on the phone (stock `kdeconnect.telephony`), as kdeconnectd's telephony and pausemusic
//! plugins do them: a ringing phone shows a notification with Mute (which sends
//! `kdeconnect.telephony.request_mute`), a missed call stays as one, and what plays here pauses
//! while the phone rings or you talk, then resumes when the call ends. Only what this paused
//! resumes. Names and numbers never go to the log.

use std::collections::HashMap;

use serde_json::Value;

pub const TELEPHONY: &str = "kdeconnect.telephony";
pub const MUTE: &str = "kdeconnect.telephony.request_mute";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Ringing,
    Talking,
    Missed,
}

#[derive(Debug, PartialEq)]
pub struct Call {
    pub kind: Kind,
    /// The call it names is over: take its notification down.
    pub over: bool,
    /// Who: the contact's name, else the number (none for a hidden number, or an app that doesn't say).
    pub who: Option<String>,
    /// The number, when the name is a contact's.
    pub number: Option<String>,
    /// The app calling over the internet (WhatsApp...): such calls have no number.
    pub app: Option<String>,
}

/// A telephony packet, if it's a call (old phones also sent texts this way: those are ignored).
pub fn parse(p: &Value) -> Option<Call> {
    let b = &p["body"];
    let kind = match b["event"].as_str()? {
        "ringing" => Kind::Ringing,
        "talking" => Kind::Talking,
        "missedCall" => Kind::Missed,
        _ => return None,
    };
    let text = |k: &str| b[k].as_str().map(str::trim).filter(|n| !n.is_empty()).map(str::to_owned);
    let number = text("phoneNumber");
    let app = text("appName");
    // Without a caller, the phone names the app there (for desktops that don't know appName).
    let name = text("contactName").filter(|n| n != "Unknown" && Some(n) != app.as_ref());
    let who = name.or_else(|| number.clone());
    let number = number.filter(|n| Some(n) != who.as_ref());
    Some(Call { kind, over: b["isCancel"].as_bool().unwrap_or(false), who, number, app })
}

impl Call {
    /// "Sam is calling", "WhatsApp call" (the app didn't say who), "Unknown number is calling".
    pub fn ringing_title(&self) -> String {
        match (&self.who, &self.app) {
            (Some(who), _) => format!("{who} is calling"),
            (None, Some(app)) => format!("{app} call"),
            (None, None) => "Unknown number is calling".into(),
        }
    }

    pub fn missed_title(&self) -> String {
        match (&self.who, &self.app) {
            (Some(who), Some(app)) => format!("Missed {app} call from {who}"),
            (Some(who), None) => format!("Missed call from {who}"),
            (None, Some(app)) => format!("Missed {app} call"),
            (None, None) => "Missed call from unknown number".into(),
        }
    }

    /// The line under it: the number or the app, and which phone.
    pub fn body(&self, phone: &str) -> String {
        match (&self.number, &self.app) {
            (Some(n), _) => format!("{n}, on {phone}"),
            (None, Some(app)) if self.who.is_some() => format!("{app} call, on {phone}"),
            _ => format!("On {phone}"),
        }
    }
}

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
}

/// What to do for [call] on [line]: the state's already moved on.
#[derive(Debug, Default, PartialEq)]
pub struct Steps {
    pub close: Vec<u32>,
    pub show_ringing: bool,
    pub show_missed: bool,
    pub pause: bool,
    pub resume: bool,
}

impl Line {
    pub fn on_call(&mut self, call: &Call) -> Steps {
        let mut s = Steps::default();
        match (call.kind, call.over) {
            (Kind::Ringing, false) => {
                s.show_ringing = true;
                s.pause = !self.on;
                self.on = true;
            }
            (Kind::Talking, false) => {
                s.close.extend(self.ringing.take());
                s.pause = !self.on;
                self.on = true;
            }
            (Kind::Ringing | Kind::Talking, true) => {
                s.close.extend(self.ringing.take());
                s.resume = self.on;
                self.on = false;
            }
            (Kind::Missed, false) => {
                s.close.extend(self.ringing.take());
                s.show_missed = true;
                s.resume = self.on;
                self.on = false;
            }
            (Kind::Missed, true) => s.close.extend(self.missed.take()),
        }
        s
    }

    /// The phone's gone: nothing will say the call ended.
    pub fn gone(&mut self) -> Steps {
        let s = Steps { close: self.ringing.take().into_iter().collect(), resume: self.on, ..Default::default() };
        self.on = false;
        s
    }
}

/// Which phone a pressed Mute belongs to.
pub fn muting(lines: &HashMap<String, Line>, notification: u32) -> Option<String> {
    lines.iter().find(|(_, l)| l.ringing == Some(notification)).map(|(id, _)| id.clone())
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
        let lines = HashMap::from([("p".to_string(), Line { ringing: Some(9), ..Default::default() })]);
        assert_eq!(muting(&lines, 9).as_deref(), Some("p"));
    }
}
