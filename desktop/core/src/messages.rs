//! Texts on the phone, read and sent from the window's Messages page (Linux and the Mac), over the stock SMS and
//! contacts packets (kdeconnect.sms.*, kdeconnect.contacts.*), so any KDE Connect phone app works.
//! Kept in memory only, per phone: asked for when the page first opens on each link, then kept
//! current by the phone, which sends each new text once it's been asked. Plain SMS only (Android
//! doesn't let other apps read RCS chats). What texts say and who they're with never go to the log.
//!
//! Vyoma's phone app holds them behind its texts lock (`vyoma.texts.*`): opening the page asks the phone, the owner
//! confirms with a fingerprint there, and the phone refuses texts until then. Locked again (and everything dropped
//! from memory) when this PC's screen locks, after [IDLE_FOR] unused, or when the phone goes.

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
/// PC → phone: ask the owner to let the texts in.
pub const UNLOCK: &str = "vyoma.texts.unlock";
/// PC → phone: locked again.
pub const LOCK: &str = "vyoma.texts.lock";
/// Phone → PC: `{ok}`, the owner's answer.
pub const UNLOCKED: &str = "vyoma.texts.unlocked";
/// Phone → PC: a request was refused, as it's locked for us.
pub const LOCKED: &str = "vyoma.texts.locked";

/// How many conversations a search asks the phone for the texts of (the newest ones not loaded yet).
const SEARCH_LOADS: usize = 40;
/// At most this many contacts and conversations come back from a search.
const SEARCH_MOST: usize = 50;

/// How many of a thread's newest texts are asked for.
const THREAD_TEXTS: u64 = 200;
/// No answer after this: the phone isn't sharing its texts (or has none).
const SILENT_AFTER: Duration = Duration::from_secs(8);
/// A text sent from here shows as sending until the phone has it, or this long.
const PENDING_FOR: Duration = Duration::from_secs(90);
/// Unlocked Messages left unused this long locks again.
pub const IDLE_FOR: Duration = Duration::from_secs(10 * 60);
/// An ask isn't sent again sooner than this: a second ask makes the phone's new prompt replace (and deny) the first.
const ASK_AGAIN_AFTER: Duration = Duration::from_secs(30);
/// An ask the phone never answers ends after this: the page offers asking again.
const ASK_TIMEOUT: Duration = Duration::from_secs(2 * 60);

/// Where a phone with the texts lock stands.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Gate {
    #[default]
    Locked,
    /// Asked at this moment; the owner hasn't answered on the phone yet.
    Asking(Instant),
    /// The owner said no (or the prompt was dismissed).
    Declined,
    Open,
}

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
    /// A new conversation started here (a negative id): who it's to, until the phone has its first text.
    to: Vec<String>,
}

#[derive(Default)]
struct Inbox {
    threads: HashMap<i64, Thread>,
    /// Phone number's last digits to the contact's name.
    contacts: HashMap<String, String>,
    /// The phone's contacts by their uid: name and numbers.
    people: BTreeMap<String, (String, Vec<String>)>,
    /// A conversation started here (negative id) to the phone's own thread for it, once its first text is back.
    moved: HashMap<i64, i64>,
    /// When the conversations were asked for on this link; None until the page opens.
    asked: Option<Instant>,
    /// Anything came back since.
    heard: bool,
    gate: Gate,
    /// When Messages was last used, while open.
    used: Option<Instant>,
    /// The phone has the texts lock (it was asked once): it's held to [gate].
    guarded: bool,
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

