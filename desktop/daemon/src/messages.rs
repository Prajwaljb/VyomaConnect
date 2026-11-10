//! Texts on the phone, read and sent from the window's Messages page, over the stock SMS and
//! contacts packets (kdeconnect.sms.*, kdeconnect.contacts.*), so any KDE Connect phone app works.
//! Kept in memory only, per phone: asked for when the page first opens on each link, then kept
//! current by the phone, which sends each new text once it's been asked. Plain SMS only (Android
//! doesn't let other apps read RCS chats). What texts say and who they're with never go to the log.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::sync::watch;

pub const MESSAGES: &str = "kdeconnect.sms.messages";
pub const REQUEST: &str = "kdeconnect.sms.request";
pub const REQUEST_CONVERSATIONS: &str = "kdeconnect.sms.request_conversations";
pub const REQUEST_CONVERSATION: &str = "kdeconnect.sms.request_conversation";
pub const CONTACTS_UIDS: &str = "kdeconnect.contacts.request_all_uids_timestamps";
pub const CONTACTS_VCARDS: &str = "kdeconnect.contacts.request_vcards_by_uid";
pub const CONTACTS_UIDS_REPLY: &str = "kdeconnect.contacts.response_uids_timestamps";
pub const CONTACTS_VCARDS_REPLY: &str = "kdeconnect.contacts.response_vcards";

/// How many of a thread's newest texts are asked for.
const THREAD_TEXTS: u64 = 200;
/// No answer after this: the phone isn't sharing its texts (or has none).
const SILENT_AFTER: Duration = Duration::from_secs(8);
/// A text sent from here shows as sending until the phone has it, or this long.
const PENDING_FOR: Duration = Duration::from_secs(90);

#[derive(Clone, Debug, PartialEq)]
struct Text {
    uid: i64,
    body: String,
    date: i64,
    /// Sent from the phone (or this PC), rather than received.
    mine: bool,
    failed: bool,
    read: bool,
    addresses: Vec<String>,
}

#[derive(Default)]
struct Thread {
    /// By (date, uid), oldest first.
    texts: BTreeMap<(i64, i64), Text>,
    /// Its own texts were asked for (not just its newest).
    asked: bool,
    /// Sent from here, not yet back from the phone.
    pending: Vec<(String, Instant)>,
}

#[derive(Default)]
struct Inbox {
    threads: HashMap<i64, Thread>,
    /// Phone number's last digits to the contact's name.
    contacts: HashMap<String, String>,
    /// When the conversations were asked for on this link; None until the page opens.
    asked: Option<Instant>,
    /// Anything came back since.
    heard: bool,
}

/// What the phones' texts are, shared by the D-Bus side and the main loop.
pub struct Store {
    inboxes: Mutex<HashMap<String, Inbox>>,
    tick: watch::Sender<u64>,
}

impl Default for Store {
    fn default() -> Self {
        Self { inboxes: Mutex::default(), tick: watch::channel(0).0 }
    }
}

