//! The stock KDE Connect plugins on Linux: what the engine's [vyoma_engine::stock] does through
//! this PC's notifications, ringer and logind session, plus the lists of what we take and send.
//! The clipboard and sharing are in their own modules.
//!
//! A packet only goes to a paired device whose identity says it takes that type.

use std::collections::HashMap;

use serde_json::Value;
use tokio::sync::mpsc;

use crate::DeviceView;
use crate::clipboard::{CLIPBOARD, CLIPBOARD_CONNECT};
use crate::media::{MPRIS, MPRIS_REQUEST};
use crate::volume::{VOLUME, VOLUME_REQUEST};
use crate::commands::{RUNCOMMAND, RUNCOMMAND_REQUEST};
use crate::input::{MOUSEPAD_ECHO, MOUSEPAD_KEYBOARDSTATE, MOUSEPAD_REQUEST};
use crate::notifications::{NOTIFICATION, NOTIFICATION_ACTION, NOTIFICATION_REPLY, NOTIFICATION_REQUEST};
use crate::share::{SHARE, SHARE_UPDATE};
use vyoma_core::messages;
use crate::telephony::{self, MUTE, TELEPHONY};
use crate::presenter::PRESENTER;
use crate::phonefiles::{SFTP, SFTP_REQUEST};
use crate::notify::{Notice, Notifier};
use crate::system::{Battery, Ringer, Session};
use vyoma_engine::stock::{self, Clipboard, Desk, Outcome};
pub use vyoma_engine::stock::{BATTERY, FIND, LOCK, LOCK_REQUEST, PING, battery_packet, lock_packet};

/// What we take. Not lock requests outward: phones can't be locked that way, and a KDE PC would lock.
pub const INCOMING: &[&str] = &[PING, BATTERY, FIND, LOCK_REQUEST, CLIPBOARD, CLIPBOARD_CONNECT, SHARE, SHARE_UPDATE, MPRIS_REQUEST, VOLUME_REQUEST, RUNCOMMAND_REQUEST, NOTIFICATION, MOUSEPAD_REQUEST, TELEPHONY, telephony::DONE, PRESENTER, SFTP, messages::MESSAGES, messages::CONTACTS_UIDS_REPLY, messages::CONTACTS_VCARDS_REPLY, messages::UNLOCKED, messages::LOCKED];
pub const OUTGOING: &[&str] = &[
    PING, BATTERY, FIND, LOCK, CLIPBOARD, CLIPBOARD_CONNECT, SHARE, SHARE_UPDATE, MPRIS, VOLUME, RUNCOMMAND,
    NOTIFICATION_REQUEST, NOTIFICATION_REPLY, NOTIFICATION_ACTION, MOUSEPAD_ECHO, MOUSEPAD_KEYBOARDSTATE, MUTE, telephony::REQUEST, SFTP_REQUEST,
    messages::REQUEST, messages::REQUEST_CONVERSATIONS, messages::REQUEST_CONVERSATION, messages::CONTACTS_UIDS, messages::CONTACTS_VCARDS, messages::UNLOCK, messages::LOCK,
    crate::power::LEAVING, telephony::DIAL,
];

/// Whether [device] is paired and its identity says it takes [kind].
pub fn takes(device: &DeviceView, kind: &str) -> bool {
    device.paired && device.incoming.iter().any(|t| t == kind)
}

/// What a device that just became reachable and paired is sent: the PC's battery and lock.
pub fn started(device: &DeviceView, battery: Option<Battery>, locked: Option<bool>) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(b) = battery.filter(|_| takes(device, BATTERY)) {
        out.push(battery_packet(b));
    }
    if let Some(l) = locked.filter(|_| takes(device, LOCK)) {
        out.push(lock_packet(l));
    }
    out
}

/// Shared by the handlers: how to reply and what to touch on this PC.
pub struct Context<'a> {
    pub notifier: &'a Notifier,
    pub session: Option<&'a Session>,
    pub ringer: &'a mut Ringer,
    /// The notification for the ring on screen, to take down with it.
    pub ring_notice: &'a mut Option<u32>,
    /// What goes onto this PC's clipboard, in order.
    pub paste: &'a mpsc::UnboundedSender<String>,
    /// What notifications' buttons open.
    pub opens: &'a mut HashMap<u32, crate::Open>,
}

