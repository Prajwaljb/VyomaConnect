//! The stock KDE Connect plugins, the PC's part of each, once for every PC side (as the phone's
//! plugins expect them: core/protocol's SimplePlugins.kt and PhonePlugins.kt). What a packet means
//! is here; how this computer shows, rings and locks is a [Desk]'s.
//!
//! So far: ping, battery both ways, ringing this PC, its screen lock, the clipboard from the phone,
//! and what it shares (text, web links, files into the Downloads folder, with what arrived said once
//! a batch is in).

use std::collections::HashMap;
use std::future::Future;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::Result;
use serde_json::{Value, json};
use vyoma_core::identity::Identity;
use vyoma_core::link::packet;
use vyoma_core::permits::DENIED;
use vyoma_core::share::{self as sharing, Shared};

pub use vyoma_core::clipboard::{CLIPBOARD, CLIPBOARD_CONNECT, Clipboard};
pub use vyoma_core::share::{SHARE, SHARE_UPDATE};

pub const PING: &str = "kdeconnect.ping";
pub const BATTERY: &str = "kdeconnect.battery";
pub const FIND: &str = "kdeconnect.findmyphone.request";
/// PC → phone: whether this PC is locked.
pub const LOCK: &str = "kdeconnect.lock";
/// Phone → PC: `setLocked`, `requestLocked`.
pub const LOCK_REQUEST: &str = "kdeconnect.lock.request";

/// This computer's battery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Battery {
    pub charge: i64,
    pub charging: bool,
}

/// The phone's battery as it last said: charge, charging, and whether it said it's low.
pub type PhoneBattery = (i64, bool, bool);

pub fn battery_packet(b: Battery) -> Value {
    // Low is the phone's to judge for itself; a PC sends no threshold event, as kdeconnectd.
    packet(BATTERY, json!({ "currentCharge": b.charge, "isCharging": b.charging, "thresholdEvent": 0 }))
}

pub fn lock_packet(locked: bool) -> Value {
    packet(LOCK, json!({ "isLocked": locked }))
}

/// The phone's battery from its packet; None when it didn't say a charge (or said nonsense).
pub fn phone_battery(body: &Value) -> Option<PhoneBattery> {
    let charge = body["currentCharge"].as_i64().or_else(|| body["currentCharge"].as_str()?.parse().ok())?;
    (0..=100).contains(&charge).then(|| (charge, body["isCharging"].as_bool().unwrap_or(false), body["thresholdEvent"].as_i64() == Some(1)))
}

/// How this computer does what the stock plugins ask: each PC side's, borrowing what it needs
/// (its notifier, its ringer) for the one packet.
pub trait Desk: Send {
    /// A short notification from the phone.
    fn tell(&mut self, title: &str, body: &str) -> impl Future<Output = ()> + Send;
    /// Rings so the person finds this computer, saying [phone] is looking (with a way to stop).
    fn ring(&mut self, phone: &str) -> impl Future<Output = ()> + Send;
    /// Locks (or unlocks) the screen; None where this computer can't do that (unlocking, mostly).
    fn set_locked(&mut self, lock: bool) -> impl Future<Output = Option<Result<()>>> + Send;
    /// Whether the screen is locked, where that can be told.
    fn locked(&mut self) -> impl Future<Output = Option<bool>> + Send {
        async { None }
    }
    /// Puts the phone's [text] on this computer's clipboard.
    fn copy(&mut self, text: &str) -> impl Future<Output = Result<()>> + Send;
    /// Opens a web link the phone shared (only http and https come here).
    fn open_link(&mut self, url: &str) -> impl Future<Output = ()> + Send;
    /// Text the phone shared: copied and said, unless the desk offers it instead (a Copy button).
    /// True when it went on the clipboard.
    fn shared_text(&mut self, phone: &str, text: &str) -> impl Future<Output = bool> + Send {
        async move {
            match self.copy(text).await {
                Ok(()) => {
                    self.tell(&format!("Text from {phone}"), "Copied. Paste it anywhere.").await;
                    true
                }
                Err(e) => {
                    tracing::warn!("couldn't copy what {phone} shared: {e:#}");
                    false
                }
            }
        }
    }
}

