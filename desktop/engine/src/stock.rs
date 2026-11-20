//! The stock KDE Connect plugins, the PC's part of each, once for every PC side (as the phone's
//! plugins expect them: core/protocol's SimplePlugins.kt and PhonePlugins.kt). What a packet means
//! is here; how this computer shows, rings and locks is a [Desk]'s.
//!
//! So far: ping, battery both ways, ringing this PC, its screen lock.

use std::future::Future;

use anyhow::Result;
use serde_json::{Value, json};
use vyoma_core::link::packet;
use vyoma_core::permits::DENIED;

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
}

/// Whether [kind] is one of the plugins here.
pub fn handles(kind: &str) -> bool {
    matches!(kind, PING | BATTERY | FIND | LOCK_REQUEST)
}

/// Does what packet [p] from phone [name] asks, keeping its [battery]; what to send back.
pub async fn on_packet(desk: &mut impl Desk, name: &str, battery: &mut Option<PhoneBattery>, p: &Value) -> Vec<Value> {
    let body = &p["body"];
    match p["type"].as_str().unwrap_or("") {
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
                        return vec![packet(DENIED, json!({ "request": LOCK_REQUEST, "name": "Lock", "reason": format!("{e:#}") }))];
                    }
                    None => tracing::debug!("{name}: this PC can't be {} from the phone", if lock { "locked" } else { "unlocked" }),
                }
            }
            if body["requestLocked"].as_bool() == Some(true)
                && let Some(locked) = desk.locked().await
            {
                return vec![lock_packet(locked)];
            }
        }
        _ => {}
    }
    Vec::new()
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
    }

    fn p(kind: &str, body: Value) -> Value {
        packet(kind, body)
    }

    #[tokio::test]
    async fn pings_ring_and_say_a_low_battery_once() {
        let (mut desk, mut battery) = (Notes::default(), None);
        on_packet(&mut desk, "Pixel", &mut battery, &p(PING, json!({}))).await;
        on_packet(&mut desk, "Pixel", &mut battery, &p(PING, json!({ "message": "hi" }))).await;
        on_packet(&mut desk, "Pixel", &mut battery, &p(FIND, json!({}))).await;
        on_packet(&mut desk, "Pixel", &mut battery, &p(BATTERY, json!({ "currentCharge": 50, "isCharging": true }))).await;
        assert_eq!(battery, Some((50, true, false)));
        for charge in [14, 13] {
            on_packet(&mut desk, "Pixel", &mut battery, &p(BATTERY, json!({ "currentCharge": charge, "thresholdEvent": 1 }))).await;
        }
        // Old phones send the charge as a string.
        on_packet(&mut desk, "Pixel", &mut battery, &p(BATTERY, json!({ "currentCharge": "12" }))).await;
        assert_eq!(battery, Some((12, false, false)));
        on_packet(&mut desk, "Pixel", &mut battery, &p(BATTERY, json!({ "currentCharge": -1 }))).await;
        assert_eq!(battery, None, "no battery to show");
        assert_eq!(desk.said, ["Pixel: Ping!", "Pixel: hi", "ring for Pixel", "Pixel's battery is low: 14% left"]);
    }

    #[tokio::test]
    async fn locks_and_says_why_it_couldnt() {
        let (mut desk, mut battery) = (Notes { unlocks: true, ..Default::default() }, None);
        assert!(on_packet(&mut desk, "Pixel", &mut battery, &p(LOCK_REQUEST, json!({ "setLocked": true }))).await.is_empty());
        let answer = on_packet(&mut desk, "Pixel", &mut battery, &p(LOCK_REQUEST, json!({ "requestLocked": true }))).await;
        assert_eq!(answer, [lock_packet(true)]);
        desk.lock_fails = true;
        let refused = on_packet(&mut desk, "Pixel", &mut battery, &p(LOCK_REQUEST, json!({ "setLocked": true }))).await;
        assert_eq!((refused[0]["type"].as_str(), refused[0]["body"]["reason"].as_str()), (Some(DENIED), Some("no screen locker")));
        // A PC that can't unlock says nothing about it, and one that can't tell says no state.
        let mut desk = Notes::default();
        assert!(on_packet(&mut desk, "Pixel", &mut battery, &p(LOCK_REQUEST, json!({ "setLocked": false, "requestLocked": true }))).await.is_empty());
        assert!(handles(LOCK_REQUEST) && handles(PING) && !handles("kdeconnect.share.request"));
        assert_eq!(battery_packet(Battery { charge: 55, charging: false })["body"], json!({ "currentCharge": 55, "isCharging": false, "thresholdEvent": 0 }));
    }
}