    /// The conversations as JSON, newest first: `{state: loading|ready|silent|locked|asking|declined, list: [...]}`,
    /// and the packets to send the phone first if this link hasn't asked yet. [guarded]: the phone has the texts lock.
    pub fn conversations(&self, id: &str, guarded: bool) -> (String, Vec<Value>) {
        let mut all = self.inboxes.lock().unwrap();
        let inbox = all.entry(id.to_owned()).or_default();
        inbox.guarded |= guarded;
        if inbox.guarded && inbox.gate != Gate::Open {
            inbox.end_stale_ask();
            let state = match inbox.gate {
                Gate::Asking(_) => "asking",
                Gate::Declined => "declined",
                _ => "locked",
            };
            return (json!({ "state": state, "list": [] }).to_string(), Vec::new());
        }
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
    pub fn thread(&self, id: &str, thread: i64, guarded: bool) -> (String, Vec<Value>) {
        let mut all = self.inboxes.lock().unwrap();
        let inbox = all.entry(id.to_owned()).or_default();
        inbox.guarded |= guarded;
        if inbox.guarded && inbox.gate != Gate::Open {
            return (json!({ "name": "", "number": "", "texts": [] }).to_string(), Vec::new());
        }
        let thread = inbox.moved.get(&thread).copied().unwrap_or(thread);
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
        // One person: the window offers to call them.
        let call = match addresses(t).as_slice() {
            [one] => dialable(one).unwrap_or_default(),
            _ => String::new(),
        };
        (json!({ "name": inbox.name_of(t), "number": number, "call": call, "texts": texts }).to_string(), ask)
    }

    /// The phone's contacts with a number, by name: `{state, list: [{name, numbers}]}` (state as [Self::conversations]'s),
    /// and what to ask the phone first.
    pub fn contacts(&self, id: &str, guarded: bool) -> (String, Vec<Value>) {
        let (inbox_state, ask) = self.conversations(id, guarded);
        let state = serde_json::from_str::<Value>(&inbox_state).ok().and_then(|v| v["state"].as_str().map(str::to_owned)).unwrap_or_default();
        let all = self.inboxes.lock().unwrap();
        let mut list: Vec<&(String, Vec<String>)> = all.get(id).map(|i| i.people.values().filter(|(_, n)| !n.is_empty()).collect()).unwrap_or_default();
        list.sort_by_key(|(name, _)| name.to_lowercase());
        let list: Vec<Value> = list.into_iter().map(|(name, numbers)| json!({ "name": name, "numbers": numbers })).collect();
        (json!({ "state": state, "list": list }).to_string(), ask)
    }

    /// Conversations and contacts matching [query] (a name, a number, or words in a text): `{chats: [...], people: [...]}`,
    /// chats as [Self::conversations] lists them, with the matching text as the snippet. Only texts on this PC can match,
    /// so a search also asks the phone for the texts of the newest conversations not loaded yet: the results grow as they come.
    pub fn search(&self, id: &str, query: &str, guarded: bool) -> (String, Vec<Value>) {
        let empty = || json!({ "chats": [], "people": [] }).to_string();
        let query = query.trim().to_lowercase();
        let mut all = self.inboxes.lock().unwrap();
        let inbox = all.entry(id.to_owned()).or_default();
        inbox.guarded |= guarded;
        if query.is_empty() || (inbox.guarded && inbox.gate != Gate::Open) {
            return (empty(), Vec::new());
        }
        if inbox.guarded {
            inbox.used = Some(Instant::now());
        }
        let digits: String = query.chars().filter(char::is_ascii_digit).collect();
        // A number matches by its digits ("555 0100" finds "+1 (555) 010-0123"), words by their letters.
        let number_like = digits.len() >= 3 && query.chars().all(|c| c.is_ascii_digit() || " +-().".contains(c));
        let hits_number = |n: &str| number_like && n.chars().filter(char::is_ascii_digit).collect::<String>().contains(&digits);
        let hits = |text: &str| text.to_lowercase().contains(&query);
        let mut chats: Vec<(i64, Value)> = Vec::new();
        for (thread, t) in &inbox.threads {
            let name = inbox.name_of(t);
            let who = hits(&name) || addresses(t).iter().any(|a| hits_number(a));
            let text = t.texts.values().rev().find(|m| hits(&m.body));
            let shown = match (text, t.texts.values().next_back()) {
                (Some(m), _) => m,
                (None, Some(last)) if who => last,
                _ => continue,
            };
            let unread = t.texts.values().next_back().is_some_and(|l| !l.mine && !l.read);
            chats.push((shown.date, json!({ "thread": thread, "name": name, "snippet": shown.body, "date": shown.date, "mine": shown.mine, "unread": unread })));
        }
        chats.sort_by(|a, b| b.0.cmp(&a.0));
        chats.truncate(SEARCH_MOST);
        let mut people: Vec<&(String, Vec<String>)> =
            inbox.people.values().filter(|(name, numbers)| !numbers.is_empty() && (hits(name) || numbers.iter().any(|n| hits_number(n)))).collect();
        people.sort_by_key(|(name, _)| name.to_lowercase());
        people.truncate(SEARCH_MOST);
        let people: Vec<Value> = people.into_iter().map(|(name, numbers)| json!({ "name": name, "numbers": numbers })).collect();
        let mut newest: Vec<(i64, i64)> = inbox.threads.iter().filter(|(id, t)| **id > 0 && !t.asked).map(|(id, t)| (t.texts.values().next_back().map_or(0, |m| m.date), *id)).collect();
        newest.sort_by(|a, b| b.0.cmp(&a.0));
        let mut ask = Vec::new();
        for (_, thread) in newest.into_iter().take(SEARCH_LOADS) {
            inbox.threads.get_mut(&thread).expect("listed").asked = true;
            ask.push(packet(REQUEST_CONVERSATION, json!({ "threadID": thread, "rangeStartTimestamp": -1, "numberToRequest": THREAD_TEXTS })));
        }
        let chats: Vec<Value> = chats.into_iter().map(|(_, v)| v).collect();
        (json!({ "chats": chats, "people": people }).to_string(), ask)
    }

    /// The conversation with [number]: the phone's own if there's one with just them, else a new one started here
    /// (a negative id) that [Self::send] can text.
    pub fn thread_for(&self, id: &str, number: &str, guarded: bool) -> Result<i64, String> {
        let number = number.trim();
        if number.is_empty() {
            return Err("No number".into());
        }
        let mut all = self.inboxes.lock().unwrap();
        let inbox = all.entry(id.to_owned()).or_default();
        inbox.guarded |= guarded;
        if inbox.guarded && inbox.gate != Gate::Open {
            return Err("Messages is locked: confirm on your phone first".into());
        }
        let want = key(number);
        if let Some((thread, _)) = inbox.threads.iter().filter(|(_, t)| !t.texts.is_empty() || !t.to.is_empty()).find(|(_, t)| matches!(addresses(t).as_slice(), [one] if key(one) == want)) {
            return Ok(*thread);
        }
        let thread = inbox.threads.keys().copied().min().unwrap_or(0).min(0) - 1;
        inbox.threads.insert(thread, Thread { asked: true, to: vec![number.to_owned()], ..Thread::default() });
        Ok(thread)
    }

    /// Whether the window may ask [id]'s phone to call someone: only while its texts are open (the owner confirmed on
    /// the phone, which checks that again itself).
    pub fn may_call(&self, id: &str, guarded: bool) -> Result<(), String> {
        let mut all = self.inboxes.lock().unwrap();
        let inbox = all.entry(id.to_owned()).or_default();
        inbox.guarded |= guarded;
        if inbox.guarded {
            if inbox.gate != Gate::Open {
                return Err("Messages is locked: confirm on your phone first".into());
            }
            inbox.used = Some(Instant::now());
        }
        Ok(())
    }

    /// Sends [body] to [thread]'s people: the packet for the phone, or why it can't.
    pub fn send(&self, id: &str, thread: i64, body: &str, guarded: bool) -> Result<Value, String> {
        let body = body.trim();
        if body.is_empty() {
            return Err("Nothing to send".into());
        }
        let mut all = self.inboxes.lock().unwrap();
        let inbox = all.get_mut(id).ok_or("That conversation isn't loaded")?;
        inbox.guarded |= guarded;
        if inbox.guarded {
            if inbox.gate != Gate::Open {
                return Err("Messages is locked: confirm on your phone first".into());
            }
            inbox.used = Some(Instant::now());
        }
        let thread = inbox.moved.get(&thread).copied().unwrap_or(thread);
        let t = inbox.threads.get_mut(&thread).ok_or("That conversation isn't loaded")?;
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
            // Only an answer to an ask that's still out counts: a late or unasked yes doesn't open the texts.
            UNLOCKED => {
                inbox.end_stale_ask();
                if !matches!(inbox.gate, Gate::Asking(_)) {
                    return out;
                }
                if b["ok"].as_bool() == Some(true) {
                    inbox.gate = Gate::Open;
                    inbox.used = Some(Instant::now());
                } else {
                    inbox.gate = Gate::Declined;
                }
            }
            LOCKED => inbox.lock(),
            // Whatever comes while locked (a request that crossed the lock) isn't kept.
            _ if inbox.gate != Gate::Open && inbox.guarded => return out,
            MESSAGES => {
                inbox.heard = true;
                for m in b["messages"].as_array().into_iter().flatten() {
                    let Some((thread, text)) = parse_text(m) else { continue };
                    if text.mine {
                        inbox.adopt(thread, &text);
                    }
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
                        for n in &numbers {
                            inbox.contacts.insert(key(n), name.clone());
                        }
                        inbox.people.insert(uid.to_owned(), (name, numbers));
                    }
                }
            }
            _ => return out,
        }
        drop(all);
        self.changed();
        out
    }

    /// The page was opened, a conversation picked or a text sent: for a locked phone, the packet asking its owner
    /// (the window shows "Confirm on your phone"); for an open one, it stays open [IDLE_FOR] more.
    pub fn unlock(&self, id: &str) -> Option<Value> {
        let mut all = self.inboxes.lock().unwrap();
        let inbox = all.entry(id.to_owned()).or_default();
        inbox.guarded = true;
        if inbox.gate == Gate::Open {
            inbox.used = Some(Instant::now());
            return None;
        }
        // Still waiting on the owner: asking again would put a new prompt over the one they may be answering.
        if let Gate::Asking(at) = inbox.gate
            && at.elapsed() < ASK_AGAIN_AFTER
        {
            return None;
        }
        inbox.gate = Gate::Asking(Instant::now());
        drop(all);
        self.changed();
        Some(packet(UNLOCK, json!({})))
    }

    /// Locks every phone's texts (this PC's screen locked): the phones to tell.
    pub fn lock_all(&self) -> Vec<String> {
        self.lock_where(|_| true)
    }

    /// Locks the ones left unused for [IDLE_FOR]: the phones to tell. Asks the phone never answered end here too.
    pub fn expire(&self) -> Vec<String> {
        let mut all = self.inboxes.lock().unwrap();
        let ended = all.values_mut().fold(false, |any, i| i.end_stale_ask() || any);
        drop(all);
        if ended {
            self.changed();
        }
        self.lock_where(|i| i.used.is_some_and(|at| at.elapsed() >= IDLE_FOR))
    }

    fn lock_where(&self, pick: impl Fn(&Inbox) -> bool) -> Vec<String> {
        let mut all = self.inboxes.lock().unwrap();
        let mut told = Vec::new();
        for (id, inbox) in all.iter_mut() {
            if inbox.guarded && matches!(inbox.gate, Gate::Open | Gate::Asking(_)) && pick(inbox) {
                inbox.lock();
                told.push(id.clone());
            }
        }
        drop(all);
        if !told.is_empty() {
            self.changed();
        }
        told
    }

    /// [id]'s link is gone: the next look asks again (what came meanwhile is news). A locked phone's texts go
    /// from memory and it locks: it forgets the owner's yes as the link ends too.
    pub fn gone(&self, id: &str) {
        let mut all = self.inboxes.lock().unwrap();
        let Some(inbox) = all.get_mut(id) else { return };
        if inbox.guarded {
            inbox.lock();
            drop(all);
            self.changed();
            return;
        }
        inbox.asked = None;
        inbox.heard = false;
        for t in inbox.threads.values_mut() {
            t.asked = false;
        }
    }

    /// [id] was unpaired: its texts go.
    pub fn forget(&self, id: &str) {
        self.inboxes.lock().unwrap().remove(id);
        self.changed();
    }
}