impl Store {
    /// Ticks whenever something the page shows changed.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.tick.subscribe()
    }

    fn changed(&self) {
        self.tick.send_modify(|n| *n += 1);
    }

    /// The conversations as JSON, newest first: `{state: loading|ready|silent, list: [...]}`, and
    /// the packets to send the phone first if this link hasn't asked yet.
    pub fn conversations(&self, id: &str) -> (String, Vec<Value>) {
        let mut all = self.inboxes.lock().unwrap();
        let inbox = all.entry(id.to_owned()).or_default();
        let mut ask = Vec::new();
        if inbox.asked.is_none() {
            inbox.asked = Some(Instant::now());
            ask.push(packet(REQUEST_CONVERSATIONS, json!({})));
            ask.push(packet(CONTACTS_UIDS, json!({})));
        }
        let mut list: Vec<(i64, Value)> = inbox
            .threads
            .iter()
            .filter_map(|(thread, t)| {
                let (pending, last) = (t.pending.last(), t.texts.values().next_back());
                let (snippet, date, mine) = match (pending, last) {
                    (Some((body, _)), _) => (body.clone(), now_ms(), true),
                    (None, Some(last)) => (last.body.clone(), last.date, last.mine),
                    (None, None) => return None,
                };
                let unread = last.is_some_and(|l| !l.mine && !l.read);
                let name = inbox.name_of(t);
                Some((date, json!({ "thread": thread, "name": name, "snippet": snippet, "date": date, "mine": mine, "unread": unread })))
            })
            .collect();
        list.sort_by(|a, b| b.0.cmp(&a.0));
        let state = if !list.is_empty() || inbox.heard {
            "ready"
        } else if inbox.asked.is_some_and(|at| at.elapsed() >= SILENT_AFTER) {
            "silent"
        } else {
            "loading"
        };
        (json!({ "state": state, "list": list.into_iter().map(|(_, v)| v).collect::<Vec<_>>() }).to_string(), ask)
    }

    /// One thread as JSON, oldest first: `{name, number, texts: [{body, date, mine, failed, sending}]}`,
    /// and the packet asking the phone for it the first time.
    pub fn thread(&self, id: &str, thread: i64) -> (String, Vec<Value>) {
        let mut all = self.inboxes.lock().unwrap();
        let inbox = all.entry(id.to_owned()).or_default();
        let t = inbox.threads.entry(thread).or_default();
        let mut ask = Vec::new();
        if !t.asked {
            t.asked = true;
            ask.push(packet(REQUEST_CONVERSATION, json!({ "threadID": thread, "rangeStartTimestamp": -1, "numberToRequest": THREAD_TEXTS })));
        }
        t.pending.retain(|(_, at)| at.elapsed() < PENDING_FOR);
        let mut texts: Vec<Value> = t.texts.values().map(|m| json!({ "body": m.body, "date": m.date, "mine": m.mine, "failed": m.failed, "sending": false })).collect();
        texts.extend(t.pending.iter().map(|(body, _)| json!({ "body": body, "date": now_ms(), "mine": true, "failed": false, "sending": true })));
        let t = &inbox.threads[&thread];
        let number = addresses(t).join(", ");
        (json!({ "name": inbox.name_of(t), "number": number, "texts": texts }).to_string(), ask)
    }

    /// Sends [body] to [thread]'s people: the packet for the phone, or why it can't.
    pub fn send(&self, id: &str, thread: i64, body: &str) -> Result<Value, String> {
        let body = body.trim();
        if body.is_empty() {
            return Err("Nothing to send".into());
        }
        let mut all = self.inboxes.lock().unwrap();
        let t = all.get_mut(id).and_then(|i| i.threads.get_mut(&thread)).ok_or("That conversation isn't loaded")?;
        let to = addresses(t);
        if to.is_empty() {
            return Err("That conversation has no number to text".into());
        }
        t.pending.push((body.to_owned(), Instant::now()));
        drop(all);
        self.changed();
        let list: Vec<Value> = to.iter().map(|a| json!({ "address": a })).collect();
        Ok(packet(REQUEST, json!({ "sendSms": true, "version": 2, "addresses": list, "phoneNumber": to[0], "messageBody": body, "subID": -1 })))
    }

    /// A packet from [id]'s phone: what to send it next (the contacts' cards).
    pub fn on_packet(&self, id: &str, p: &Value) -> Vec<Value> {
        let b = &p["body"];
        let mut all = self.inboxes.lock().unwrap();
        let inbox = all.entry(id.to_owned()).or_default();
        let mut out = Vec::new();
        match p["type"].as_str().unwrap_or("") {
            MESSAGES => {
                inbox.heard = true;
                for m in b["messages"].as_array().into_iter().flatten() {
                    let Some((thread, text)) = parse_text(m) else { continue };
                    let t = inbox.threads.entry(thread).or_default();
                    if text.mine
                        && let Some(i) = t.pending.iter().position(|(body, _)| *body == text.body)
                    {
                        t.pending.remove(i);
                    }
                    t.texts.insert((text.date, text.uid), text);
                }
            }
            CONTACTS_UIDS_REPLY => {
                let uids: Vec<Value> = b["uids"].as_array().cloned().unwrap_or_default();
                if !uids.is_empty() {
                    out.push(packet(CONTACTS_VCARDS, json!({ "uids": uids })));
                }
                return out;
            }
            CONTACTS_VCARDS_REPLY => {
                for uid in b["uids"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                    if let Some((name, numbers)) = b[uid].as_str().and_then(vcard) {
                        for n in numbers {
                            inbox.contacts.insert(key(&n), name.clone());
                        }
                    }
                }
            }
            _ => return out,
        }
        drop(all);
        self.changed();
        out
    }

    /// [id]'s link is gone: the next look asks again (what came meanwhile is news).
    pub fn gone(&self, id: &str) {
        if let Some(inbox) = self.inboxes.lock().unwrap().get_mut(id) {
            inbox.asked = None;
            inbox.heard = false;
            for t in inbox.threads.values_mut() {
                t.asked = false;
            }
        }
    }

    /// [id] was unpaired: its texts go.
    pub fn forget(&self, id: &str) {
        self.inboxes.lock().unwrap().remove(id);
        self.changed();
    }
}