/// One phone as the stock plugins keep it.
#[derive(Clone, Debug, Default)]
pub struct Phone {
    /// Its name as it's safe to show.
    pub name: String,
    pub battery: Option<PhoneBattery>,
}

/// A file the phone is sending, for the PC side to fetch ([fetch]).
#[derive(Clone, Debug, PartialEq)]
pub struct Incoming {
    pub name: String,
    /// How many files its batch has.
    pub count: u64,
}

/// What a packet needs from the PC side after [on_packet]: packets to send back, a file to fetch.
#[derive(Debug, Default, PartialEq)]
pub struct Outcome {
    pub send: Vec<Value>,
    pub fetch: Option<Incoming>,
}

impl From<Vec<Value>> for Outcome {
    fn from(send: Vec<Value>) -> Outcome {
        Outcome { send, fetch: None }
    }
}

/// Whether [kind] is one of the plugins here.
pub fn handles(kind: &str) -> bool {
    matches!(kind, PING | BATTERY | FIND | LOCK_REQUEST | CLIPBOARD | CLIPBOARD_CONNECT | SHARE | SHARE_UPDATE)
}

/// Does what packet [p] from [phone] asks; what's left for the PC side to do. [clip] is this PC's
/// clipboard as the phones know it.
pub async fn on_packet(desk: &mut impl Desk, phone: &mut Phone, clip: &mut Clipboard, p: &Value) -> Outcome {
    let (name, battery) = (phone.name.as_str(), &mut phone.battery);
    let body = &p["body"];
    match p["type"].as_str().unwrap_or("") {
        CLIPBOARD | CLIPBOARD_CONNECT => {
            if let Some(text) = clip.received(p) {
                tracing::info!("{name}: clipboard, {} chars", text.chars().count());
                if let Err(e) = desk.copy(&text).await {
                    tracing::warn!("couldn't set the clipboard: {e:#}");
                }
            }
        }
        SHARE => match sharing::classify(p) {
            Shared::File { name, count } => return Outcome { send: Vec::new(), fetch: Some(Incoming { name, count }) },
            Shared::Url(url) => desk.open_link(&url).await,
            Shared::Text(text) => {
                if desk.shared_text(name, &text).await {
                    // Ours now: the clipboard's watch mustn't send it straight back.
                    clip.changed(text);
                }
            }
            Shared::Nothing => {}
        },
        SHARE_UPDATE => {}
        PING => desk.tell(name, body["message"].as_str().unwrap_or("Ping!")).await,
        BATTERY => {
            let was_low = battery.is_some_and(|b| b.2);
            *battery = phone_battery(body);
            if let Some((charge, _, true)) = *battery
                && !was_low
            {
                desk.tell(&format!("{name}'s battery is low"), &format!("{charge}% left")).await;
            }
        }
        FIND => desk.ring(name).await,
        LOCK_REQUEST => {
            if let Some(lock) = body["setLocked"].as_bool() {
                match desk.set_locked(lock).await {
                    Some(Ok(())) => tracing::info!("{name}: {} this PC", if lock { "locked" } else { "unlocked" }),
                    Some(Err(e)) => {
                        tracing::warn!("{name}: {e:#}");
                        // Said on the phone, where Lock was pressed (`reason`: the PC can't, not a switch).
                        return vec![packet(DENIED, json!({ "request": LOCK_REQUEST, "name": "Lock", "reason": format!("{e:#}") }))].into();
                    }
                    None => tracing::debug!("{name}: this PC can't be {} from the phone", if lock { "locked" } else { "unlocked" }),
                }
            }
            if body["requestLocked"].as_bool() == Some(true)
                && let Some(locked) = desk.locked().await
            {
                return vec![lock_packet(locked)].into();
            }
        }
        _ => {}
    }
    Outcome::default()
}