impl Inbox {
    /// An ask out longer than [ASK_TIMEOUT] ends (the phone never answered): locked, so the page offers asking again.
    fn end_stale_ask(&mut self) -> bool {
        let stale = matches!(self.gate, Gate::Asking(at) if at.elapsed() >= ASK_TIMEOUT);
        if stale {
            self.gate = Gate::Locked;
        }
        stale
    }

    /// The phone's [thread] has [text], sent from here: if it began as a conversation started here, that one is now
    /// [thread] (its pending texts go with it, and the window's open page follows it there).
    fn adopt(&mut self, thread: i64, text: &Text) {
        let Some(started) = self.threads.iter().find(|(id, t)| **id < 0 && t.pending.iter().any(|(body, _)| *body == text.body) && t.to.iter().any(|a| text.addresses.iter().any(|b| key(a) == key(b)))).map(|(id, _)| *id) else {
            return;
        };
        let old = self.threads.remove(&started).expect("found");
        let t = self.threads.entry(thread).or_default();
        t.pending.extend(old.pending);
        self.moved.insert(started, thread);
    }

    /// Locked: nothing of the texts or the contacts stays.
    fn lock(&mut self) {
        *self = Inbox { guarded: self.guarded, ..Inbox::default() };
    }

    /// The thread's people by name where they're contacts: "Sam, 555 0100".
    fn name_of(&self, t: &Thread) -> String {
        let names: Vec<String> = addresses(t).iter().map(|a| self.contacts.get(&key(a)).cloned().unwrap_or_else(|| a.clone())).collect();
        if names.is_empty() { "Unknown".into() } else { names.join(", ") }
    }
}

