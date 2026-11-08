//! The stock KDE Connect plugins, the PC's part of each, as the phone's plugins expect it
//! (core/protocol's SimplePlugins.kt and PhonePlugins.kt): ping, battery both ways, the PC's screen
//! lock, ringing either side, and the phone's mobile signal. The clipboard and sharing are in their
//! own modules.
//!
//! A packet only goes to a paired device whose identity says it takes that type.

use serde_json::{Value, json};

use crate::DeviceView;
use crate::clipboard::{CLIPBOARD, CLIPBOARD_CONNECT};
use crate::media::{MPRIS, MPRIS_REQUEST};
use crate::volume::{VOLUME, VOLUME_REQUEST};
use crate::commands::{RUNCOMMAND, RUNCOMMAND_REQUEST};
use crate::input::{MOUSEPAD_ECHO, MOUSEPAD_KEYBOARDSTATE, MOUSEPAD_REQUEST};
use crate::notifications::{NOTIFICATION, NOTIFICATION_ACTION, NOTIFICATION_REPLY, NOTIFICATION_REQUEST};
use crate::share::{SHARE, SHARE_UPDATE};
use crate::telephony::{MUTE, TELEPHONY};
use crate::notify::{Notice, Notifier};
use crate::system::{Battery, Ringer, Session};
use vyoma_core::link::packet;

pub const PING: &str = "kdeconnect.ping";
pub const BATTERY: &str = "kdeconnect.battery";
pub const FIND: &str = "kdeconnect.findmyphone.request";
pub const LOCK: &str = "kdeconnect.lock";
pub const LOCK_REQUEST: &str = "kdeconnect.lock.request";
pub const SIGNAL: &str = "kdeconnect.connectivity_report";
pub const SIGNAL_REQUEST: &str = "kdeconnect.connectivity_report.request";

/// What we take. Not lock requests outward: phones can't be locked that way, and a KDE PC would lock.
pub const INCOMING: &[&str] = &[PING, BATTERY, FIND, LOCK_REQUEST, SIGNAL, CLIPBOARD, CLIPBOARD_CONNECT, SHARE, SHARE_UPDATE, MPRIS_REQUEST, VOLUME_REQUEST, RUNCOMMAND_REQUEST, NOTIFICATION, MOUSEPAD_REQUEST, TELEPHONY];
pub const OUTGOING: &[&str] = &[
    PING, BATTERY, FIND, LOCK, SIGNAL_REQUEST, CLIPBOARD, CLIPBOARD_CONNECT, SHARE, SHARE_UPDATE, MPRIS, VOLUME, RUNCOMMAND,
    NOTIFICATION_REQUEST, NOTIFICATION_REPLY, NOTIFICATION_ACTION, MOUSEPAD_ECHO, MOUSEPAD_KEYBOARDSTATE, MUTE,
];

/// Whether [device] is paired and its identity says it takes [kind].
pub fn takes(device: &DeviceView, kind: &str) -> bool {
    device.paired && device.incoming.iter().any(|t| t == kind)
}

/// What a device that just became reachable and paired is sent: the PC's battery and lock, and a
/// request for its signal.
pub fn started(device: &DeviceView, battery: Option<Battery>, locked: Option<bool>) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(b) = battery.filter(|_| takes(device, BATTERY)) {
        out.push(battery_packet(b));
    }
    if let Some(l) = locked.filter(|_| takes(device, LOCK)) {
        out.push(lock_packet(l));
    }
    if takes(device, SIGNAL_REQUEST) {
        out.push(packet(SIGNAL_REQUEST, json!({})));
    }
    out
}

pub fn battery_packet(b: Battery) -> Value {
    // Low is the phone's to judge for itself; a PC sends no threshold event, as kdeconnectd.
    packet(BATTERY, json!({ "currentCharge": b.charge, "isCharging": b.charging, "thresholdEvent": 0 }))
}