impl Inbox {
    /// The thread's people by name where they're contacts: "Sam, 555 0100".
    fn name_of(&self, t: &Thread) -> String {
        let names: Vec<String> = addresses(t).iter().map(|a| self.contacts.get(&key(a)).cloned().unwrap_or_else(|| a.clone())).collect();
        if names.is_empty() { "Unknown".into() } else { names.join(", ") }
    }
}

/// Everyone in the thread, from its newest text that names them.
fn addresses(t: &Thread) -> Vec<String> {
    t.texts.values().rev().find(|m| !m.addresses.is_empty()).map(|m| m.addresses.clone()).unwrap_or_default()
}

/// A text from `kdeconnect.sms.messages`: its thread, and the text.
fn parse_text(m: &Value) -> Option<(i64, Text)> {
    let num = |k: &str| m[k].as_i64().or_else(|| m[k].as_str().and_then(|s| s.parse().ok()));
    let thread = num("thread_id")?;
    // Android's Sms.MESSAGE_TYPE: 1 inbox, 2 sent, 3 draft, 4 outbox, 5 failed, 6 queued.
    let kind = num("type").unwrap_or(1);
    if kind == 3 {
        return None;
    }
    let addresses = m["addresses"].as_array().into_iter().flatten().filter_map(|a| a["address"].as_str()).map(str::trim).filter(|a| !a.is_empty()).map(str::to_owned).collect();
    Some((
        thread,
        Text {
            uid: num("_id").unwrap_or(0),
            body: m["body"].as_str().unwrap_or("").to_owned(),
            date: num("date").unwrap_or(0),
            mine: kind != 1,
            failed: kind == 5,
            read: num("read").unwrap_or(1) != 0,
            addresses,
        },
    ))
}

/// A number's last ten digits: "+91 98450 12345" and "098450 12345" are the same phone.
fn key(number: &str) -> String {
    let digits: String = number.chars().filter(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return number.trim().to_lowercase();
    }
    digits[digits.len().saturating_sub(10)..].to_owned()
}

/// A vCard's name (FN, else N) and phone numbers.
fn vcard(card: &str) -> Option<(String, Vec<String>)> {
    let (mut name, mut fallback, mut numbers) = (None, None, Vec::new());
    for line in unfold(card) {
        let Some((head, value)) = line.split_once(':') else { continue };
        let mut parts = head.split(';');
        let prop = parts.next().unwrap_or("").to_ascii_uppercase();
        let prop = prop.rsplit('.').next().unwrap_or("");
        let params: Vec<String> = parts.map(str::to_ascii_uppercase).collect();
        let value = if params.iter().any(|p| p.contains("QUOTED-PRINTABLE")) { quoted_printable(value) } else { value.to_owned() };
        let value = value.replace("\\,", ",").replace("\\;", ";").trim().to_owned();
        match prop {
            "FN" if !value.is_empty() => name = Some(value),
            "N" if !value.is_empty() => {
                let n: Vec<&str> = value.split(';').collect();
                let parts: Vec<&str> = [n.get(1), n.first()].into_iter().flatten().copied().filter(|s| !s.trim().is_empty()).collect();
                fallback = Some(parts.join(" "));
            }
            "TEL" if !value.is_empty() => numbers.push(value),
            _ => {}
        }
    }
    let name = name.or(fallback).filter(|n| !n.is_empty())?;
    (!numbers.is_empty()).then_some((name, numbers))
}

/// Joins folded lines (a leading space goes on) and quoted-printable soft breaks (a trailing `=`).
fn unfold(card: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut soft = false;
    for raw in card.replace("\r\n", "\n").split('\n') {
        match out.last_mut() {
            Some(last) if soft => {
                last.pop();
                last.push_str(raw.trim_start());
            }
            Some(last) if raw.starts_with(' ') || raw.starts_with('\t') => last.push_str(&raw[1..]),
            _ => out.push(raw.to_owned()),
        }
        let last = out.last().map(String::as_str).unwrap_or("");
        soft = last.ends_with('=') && last.split_once(':').is_some_and(|(h, _)| h.to_ascii_uppercase().contains("QUOTED-PRINTABLE"));
    }
    out
}

