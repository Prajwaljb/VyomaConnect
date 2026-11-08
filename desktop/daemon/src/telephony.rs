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
    /// Who: the contact's name, else the number.
    pub who: String,
    /// The number, when the name is a contact's.
    pub number: Option<String>,
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
    let number = b["phoneNumber"].as_str().map(str::trim).filter(|n| !n.is_empty()).map(str::to_owned);
    let name = b["contactName"].as_str().map(str::trim).filter(|n| !n.is_empty() && *n != "Unknown");
    let who = name.map(str::to_owned).or_else(|| number.clone()).unwrap_or_else(|| "Unknown number".into());
    let number = number.filter(|n| *n != who);
    Some(Call { kind, over: b["isCancel"].as_bool().unwrap_or(false), who, number })
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
        assert_eq!((c.kind, c.over, c.who.as_str(), c.number), (Kind::Ringing, false, "123", None));
        let c = call("ringing", false);
        assert_eq!((c.who.as_str(), c.number.as_deref()), ("A", Some("1")));
        assert!(parse(&json!({ "body": { "event": "sms" } })).is_none());
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