impl Desk for Context<'_> {
    async fn tell(&mut self, title: &str, body: &str) {
        tell(self.notifier, title, body).await;
    }

    async fn ring(&mut self, phone: &str) {
        self.ringer.start();
        let title = format!("{phone} is looking for this PC");
        let notice = Notice { icon: "smartphone", title: &title, body: "", actions: &[("found", "Found it")], sticky: false, replaces: *self.ring_notice };
        *self.ring_notice = self.notifier.show(notice).await.ok();
    }

    async fn set_locked(&mut self, lock: bool) -> Option<anyhow::Result<()>> {
        // The new state goes out when logind reports it, to every device that follows it.
        Some(self.session?.set_locked(lock).await)
    }

    async fn locked(&mut self) -> Option<bool> {
        self.session?.locked().await.ok()
    }

    async fn copy(&mut self, text: &str) -> anyhow::Result<()> {
        self.paste.send(text.to_owned()).map_err(|_| anyhow::anyhow!("the clipboard's writer stopped"))
    }

    async fn open_link(&mut self, url: &str) {
        crate::share::open(url);
    }

    /// Offered with a Copy button rather than copied: what's on the clipboard stays until asked.
    async fn shared_text(&mut self, phone: &str, text: &str) -> bool {
        let title = format!("Text from {phone}");
        let shown: String = text.chars().take(400).collect();
        let notice = Notice { icon: "edit-paste", title: &title, body: &shown, actions: &[("copy", "Copy")], sticky: false, replaces: None };
        crate::show_open(self.notifier, self.opens, notice, crate::Open::Copy(text.to_owned())).await;
        false
    }
}

/// Handles one packet from a paired device; what to send back, a file to fetch.
pub async fn on_packet(mut cx: Context<'_>, device: &mut DeviceView, clip: &mut Clipboard, p: &Value) -> Outcome {
    let kind = p["type"].as_str().unwrap_or("");
    if !stock::handles(kind) {
        tracing::debug!("{}: no plugin for {kind}", device.name);
        return Outcome::default();
    }
    let mut phone = stock::Phone { name: device.name.clone(), battery: device.battery };
    let out = stock::on_packet(&mut cx, &mut phone, clip, p).await;
    device.battery = phone.battery;
    out
}

/// Stops a ring started by a device and takes its notification down.
pub async fn stop_ringing(notifier: &Notifier, ringer: &mut Ringer, ring_notice: &mut Option<u32>) {
    ringer.stop();
    if let Some(n) = ring_notice.take() {
        notifier.close(n).await;
    }
}

pub async fn tell(notifier: &Notifier, title: &str, body: &str) {
    if let Err(e) = notifier.show(Notice { icon: "smartphone", title, body, actions: &[], sticky: false, replaces: None }).await {
        tracing::warn!("couldn't show \"{title}\": {e:#}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn paired(incoming: &[&str]) -> DeviceView {
        DeviceView { paired: true, linked: true, incoming: incoming.iter().map(|s| s.to_string()).collect(), ..Default::default() }
    }

    #[test]
    fn only_what_a_device_takes_is_sent() {
        let battery = Battery { charge: 55, charging: false };
        let b = Some(battery);
        let phone = paired(&[BATTERY, LOCK]);
        let kinds: Vec<_> = started(&phone, b, Some(true)).iter().map(|p| p["type"].as_str().unwrap().to_owned()).collect();
        assert_eq!(kinds, [BATTERY, LOCK]);
        assert!(started(&paired(&[PING]), b, Some(true)).is_empty());
        assert!(started(&DeviceView { paired: false, ..phone }, b, Some(true)).is_empty(), "nothing before pairing");
        assert!(started(&paired(&[BATTERY]), None, None).is_empty(), "a desktop has no battery to send");
        assert_eq!(battery_packet(battery)["body"], json!({ "currentCharge": 55, "isCharging": false, "thresholdEvent": 0 }));
    }
}