fn quoted_printable(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'='
            && i + 2 < b.len()
            && let Some(byte) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn packet(kind: &str, body: Value) -> Value {
    vyoma_core::link::packet(kind, body)
}

fn now_ms() -> i64 {
    vyoma_core::link::now_ms()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(list: Value) -> Value {
        json!({ "type": MESSAGES, "body": { "messages": list, "version": 2 } })
    }

    fn sms(thread: i64, uid: i64, date: i64, kind: i64, body: &str, to: &str) -> Value {
        json!({ "thread_id": thread, "_id": uid, "date": date, "type": kind, "read": 0, "body": body, "addresses": [{ "address": to }] })
    }

    #[test]
    fn asks_once_per_link_and_lists_newest_first() {
        let s = Store::default();
        let (json, ask) = s.conversations("p");
        assert_eq!(ask.len(), 2);
        assert_eq!(serde_json::from_str::<Value>(&json).unwrap()["state"], "loading");
        assert!(s.conversations("p").1.is_empty());
        s.on_packet("p", &texts(json!([sms(1, 10, 100, 1, "hi", "+1 555 0100")])));
        s.on_packet("p", &texts(json!([sms(2, 11, 200, 2, "yo", "555 0199")])));
        let v: Value = serde_json::from_str(&s.conversations("p").0).unwrap();
        assert_eq!(v["state"], "ready");
        assert_eq!(v["list"][0]["thread"], 2);
        assert_eq!((v["list"][0]["mine"].as_bool(), v["list"][1]["unread"].as_bool()), (Some(true), Some(true)));
        s.gone("p");
        assert_eq!(s.conversations("p").1.len(), 2);
    }

    #[test]
    fn names_people_from_their_cards() {
        let s = Store::default();
        let next = s.on_packet("p", &json!({ "type": CONTACTS_UIDS_REPLY, "body": { "uids": ["a"], "a": 5 } }));
        assert_eq!(next[0]["type"], CONTACTS_VCARDS);
        let card = "BEGIN:VCARD\nVERSION:2.1\nN:Lee;Sam;;;\nFN:Sam Lee\nTEL;CELL:+91 98450 12345\nEND:VCARD";
        s.on_packet("p", &json!({ "type": CONTACTS_VCARDS_REPLY, "body": { "uids": ["a"], "a": card } }));
        s.on_packet("p", &texts(json!([sms(1, 1, 1, 1, "hi", "09845012345")])));
        let v: Value = serde_json::from_str(&s.conversations("p").0).unwrap();
        assert_eq!(v["list"][0]["name"], "Sam Lee");
    }

    #[test]
    fn reads_vcards() {
        assert_eq!(vcard("FN:A B\nTEL;TYPE=CELL:1\nitem1.TEL:2"), Some(("A B".into(), vec!["1".into(), "2".into()])));
        assert_eq!(vcard("N:Lee;Sam;;;\nTEL:1"), Some(("Sam Lee".into(), vec!["1".into()])));
        assert_eq!(vcard("FN;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:=C3=A9mile=\n =C3=A9\nTEL:3"), Some(("émileé".into(), vec!["3".into()])));
        assert_eq!(vcard("FN:Nobody"), None);
        assert_eq!(key("+91 98450-12345"), "9845012345");
        assert_eq!(key("12345"), "12345");
    }

    #[test]
    fn a_sent_text_shows_as_sending_until_the_phone_has_it() {
        let s = Store::default();
        assert!(s.send("p", 1, "hi").is_err());
        s.on_packet("p", &texts(json!([sms(1, 1, 1, 1, "hello", "555")])));
        assert_eq!(s.send("p", 1, "  ").unwrap_err(), "Nothing to send");
        let p = s.send("p", 1, " see you ").unwrap();
        assert_eq!((p["type"].as_str(), p["body"]["messageBody"].as_str(), p["body"]["addresses"][0]["address"].as_str()), (Some(REQUEST), Some("see you"), Some("555")));
        let v: Value = serde_json::from_str(&s.thread("p", 1).0).unwrap();
        assert_eq!(v["texts"][1]["sending"], true);
        s.on_packet("p", &texts(json!([sms(1, 2, 2, 2, "see you", "555")])));
        let v: Value = serde_json::from_str(&s.thread("p", 1).0).unwrap();
        assert_eq!((v["texts"].as_array().unwrap().len(), v["texts"][1]["sending"].as_bool()), (2, Some(false)));
    }

    #[test]
    fn asks_for_a_thread_once() {
        let s = Store::default();
        let ask = s.thread("p", 7).1;
        assert_eq!((ask[0]["type"].as_str(), ask[0]["body"]["threadID"].as_i64()), (Some(REQUEST_CONVERSATION), Some(7)));
        assert!(s.thread("p", 7).1.is_empty());
    }
}