/// Files arriving from phones, counted per phone so a batch is said once it's all in.
#[derive(Default)]
pub struct Arrivals(Mutex<HashMap<String, (u64, u64)>>);

/// A file fetched (or not), and where its batch stands.
#[derive(Debug)]
pub struct Landed {
    /// The phone's device id and name.
    pub id: String,
    pub phone: String,
    /// The name it was sent with.
    pub name: String,
    /// Where it landed, or why it didn't.
    pub result: Result<PathBuf>,
    /// Files of the batch done (landed or not), landed, and how many it has.
    pub done: u64,
    pub ok: u64,
    pub count: u64,
    /// For the activity log, once the whole batch is done and something of it landed.
    pub activity: Option<String>,
}

impl Landed {
    /// The landed file's own name here.
    pub fn file_name(&self) -> String {
        self.result.as_ref().ok().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| self.name.clone())
    }

    /// A plain notification's title and text for it, where a desk has no progress to update: a
    /// lone file as it lands, a batch once it's all done (None until then).
    pub fn summary(&self) -> Option<(String, String)> {
        let phone = &self.phone;
        if self.count <= 1 {
            return Some(match &self.result {
                Ok(_) => (format!("From {phone}"), format!("{} is in Downloads", self.file_name())),
                Err(_) => (format!("{} didn't arrive", self.name), "The phone stopped sending it. Try again.".into()),
            });
        }
        if self.done < self.count {
            return None;
        }
        Some(match self.ok {
            0 => (format!("{} files didn't arrive", self.count), "The phone stopped sending them. Try again.".into()),
            ok if ok == self.count => (format!("From {phone}"), format!("{ok} files are in Downloads")),
            ok => (format!("From {phone}"), format!("{ok} of {} files are in Downloads; the rest didn't arrive", self.count)),
        })
    }
}

impl Arrivals {
    /// Counts one file of [incoming]'s batch from phone [id] as done; [this] names this computer
    /// for the activity log ("this PC", "this Mac").
    pub fn landed(&self, id: &str, phone: &str, incoming: Incoming, result: Result<PathBuf>, this: &str) -> Landed {
        let mut batches = self.0.lock().unwrap();
        let entry = batches.entry(id.to_owned()).or_default();
        entry.0 += 1;
        if result.is_ok() {
            entry.1 += 1;
        }
        let (done, ok) = *entry;
        let finished = done >= incoming.count;
        if finished {
            batches.remove(id);
        }
        let mut landed = Landed { id: id.to_owned(), phone: phone.to_owned(), name: incoming.name, result, done, ok, count: incoming.count, activity: None };
        landed.activity = (finished && ok > 0).then(|| {
            if incoming.count <= 1 { format!("Shared \u{201c}{}\u{201d} with {this}", landed.file_name()) } else { format!("Shared {ok} files with {this}") }
        });
        landed
    }
}

