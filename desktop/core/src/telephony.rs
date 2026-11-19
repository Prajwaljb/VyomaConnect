//! Calls on the phone as the stock `kdeconnect.telephony` packets say them, with Vyoma's extra
//! fields, and how a PC words them. Names and numbers never go to the log.

use serde_json::Value;

pub const TELEPHONY: &str = "kdeconnect.telephony";
pub const MUTE: &str = "kdeconnect.telephony.request_mute";
/// PC → phone: `{action: answer|decline|hangup}`.
pub const REQUEST: &str = "vyoma.call.request";
/// Phone → PC: `{action, error?}`.
pub const DONE: &str = "vyoma.call.done";
/// PC → phone: `{number}`, call it from the window's Messages page. The phone allows it only while it has let this PC
/// see its texts (its owner's fingerprint), only for a plain number, and never an emergency number; it answers with
/// [DONE] (`action: dial`).
pub const DIAL: &str = "vyoma.call.dial";

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
    /// This PC may answer, decline and hang up (Vyoma's phone app, with the permission).
    pub answerable: bool,
    /// The phone has a Bluetooth headset of its own: the sound stays with it.
    pub headset: bool,
    /// The phone's Bluetooth name, to find it among this PC's paired devices.
    pub bluetooth: Option<String>,
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
    let flag = |k: &str| b[k].as_bool().unwrap_or(false);
    Some(Call { kind, over: flag("isCancel"), who, number, app, answerable: flag("vyomaAnswer"), headset: flag("vyomaHeadset"), bluetooth: text("vyomaBluetooth") })
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

    /// "On a call with Sam", "On a WhatsApp call".
    pub fn talking_title(&self) -> String {
        match (&self.who, &self.app) {
            (Some(who), _) => format!("On a call with {who}"),
            (None, Some(app)) => format!("On a {app} call"),
            (None, None) => "On a call".into(),
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

/// What a PC side does as one phone's calls come and go: told who's ringing once a call, the
/// alert taken down when it's answered or over, a missed call left until the phone clears it, and
/// what's playing paused for the call and resumed after it.
#[derive(Debug, Default, PartialEq)]
pub struct Steps {
    pub ring: bool,
    pub stop_ringing: bool,
    pub missed: bool,
    pub clear_missed: bool,
    pub pause: bool,
    pub resume: bool,
}

/// One phone's line: whether a call is on (ringing or talking), so what's paused stays paused.
#[derive(Default)]
pub struct Line {
    on: bool,
}

impl Line {
    pub fn on_call(&mut self, call: &Call) -> Steps {
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

    /// The phone's gone: nothing will say the call ended.
    pub fn gone(&mut self) -> Steps {
        let mut s = Steps::default();
        self.end(&mut s);
        s
    }

    fn end(&mut self, s: &mut Steps) {
        s.stop_ringing = true;
        s.resume = self.on;
        self.on = false;
    }
}

#[cfg(test)]
mod line_tests {
    use super::*;
    use serde_json::json;

    fn call(event: &str, over: bool) -> Call {
        parse(&json!({ "type": TELEPHONY, "body": { "event": event, "contactName": "A", "isCancel": over } })).unwrap()
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
