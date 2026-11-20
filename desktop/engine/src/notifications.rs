//! The phone's notifications on the PC (`kdeconnect.notification`), read once for every PC side:
//! what each packet asks (show, take down, or nothing), the notification in plain fields, and the
//! packets that answer it (a reply, one of its buttons, dismissed here). How it's shown, and how a
//! press comes back, is each side's (freedesktop notifications, toasts, UserNotifications).
//!
//! The phone's backlog (`silent`, sent when asked) isn't shown: it's for a list, not the screen.

use serde_json::{Value, json};
use vyoma_core::link::packet;

pub const NOTIFICATION: &str = "kdeconnect.notification";
/// PC → phone: `{cancel: id}`, dismissed here.
pub const NOTIFICATION_REQUEST: &str = "kdeconnect.notification.request";
pub const NOTIFICATION_REPLY: &str = "kdeconnect.notification.reply";
pub const NOTIFICATION_ACTION: &str = "kdeconnect.notification.action";

/// The most buttons shown: desktops show three at most.
const MAX_ACTIONS: usize = 3;

/// What a notification packet asks.
#[derive(Debug, PartialEq)]
pub enum Incoming {
    Show(Note),
    /// Gone from the phone: its id.
    Cancel(String),
    Ignore,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    /// The phone's key for it.
    pub id: String,
    pub app: String,
    pub title: String,
    pub text: String,
    /// Whether dismissing it here may dismiss it on the phone.
    pub clearable: bool,
    /// Where a reply goes, if it takes one.
    pub reply: Option<String>,
    /// Its buttons' labels.
    pub actions: Vec<String>,
    /// Its icon's payload hash (lower-case hex), when the packet carries one.
    pub icon: Option<String>,
}

impl Note {
    /// "App: Title", or whichever of them there is.
    pub fn heading(&self) -> String {
        match (self.app.is_empty(), self.title.is_empty() || self.title == self.app) {
            (_, true) => self.app.clone(),
            (true, false) => self.title.clone(),
            (false, false) => format!("{}: {}", self.app, self.title),
        }
    }

    /// Its title, or the app's name where it has none.
    pub fn title_or_app(&self) -> &str {
        if self.title.is_empty() { &self.app } else { &self.title }
    }

    /// Where it's from: "App · Phone", or the phone alone.
    pub fn from_line(&self, phone: &str) -> String {
        if self.app.is_empty() { phone.to_owned() } else { format!("{} \u{b7} {phone}", self.app) }
    }

    /// The reply field's placeholder.
    pub fn reply_hint(&self) -> String {
        format!("Reply to {}", self.title_or_app())
    }

    /// The reply [message] to it, if it takes one.
    pub fn reply_packet(&self, message: &str) -> Option<Value> {
        Some(reply_packet(self.reply.as_deref()?, message))
    }

    /// Its button [i] pressed.
    pub fn action_packet(&self, i: usize) -> Option<Value> {
        Some(action_packet(&self.id, self.actions.get(i)?))
    }

    /// Dismissed here by the person: to dismiss on the phone, if it may be.
    pub fn dismiss_packet(&self) -> Option<Value> {
        self.clearable.then(|| dismiss_packet(&self.id))
    }
}

pub fn reply_packet(reply_id: &str, message: &str) -> Value {
    packet(NOTIFICATION_REPLY, json!({ "requestReplyId": reply_id, "message": message }))
}

pub fn action_packet(key: &str, action: &str) -> Value {
    packet(NOTIFICATION_ACTION, json!({ "key": key, "action": action }))
}

pub fn dismiss_packet(key: &str) -> Value {
    packet(NOTIFICATION_REQUEST, json!({ "cancel": key }))
}

pub fn parse(p: &Value) -> Incoming {
    let b = &p["body"];
    let Some(id) = b["id"].as_str().filter(|id| !id.is_empty()) else { return Incoming::Ignore };
    if b["isCancel"].as_bool() == Some(true) {
        return Incoming::Cancel(id.to_owned());
    }
    if b["silent"].as_bool() == Some(true) {
        return Incoming::Ignore;
    }
    let s = |k: &str| b[k].as_str().unwrap_or("").trim().to_owned();
    let note = Note {
        id: id.to_owned(),
        app: s("appName"),
        title: s("title"),
        text: s("text"),
        clearable: b["isClearable"].as_bool().unwrap_or(false),
        reply: b["requestReplyId"].as_str().map(str::trim).filter(|r| !r.is_empty()).map(str::to_owned),
        actions: b["actions"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).take(MAX_ACTIONS).collect()).unwrap_or_default(),
        icon: b["payloadHash"].as_str().filter(|h| (8..=64).contains(&h.len()) && h.bytes().all(|c| c.is_ascii_hexdigit())).map(str::to_lowercase),
    };
    if note.title.is_empty() && note.text.is_empty() {
        return Incoming::Ignore;
    }
    Incoming::Show(note)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(body: Value) -> Incoming {
        parse(&packet(NOTIFICATION, body))
    }

    #[test]
    fn reads_what_to_show() {
        let Incoming::Show(note) = n(json!({
            "id": "0|chat|1", "appName": "Chat", "title": " Sam ", "text": "Lunch?", "isClearable": true,
            "requestReplyId": "r1", "actions": ["Mark read", "Mute", "Archive", "Fourth"], "payloadHash": "ABCDEF0123"
        })) else { panic!("shown") };
        assert_eq!((note.title.as_str(), note.heading(), note.from_line("Pixel")), ("Sam", "Chat: Sam".into(), "Chat \u{b7} Pixel".into()));
        assert_eq!((note.actions.len(), note.icon.as_deref(), note.reply_hint()), (3, Some("abcdef0123"), "Reply to Sam".into()));
        assert_eq!(note.reply_packet("ok").unwrap()["body"], json!({ "requestReplyId": "r1", "message": "ok" }));
        assert_eq!(note.action_packet(1).unwrap()["body"], json!({ "key": "0|chat|1", "action": "Mute" }));
        assert!(note.action_packet(3).is_none());
        assert_eq!(note.dismiss_packet().unwrap()["body"], json!({ "cancel": "0|chat|1" }));
    }

    #[test]
    fn cancels_and_skips() {
        assert_eq!(n(json!({ "id": "a", "isCancel": true })), Incoming::Cancel("a".into()));
        assert_eq!(n(json!({ "id": "a", "title": "x", "silent": true })), Incoming::Ignore, "the backlog");
        assert_eq!(n(json!({ "id": "a", "appName": "Clock" })), Incoming::Ignore, "nothing to read");
        assert_eq!(n(json!({ "title": "x" })), Incoming::Ignore, "no id");
        let Incoming::Show(note) = n(json!({ "id": "b", "appName": "Mail", "text": "new", "payloadHash": "../x" })) else { panic!() };
        assert_eq!((note.heading(), note.title_or_app(), note.icon.as_deref(), note.dismiss_packet()), ("Mail".into(), "Mail", None, None), "not clearable, no icon");
        assert!(note.reply_packet("x").is_none());
    }
}