/// Fetches [incoming] (sent in packet [p]) from the phone at [from] into [dir], counting it in
/// [arrivals].
#[allow(clippy::too_many_arguments)]
pub async fn fetch(me: &Identity, pinned: &[u8], from: IpAddr, p: &Value, dir: &Path, incoming: Incoming, arrivals: &Arrivals, id: &str, phone: &str, this: &str) -> Landed {
    let result = sharing::receive(me, pinned, from, p, dir).await;
    if let Err(e) = &result {
        tracing::warn!("{id}: {} not received: {e:#}", incoming.name);
    }
    arrivals.landed(id, phone, incoming, result, this)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A desk that writes down what it was asked.
    #[derive(Default)]
    struct Notes {
        said: Vec<String>,
        locked: Option<bool>,
        lock_fails: bool,
        unlocks: bool,
    }

    impl Desk for Notes {
        async fn tell(&mut self, title: &str, body: &str) {
            self.said.push(format!("{title}: {body}"));
        }
        async fn ring(&mut self, phone: &str) {
            self.said.push(format!("ring for {phone}"));
        }
        async fn set_locked(&mut self, lock: bool) -> Option<Result<()>> {
            if !lock && !self.unlocks {
                return None;
            }
            if self.lock_fails {
                return Some(Err(anyhow::anyhow!("no screen locker")));
            }
            self.locked = Some(lock);
            Some(Ok(()))
        }
        async fn locked(&mut self) -> Option<bool> {
            self.locked
        }
        async fn copy(&mut self, text: &str) -> Result<()> {
            self.said.push(format!("copied {text}"));
            Ok(())
        }
        async fn open_link(&mut self, url: &str) {
            self.said.push(format!("opened {url}"));
        }
    }

    /// The one call the old tests make: the packets sent back.
    async fn on(desk: &mut Notes, battery: &mut Option<PhoneBattery>, p: &Value) -> Vec<Value> {
        let mut phone = Phone { name: "Pixel".into(), battery: *battery };
        let out = on_packet(desk, &mut phone, &mut Clipboard::default(), p).await;
        *battery = phone.battery;
        out.send
    }

    fn p(kind: &str, body: Value) -> Value {
        packet(kind, body)
    }

    #[tokio::test]
    async fn pings_ring_and_say_a_low_battery_once() {
        let (mut desk, mut battery) = (Notes::default(), None);
        on(&mut desk, &mut battery, &p(PING, json!({}))).await;
        on(&mut desk, &mut battery, &p(PING, json!({ "message": "hi" }))).await;
        on(&mut desk, &mut battery, &p(FIND, json!({}))).await;
        on(&mut desk, &mut battery, &p(BATTERY, json!({ "currentCharge": 50, "isCharging": true }))).await;
        assert_eq!(battery, Some((50, true, false)));
        for charge in [14, 13] {
            on(&mut desk, &mut battery, &p(BATTERY, json!({ "currentCharge": charge, "thresholdEvent": 1 }))).await;
        }
        // Old phones send the charge as a string.
        on(&mut desk, &mut battery, &p(BATTERY, json!({ "currentCharge": "12" }))).await;
        assert_eq!(battery, Some((12, false, false)));
        on(&mut desk, &mut battery, &p(BATTERY, json!({ "currentCharge": -1 }))).await;
        assert_eq!(battery, None, "no battery to show");
        assert_eq!(desk.said, ["Pixel: Ping!", "Pixel: hi", "ring for Pixel", "Pixel's battery is low: 14% left"]);
    }

    #[tokio::test]
    async fn the_clipboard_and_shares() {
        let (mut desk, mut clip) = (Notes::default(), Clipboard::default());
        let mut phone = Phone { name: "Pixel".into(), battery: None };
        on_packet(&mut desk, &mut phone, &mut clip, &p(CLIPBOARD, json!({ "content": "one" }))).await;
        on_packet(&mut desk, &mut phone, &mut clip, &p(CLIPBOARD, json!({ "content": "one" }))).await;
        on_packet(&mut desk, &mut phone, &mut clip, &p(SHARE, json!({ "url": "https://example.org/a" }))).await;
        on_packet(&mut desk, &mut phone, &mut clip, &p(SHARE, json!({ "url": "file:///etc/passwd" }))).await;
        on_packet(&mut desk, &mut phone, &mut clip, &p(SHARE, json!({ "text": "two" }))).await;
        assert!(clip.changed("two".into()).is_none(), "shared text copied here isn't sent back as a copy of ours");
        let file = on_packet(&mut desk, &mut phone, &mut clip, &p(SHARE, json!({ "filename": "../a.jpg", "numberOfFiles": 2 }))).await;
        assert_eq!(file.fetch, Some(Incoming { name: "a.jpg".into(), count: 2 }), "a safe name, for the PC side to fetch");
        assert_eq!(desk.said, [
            "copied one",
            "opened https://example.org/a",
            "copied file:///etc/passwd",
            "Text from Pixel: Copied. Paste it anywhere.",
            "copied two",
            "Text from Pixel: Copied. Paste it anywhere.",
        ], "only web links open; anything else is text");
    }

    #[test]
    fn a_batch_is_said_once_it_is_all_in() {
        let arrivals = Arrivals::default();
        let one = |name: &str| Incoming { name: name.into(), count: 3 };
        let a = arrivals.landed("p", "Pixel", one("a"), Ok("/d/a".into()), "this PC");
        assert_eq!((a.done, a.count, a.activity.as_deref()), (1, 3, None));
        let b = arrivals.landed("p", "Pixel", one("b"), Err(anyhow::anyhow!("gone")), "this PC");
        assert_eq!((b.done, b.file_name().as_str()), (2, "b"));
        let c = arrivals.landed("p", "Pixel", one("c"), Ok("/d/c (1)".into()), "this PC");
        assert_eq!(c.activity.as_deref(), Some("Shared 2 files with this PC"), "what landed, once the batch is done");
        assert_eq!((a.summary(), b.summary()), (None, None), "a batch is said once");
        assert_eq!(c.summary().unwrap().1, "2 of 3 files are in Downloads; the rest didn't arrive");
        let single = arrivals.landed("p", "Pixel", Incoming { name: "x".into(), count: 1 }, Ok("/d/x (2).png".into()), "this Mac");
        assert_eq!((single.done, single.activity.as_deref()), (1, Some("Shared \u{201c}x (2).png\u{201d} with this Mac")), "a new batch starts afresh");
        let lost = arrivals.landed("p", "Pixel", Incoming { name: "y".into(), count: 1 }, Err(anyhow::anyhow!("gone")), "this PC");
        assert_eq!(lost.activity, None, "nothing landed, nothing to log");
        assert_eq!(single.summary().unwrap(), ("From Pixel".to_string(), "x (2).png is in Downloads".to_string()));
        assert_eq!(lost.summary().unwrap().0, "y didn't arrive");
    }

    #[tokio::test]
    async fn locks_and_says_why_it_couldnt() {
        let (mut desk, mut battery) = (Notes { unlocks: true, ..Default::default() }, None);
        assert!(on(&mut desk, &mut battery, &p(LOCK_REQUEST, json!({ "setLocked": true }))).await.is_empty());
        let answer = on(&mut desk, &mut battery, &p(LOCK_REQUEST, json!({ "requestLocked": true }))).await;
        // Type and body: the id is the clock's millisecond, which may tick between the two (often on Windows).
        let expected = lock_packet(true);
        assert_eq!(answer.len(), 1);
        assert_eq!((&answer[0]["type"], &answer[0]["body"]), (&expected["type"], &expected["body"]));
        desk.lock_fails = true;
        let refused = on(&mut desk, &mut battery, &p(LOCK_REQUEST, json!({ "setLocked": true }))).await;
        assert_eq!((refused[0]["type"].as_str(), refused[0]["body"]["reason"].as_str()), (Some(DENIED), Some("no screen locker")));
        // A PC that can't unlock says nothing about it, and one that can't tell says no state.
        let mut desk = Notes::default();
        assert!(on(&mut desk, &mut battery, &p(LOCK_REQUEST, json!({ "setLocked": false, "requestLocked": true }))).await.is_empty());
        assert!(handles(LOCK_REQUEST) && handles(SHARE) && !handles("kdeconnect.mpris.request"));
        assert_eq!(battery_packet(Battery { charge: 55, charging: false })["body"], json!({ "currentCharge": 55, "isCharging": false, "thresholdEvent": 0 }));
    }
}
