//! The phone's notifications on this PC, as kdeconnectd's notifications plugin: each one the phone
//! posts shows here with its buttons (and a reply field where the desktop has inline replies);
//! dismissing one here dismisses it on the phone if the phone allows that, and the phone's own
//! dismissal takes it down here.
//!
//! Icons come as payloads named by their hash, kept in `~/.cache/vyoma-desktop/icons` so each is
//! fetched once. The phone's backlog (`silent`, sent when asked) isn't shown: it's for a list in
//! the GUI, which isn't there yet.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;
use vyoma_core::identity::Identity;
use vyoma_core::link::packet;
use vyoma_core::payload;

use crate::notify::Heard;

pub const NOTIFICATION: &str = "kdeconnect.notification";
pub const NOTIFICATION_REQUEST: &str = "kdeconnect.notification.request";
pub const NOTIFICATION_REPLY: &str = "kdeconnect.notification.reply";
pub const NOTIFICATION_ACTION: &str = "kdeconnect.notification.action";

/// Notifications remembered, to match their buttons; older ones are long off the screen.
const KEPT: usize = 256;
/// The most an icon may be.
const MAX_ICON: u64 = 1 << 20;

/// What a notification packet asks.
#[derive(Debug, PartialEq)]
pub enum Incoming {
    Show(Note),
    /// Gone from the phone.
    Cancel(String),
    Ignore,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub id: String,
    pub app: String,
    pub title: String,
    pub text: String,
    clearable: bool,
    reply: Option<String>,
    actions: Vec<String>,
    /// Its icon's name in the cache, when the packet carries one.
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

    /// Its buttons: key and label.
    pub fn buttons(&self) -> Vec<(String, String)> {
        self.actions.iter().enumerate().map(|(i, a)| (format!("phone-action-{i}"), a.clone())).collect()
    }

    pub fn replies(&self) -> bool {
        self.reply.is_some()
    }
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
    let s = |k: &str| b[k].as_str().unwrap_or("").to_owned();
    let note = Note {
        id: id.to_owned(),
        app: s("appName"),
        title: s("title"),
        text: s("text"),
        clearable: b["isClearable"].as_bool().unwrap_or(false),
        reply: b["requestReplyId"].as_str().filter(|r| !r.is_empty()).map(str::to_owned),
        actions: b["actions"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).take(3).collect()).unwrap_or_default(),
        icon: b["payloadHash"].as_str().filter(|h| (8..=64).contains(&h.len()) && h.bytes().all(|c| c.is_ascii_hexdigit())).map(str::to_lowercase),
    };
    if note.title.is_empty() && note.text.is_empty() {
        return Incoming::Ignore;
    }
    Incoming::Show(note)
}

struct Shown {
    device: String,
    note: Note,
}

/// A reply to ask for in a dialog (see [Mirror::take_reply]).
#[derive(Debug, PartialEq)]
pub struct ReplyAsk {
    pub device: String,
    pub reply_id: String,
    pub heading: String,
    /// The message being answered.
    pub text: String,
    pub placeholder: String,
}

impl ReplyAsk {
    pub fn packet(&self, message: &str) -> Value {
        packet(NOTIFICATION_REPLY, json!({ "requestReplyId": self.reply_id, "message": message }))
    }
}

/// Which notification here is which on which phone.
#[derive(Default)]
pub struct Mirror {
    by_notice: HashMap<u32, Shown>,
    order: VecDeque<u32>,
}

impl Mirror {
    /// The notification showing [phone_id] from [device], to update in place.
    pub fn showing(&self, device: &str, phone_id: &str) -> Option<u32> {
        self.by_notice.iter().find(|(_, s)| s.device == device && s.note.id == phone_id).map(|(n, _)| *n)
    }

    pub fn shown(&mut self, device: &str, note: Note, notice: u32) {
        if self.by_notice.insert(notice, Shown { device: device.to_owned(), note }).is_none() {
            self.order.push_back(notice);
        }
        while self.order.len() > KEPT {
            if let Some(old) = self.order.pop_front() {
                self.by_notice.remove(&old);
            }
        }
    }

    /// The phone dismissed it: the notification to take down here.
    pub fn cancelled(&mut self, device: &str, phone_id: &str) -> Option<u32> {
        let n = self.showing(device, phone_id)?;
        self.forget(n);
        Some(n)
    }

    fn forget(&mut self, notice: u32) -> Option<Shown> {
        self.order.retain(|n| *n != notice);
        self.by_notice.remove(&notice)
    }

    /// Its Reply button was pressed where the desktop has no inline replies (GNOME, Cinnamon...):
    /// what asking for the text needs. It's forgotten here, so the notification closing as the
    /// button is pressed doesn't dismiss it on the phone before the reply goes.
    pub fn take_reply(&mut self, notice: u32) -> Option<ReplyAsk> {
        self.by_notice.get(&notice)?.note.reply.as_ref()?;
        let s = self.forget(notice)?;
        let heading = s.note.heading();
        let to = if s.note.title.is_empty() { s.note.app.clone() } else { s.note.title.clone() };
        Some(ReplyAsk { device: s.device, reply_id: s.note.reply.unwrap_or_default(), heading, text: s.note.text, placeholder: format!("Reply to {to}") })
    }