/// Everyone in the thread, from its newest text that names them (a conversation started here: who it's to).
fn addresses(t: &Thread) -> Vec<String> {
    t.texts.values().rev().find(|m| !m.addresses.is_empty()).map(|m| m.addresses.clone()).unwrap_or_else(|| t.to.clone())
}

/// [number] as a phone can call it (digits, a leading +), or None. Only a plain number: never the * and # of codes
/// that change the phone's settings (call forwarding...), nor pauses, letters or anything else.
pub fn dialable(number: &str) -> Option<String> {
    let number = number.trim();
    let mut out = String::new();
    for (i, c) in number.chars().enumerate() {
        match c {
            '0'..='9' => out.push(c),
            '+' if i == 0 => out.push(c),
            ' ' | '-' | '(' | ')' | '.' => {}
            _ => return None,
        }
    }
    let digits = out.trim_start_matches('+').len();
    (3..=20).contains(&digits).then_some(out)
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
    crate::link::packet(kind, body)
}

fn now_ms() -> i64 {
    crate::link::now_ms()
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
        let (json, ask) = s.conversations("p", false);
        assert_eq!(ask.len(), 2);
        assert_eq!(serde_json::from_str::<Value>(&json).unwrap()["state"], "loading");
        assert!(s.conversations("p", false).1.is_empty());
        s.on_packet("p", &texts(json!([sms(1, 10, 100, 1, "hi", "+1 555 0100")])));
        s.on_packet("p", &texts(json!([sms(2, 11, 200, 2, "yo", "555 0199")])));
        let v: Value = serde_json::from_str(&s.conversations("p", false).0).unwrap();
        assert_eq!(v["state"], "ready");
        assert_eq!(v["list"][0]["thread"], 2);
        assert_eq!((v["list"][0]["mine"].as_bool(), v["list"][1]["unread"].as_bool()), (Some(true), Some(true)));
        s.gone("p");
        assert_eq!(s.conversations("p", false).1.len(), 2);
    }

    #[test]
    fn names_people_from_their_cards() {
        let s = Store::default();
        let next = s.on_packet("p", &json!({ "type": CONTACTS_UIDS_REPLY, "body": { "uids": ["a"], "a": 5 } }));
        assert_eq!(next[0]["type"], CONTACTS_VCARDS);
        let card = "BEGIN:VCARD\nVERSION:2.1\nN:Lee;Sam;;;\nFN:Sam Lee\nTEL;CELL:+91 98450 12345\nEND:VCARD";
        s.on_packet("p", &json!({ "type": CONTACTS_VCARDS_REPLY, "body": { "uids": ["a"], "a": card } }));
        s.on_packet("p", &texts(json!([sms(1, 1, 1, 1, "hi", "09845012345")])));
        let v: Value = serde_json::from_str(&s.conversations("p", false).0).unwrap();
        assert_eq!(v["list"][0]["name"], "Sam Lee");
    }

    #[test]
    fn only_plain_numbers_can_be_called() {
        assert_eq!(dialable(" +1 (555) 010-0123 "), Some("+15550100123".into()));
        assert_eq!(dialable("112"), Some("112".into()));
        for code in ["*21*5550100#", "*#06#", "555,123", "555;1", "1-800-FLOWERS", "5+55", "12", ""] {
            assert_eq!(dialable(code), None, "{code}");
        }
    }

    fn card(s: &Store, uid: &str, name: &str, tel: &str) {
        s.on_packet("p", &json!({ "type": CONTACTS_VCARDS_REPLY, "body": { "uids": [uid], uid: format!("BEGIN:VCARD\nFN:{name}\nTEL:{tel}\nEND:VCARD") } }));
    }

    #[test]
    fn searches_names_numbers_and_words_and_loads_more_to_search() {
        let s = Store::default();
        s.conversations("p", false);
        card(&s, "a", "Sam Lee", "+1 555 0100");
        card(&s, "b", "Ana", "555 0199");
        s.on_packet("p", &texts(json!([sms(1, 1, 100, 1, "the pizza place", "+1 555 0100"), sms(2, 2, 200, 1, "hello", "555 0177")])));
        let find = |q: &str| -> (Value, usize) {
            let (json, ask) = s.search("p", q, false);
            (serde_json::from_str(&json).unwrap(), ask.len())
        };
        let (v, asked) = find("PIZZA");
        assert_eq!((v["chats"][0]["thread"].as_i64(), v["chats"][0]["snippet"].as_str()), (Some(1), Some("the pizza place")));
        assert_eq!(asked, 2, "the first search loads the conversations' texts");
        assert_eq!(find("pizza").1, 0, "once");
        let (v, _) = find("555 01");
        assert_eq!(v["people"].as_array().unwrap().len(), 2);
        assert_eq!(find("0100").0["chats"][0]["thread"], 1, "by its number");
        assert_eq!(find("sam").0["people"][0]["name"], "Sam Lee");
        assert_eq!(find("  ").0["chats"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn texts_someone_new_and_follows_the_phone_to_its_thread() {
        let s = Store::default();
        s.conversations("p", false);
        s.on_packet("p", &texts(json!([sms(7, 1, 100, 1, "hi", "+1 415 555 0100")])));
        assert_eq!(s.thread_for("p", "415-555-0100", false), Ok(7), "their conversation");
        let new = s.thread_for("p", "+1 415 555 0142", false).unwrap();
        assert!(new < 0);
        let sent = s.send("p", new, "hello there", false).unwrap();
        assert_eq!(sent["body"]["addresses"][0]["address"], "+1 415 555 0142");
        let v: Value = serde_json::from_str(&s.conversations("p", false).0).unwrap();
        assert_eq!(v["list"][0]["snippet"], "hello there");
        s.on_packet("p", &texts(json!([sms(9, 2, 300, 2, "hello there", "+14155550142")])));
        let t: Value = serde_json::from_str(&s.thread("p", new, false).0).unwrap();
        assert_eq!((t["texts"].as_array().unwrap().len(), t["call"].as_str()), (1, Some("+14155550142")));
        assert_eq!(s.thread_for("p", "4155550142", false), Ok(9));
    }

    #[test]
    fn contacts_and_calls_wait_for_the_owner() {
        let s = Store::default();
        assert!(s.may_call("p", true).is_err());
        assert_eq!(serde_json::from_str::<Value>(&s.contacts("p", true).0).unwrap()["state"], "locked");
        s.unlock("p");
        s.on_packet("p", &json!({ "type": UNLOCKED, "body": { "ok": true } }));
        assert!(s.may_call("p", true).is_ok());
        card(&s, "b", "ana", "1");
        card(&s, "a", "Bo", "2");
        card(&s, "c", "No number", "");
        let v: Value = serde_json::from_str(&s.contacts("p", true).0).unwrap();
        assert_eq!(v["list"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect::<Vec<_>>(), ["ana", "Bo"]);
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
        assert!(s.send("p", 1, "hi", false).is_err());
        s.on_packet("p", &texts(json!([sms(1, 1, 1, 1, "hello", "555")])));
        assert_eq!(s.send("p", 1, "  ", false).unwrap_err(), "Nothing to send");
        let p = s.send("p", 1, " see you ", false).unwrap();
        assert_eq!((p["type"].as_str(), p["body"]["messageBody"].as_str(), p["body"]["addresses"][0]["address"].as_str()), (Some(REQUEST), Some("see you"), Some("555")));
        let v: Value = serde_json::from_str(&s.thread("p", 1, false).0).unwrap();
        assert_eq!(v["texts"][1]["sending"], true);
        s.on_packet("p", &texts(json!([sms(1, 2, 2, 2, "see you", "555")])));
        let v: Value = serde_json::from_str(&s.thread("p", 1, false).0).unwrap();
        assert_eq!((v["texts"].as_array().unwrap().len(), v["texts"][1]["sending"].as_bool()), (2, Some(false)));
    }

    #[test]
    fn asks_for_a_thread_once() {
        let s = Store::default();
        let ask = s.thread("p", 7, false).1;
        assert_eq!((ask[0]["type"].as_str(), ask[0]["body"]["threadID"].as_i64()), (Some(REQUEST_CONVERSATION), Some(7)));
        assert!(s.thread("p", 7, false).1.is_empty());
    }

    #[test]
    fn a_locked_phone_shows_nothing_until_its_owner_says_yes() {
        let s = Store::default();
        let state = |s: &Store| serde_json::from_str::<Value>(&s.conversations("p", true).0).unwrap()["state"].clone();
        assert_eq!(state(&s), "locked");
        assert!(s.conversations("p", true).1.is_empty(), "nothing asked of a locked phone");
        assert!(s.send("p", 1, "hi", true).is_err());
        assert_eq!(s.unlock("p").unwrap()["type"], UNLOCK);
        assert_eq!(state(&s), "asking");
        s.on_packet("p", &json!({ "type": UNLOCKED, "body": { "ok": false } }));
        assert_eq!(state(&s), "declined");
        s.unlock("p");
        s.on_packet("p", &json!({ "type": UNLOCKED, "body": { "ok": true } }));
        assert_eq!(s.conversations("p", true).1.len(), 2, "open: asks the phone");
        s.on_packet("p", &texts(json!([sms(1, 10, 100, 1, "hi", "555")])));
        assert_eq!(state(&s), "ready");
        assert!(s.unlock("p").is_none(), "already open");
        assert_eq!(s.lock_all(), vec!["p".to_owned()]);
        assert_eq!(state(&s), "locked");
        assert!(s.lock_all().is_empty());
        // Texts that crossed the lock aren't kept.
        s.on_packet("p", &texts(json!([sms(1, 11, 200, 1, "late", "555")])));
        s.unlock("p");
        s.on_packet("p", &json!({ "type": UNLOCKED, "body": { "ok": true } }));
        let v: Value = serde_json::from_str(&s.conversations("p", true).0).unwrap();
        assert_eq!(v["list"].as_array().unwrap().len(), 0);
        s.on_packet("p", &json!({ "type": LOCKED, "body": {} }));
        assert_eq!(state(&s), "locked");
        s.unlock("p");
        s.on_packet("p", &json!({ "type": UNLOCKED, "body": { "ok": true } }));
        s.gone("p");
        assert_eq!(state(&s), "locked");
    }

    #[test]
    fn asks_once_and_only_an_asked_yes_opens() {
        let s = Store::default();
        let state = |s: &Store| serde_json::from_str::<Value>(&s.conversations("p", true).0).unwrap()["state"].clone();
        // A yes nobody asked for doesn't open the texts.
        s.conversations("p", true);
        s.on_packet("p", &json!({ "type": UNLOCKED, "body": { "ok": true } }));
        assert_eq!(state(&s), "locked");
        assert!(s.unlock("p").is_some());
        assert!(s.unlock("p").is_none(), "no second prompt over the first");
        let set_asked = |s: &Store, ago: Duration| s.inboxes.lock().unwrap().get_mut("p").unwrap().gate = Gate::Asking(Instant::now().checked_sub(ago).unwrap());
        set_asked(&s, ASK_AGAIN_AFTER);
        assert!(s.unlock("p").is_some(), "asked again after a while");
        // Never answered: back to where the page offers asking again.
        set_asked(&s, ASK_TIMEOUT);
        assert!(s.expire().is_empty(), "nothing to tell the phone");
        assert_eq!(state(&s), "locked");
        s.on_packet("p", &json!({ "type": UNLOCKED, "body": { "ok": true } }));
        assert_eq!(state(&s), "locked", "a yes after the ask ended doesn't count");
    }
}