pub fn lock_packet(locked: bool) -> Value {
    packet(LOCK, json!({ "isLocked": locked }))
}

/// Shared by the handlers: how to reply and what to touch on this PC.
pub struct Context<'a> {
    pub notifier: &'a Notifier,
    pub session: Option<&'a Session>,
    pub ringer: &'a mut Ringer,
    /// The notification for the ring on screen, to take down with it.
    pub ring_notice: &'a mut Option<u32>,
}

/// Handles one packet from a paired device; what to send back.
pub async fn on_packet(cx: Context<'_>, device: &mut DeviceView, p: &Value) -> Vec<Value> {
    let body = &p["body"];
    match p["type"].as_str().unwrap_or("") {
        PING => {
            let message = body["message"].as_str().unwrap_or("Ping!");
            tell(cx.notifier, &device.name, message).await;
        }
        BATTERY => {
            let charge = body["currentCharge"].as_i64().or_else(|| body["currentCharge"].as_str()?.parse().ok()).unwrap_or(-1);
            let was_low = device.battery.is_some_and(|b| b.2);
            device.battery = (0..=100).contains(&charge).then(|| (charge, body["isCharging"].as_bool().unwrap_or(false), body["thresholdEvent"].as_i64() == Some(1)));
            if let Some((charge, _, true)) = device.battery
                && !was_low
            {
                tell(cx.notifier, &format!("{}'s battery is low", device.name), &format!("{charge}% left")).await;
            }
        }
        FIND => {
            cx.ringer.start();
            let title = format!("{} is looking for this PC", device.name);
            let notice = Notice { icon: "smartphone", title: &title, body: "", actions: &[("found", "Found it")], sticky: false, replaces: *cx.ring_notice };
            *cx.ring_notice = cx.notifier.show(notice).await.ok();
        }
        LOCK_REQUEST => {
            let Some(session) = cx.session else { return Vec::new() };
            if let Some(lock) = body["setLocked"].as_bool() {
                // The new state goes out when logind reports it, to every device that follows it.
                if let Err(e) = session.set_locked(lock).await {
                    tracing::warn!("{}: {e:#}", device.name);
                }
            }
            if body["requestLocked"].as_bool() == Some(true)
                && let Ok(locked) = session.locked().await
            {
                return vec![lock_packet(locked)];
            }
        }
        SIGNAL => {
            device.signal = body["signalStrengths"].as_object().map(|subs| {
                subs.iter()
                    .map(|(sub, s)| json!({ "sim": sub, "network": s["networkType"].as_str().unwrap_or(""), "level": s["signalStrength"].as_i64().unwrap_or(0).clamp(0, 4) }))
                    .collect()
            });
        }
        other => tracing::debug!("{}: no plugin for {other}", device.name),
    }
    Vec::new()
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

    fn paired(incoming: &[&str]) -> DeviceView {
        DeviceView { paired: true, linked: true, incoming: incoming.iter().map(|s| s.to_string()).collect(), ..Default::default() }
    }

    #[test]
    fn only_what_a_device_takes_is_sent() {
        let battery = Battery { charge: 55, charging: false };
        let b = Some(battery);
        let phone = paired(&[BATTERY, LOCK, SIGNAL_REQUEST]);
        let kinds: Vec<_> = started(&phone, b, Some(true)).iter().map(|p| p["type"].as_str().unwrap().to_owned()).collect();
        assert_eq!(kinds, [BATTERY, LOCK, SIGNAL_REQUEST]);
        assert!(started(&paired(&[PING]), b, Some(true)).is_empty());
        assert!(started(&DeviceView { paired: false, ..phone }, b, Some(true)).is_empty(), "nothing before pairing");
        assert!(started(&paired(&[BATTERY]), None, None).is_empty(), "a desktop has no battery to send");
        assert_eq!(battery_packet(battery)["body"], json!({ "currentCharge": 55, "isCharging": false, "thresholdEvent": 0 }));
    }
}