    /// What to tell which phone after something happened to one of ours.
    pub fn heard(&mut self, h: &Heard) -> Option<(String, Value)> {
        match h {
            Heard::Pressed { notification, action } => {
                let s = self.by_notice.get(notification)?;
                let i: usize = action.strip_prefix("phone-action-")?.parse().ok()?;
                let label = s.note.actions.get(i)?;
                Some((s.device.clone(), packet(NOTIFICATION_ACTION, json!({ "key": s.note.id, "action": label }))))
            }
            Heard::Replied { notification, text } => {
                let s = self.by_notice.get(notification)?;
                Some((s.device.clone(), packet(NOTIFICATION_REPLY, json!({ "requestReplyId": s.note.reply.as_ref()?, "message": text }))))
            }
            Heard::Closed { notification, by_user } => {
                let s = self.forget(*notification)?;
                (*by_user && s.note.clearable).then(|| (s.device, packet(NOTIFICATION_REQUEST, json!({ "cancel": s.note.id }))))
            }
        }
    }
}

/// Asks for the text of a reply where notifications have no reply field: in GNOME's own dialog
/// (through Vyoma's extension), else zenity's. `None` if cancelled.
pub async fn ask_reply(ask: &ReplyAsk) -> Result<Option<String>> {
    if vyoma_companion::desktop::is_gnome() {
        return vyoma_companion::shell::ask_reply(&ask.heading, &ask.text, &ask.placeholder).await;
    }
    let out = tokio::process::Command::new("zenity")
        // Its text is read as markup, and it comes from the phone.
        .args(["--entry", "--title", &ask.heading, "--text", &crate::notify::escape(&ask.text), "--ok-label", "Send"])
        .kill_on_drop(true)
        .output()
        .await
        .context("this desktop's notifications have no reply field, and zenity isn't installed to ask instead")?;
    let text = String::from_utf8_lossy(&out.stdout).trim_end_matches('\n').to_owned();
    Ok(Some(text).filter(|t| out.status.success() && !t.trim().is_empty()))
}

/// Where icon [name] is kept.
pub fn icon_path(name: &str) -> PathBuf {
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache"));
    cache.join("vyoma-desktop/icons").join(format!("{name}.png"))
}

/// Fetches the icon [p] carries into [path].
pub async fn fetch_icon(me: &Identity, pinned: &[u8], from: IpAddr, p: &Value, path: &Path) -> Result<()> {
    let size = p["payloadSize"].as_u64().context("no icon")?;
    if size == 0 || size > MAX_ICON {
        bail!("icon of {size} bytes");
    }
    let mut bytes = Vec::with_capacity(size as usize);
    payload::fetch(me, pinned, from, p).await?.read_to_end(&mut bytes).await?;
    if bytes.len() as u64 != size {
        bail!("icon cut short");
    }
    let dir = path.parent().context("no cache folder")?;
    tokio::fs::create_dir_all(dir).await?;
    let part = path.with_extension(format!("{}.part", std::process::id()));
    tokio::fs::write(&part, &bytes).await?;
    tokio::fs::rename(&part, path).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post(body: Value) -> Value {
        packet(NOTIFICATION, body)
    }

    #[test]
    fn notes_and_what_comes_of_them() {
        assert_eq!(parse(&post(json!({ "id": "1", "isCancel": true }))), Incoming::Cancel("1".into()));
        assert_eq!(parse(&post(json!({ "id": "1", "title": "x", "silent": true }))), Incoming::Ignore);
        assert_eq!(parse(&post(json!({ "id": "1", "appName": "Mail" }))), Incoming::Ignore, "nothing to show");
        let Incoming::Show(note) = parse(&post(json!({
            "id": "k1", "appName": "Chat", "title": "Ann", "text": "hi", "isClearable": true,
            "requestReplyId": "r1", "actions": ["Mark read"], "payloadHash": "ABCDEF0123456789", "time": "1",
        }))) else {
            panic!()
        };
        assert_eq!((note.heading(), note.icon.as_deref()), ("Chat: Ann".into(), Some("abcdef0123456789")));
        let Incoming::Show(odd) = parse(&post(json!({ "id": "2", "title": "t", "payloadHash": "../../x" }))) else { panic!() };
        assert_eq!((odd.heading(), odd.icon), ("t".into(), None));

        let mut m = Mirror::default();
        m.shown("phone", note.clone(), 7);
        assert_eq!(m.showing("phone", "k1"), Some(7));
        let (to, p) = m.heard(&Heard::Pressed { notification: 7, action: "phone-action-0".into() }).unwrap();
        assert_eq!((to.as_str(), &p["body"]), ("phone", &json!({ "key": "k1", "action": "Mark read" })));
        assert!(m.heard(&Heard::Pressed { notification: 7, action: "phone-action-5".into() }).is_none());
        assert!(m.heard(&Heard::Pressed { notification: 8, action: "phone-action-0".into() }).is_none(), "not ours");
        let (_, p) = m.heard(&Heard::Replied { notification: 7, text: "ok".into() }).unwrap();
        assert_eq!(p["body"], json!({ "requestReplyId": "r1", "message": "ok" }));
        let (_, p) = m.heard(&Heard::Closed { notification: 7, by_user: true }).unwrap();
        assert_eq!(p["body"], json!({ "cancel": "k1" }));
        assert!(m.showing("phone", "k1").is_none());

        m.shown("phone", note, 9);
        assert_eq!(m.cancelled("phone", "k1"), Some(9));
        assert!(m.heard(&Heard::Closed { notification: 9, by_user: false }).is_none(), "taken down for the phone: nothing back");
    }
}
