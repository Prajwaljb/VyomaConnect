//! Talking on the PC: this PC becomes the phone's Bluetooth headset for the call (hands-free, the
//! phone as audio gateway), as a car does. BlueZ connects the profile; PipeWire's own hands-free
//! role (on by default) then plays the phone's side of the call to the default output as a stream
//! and records the default microphone for it, so headphones here work too. At the call's end the
//! profile is let go again if this connected it, so the phone keeps no headset it didn't have.
//! Device names and addresses never go to the log.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;
use tokio::process::Command;
use zbus::{Connection, OwnedObjectPath, OwnedVariant};

/// The phone's side of hands-free: the audio gateway.
const AUDIO_GATEWAY: &str = "0000111f-0000-1000-8000-00805f9b34fb";
/// PipeWire's card profile for a phone as audio gateway.
const PROFILE: &str = "audio-gateway";

/// A phone bonded to this PC that can carry calls.
#[derive(Clone, Debug, PartialEq)]
pub struct Bonded {
    pub path: String,
    pub address: String,
    pub name: String,
    pub alias: String,
    pub connected: bool,
}

/// Which bonded phone is [name] (the phone's own Bluetooth name, as Vyoma tells it): the one by that
/// name, else the only one there is.
pub fn pick<'a>(phones: &'a [Bonded], name: Option<&str>) -> Option<&'a Bonded> {
    let name = name.map(str::trim).filter(|n| !n.is_empty());
    if let Some(n) = name
        && let Some(p) = phones.iter().find(|p| p.alias.trim() == n || p.name.trim() == n)
    {
        return Some(p);
    }
    match phones {
        [only] => Some(only),
        _ => None,
    }
}

/// The call's sound on this PC; give it back with [release].
pub struct Audio {
    path: String,
    address: String,
    /// This connected the profile, so it disconnects it after.
    connected_here: bool,
    /// The card's profile before, to put back when the phone stays connected.
    was: Option<String>,
}

/// Takes the call's sound from the phone called [name] over Bluetooth.
pub async fn take(name: Option<&str>) -> Result<Audio> {
    let system = Connection::system().await.context("no system bus")?;
    let phones = bonded(&system).await?;
    let Some(phone) = pick(&phones, name) else {
        bail!(if phones.is_empty() { "This PC isn't paired with the phone over Bluetooth" } else { "Couldn't tell which Bluetooth device is the phone" });
    };
    let reply = device(&system, &phone.path, "ConnectProfile").await;
    let connected_here = match reply {
        Ok(()) => !phone.connected,
        Err(e) if e.to_string().contains("AlreadyConnected") => false,
        Err(e) => return Err(e).context("the phone didn't take this PC as its headset"),
    };
    tracing::info!("call audio: this PC is the phone's headset{}", if connected_here { "" } else { " (it already was)" });
    let mut audio = Audio { path: phone.path.clone(), address: phone.address.clone(), connected_here, was: None };
    // The card shows up a moment after the profile connects.
    for _ in 0..20 {
        if let Some((card, active, has)) = card(&audio.address).await {
            if active != PROFILE && has {
                pactl(&["set-card-profile", &card, PROFILE]).await?;
                audio.was = Some(active);
            }
            return Ok(audio);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    tracing::warn!("call audio: the phone's sound card didn't show up");
    Ok(audio)
}

/// Gives the call's sound back to the phone.
pub async fn release(audio: Audio) {
    if let Some(was) = &audio.was
        && !audio.connected_here
        && let Some((card, ..)) = card(&audio.address).await
    {
        let _ = pactl(&["set-card-profile", &card, was]).await;
    }
    tracing::info!("call audio: back to the phone");
    if audio.connected_here {
        let Ok(system) = Connection::system().await else { return };
        if let Err(e) = device(&system, &audio.path, "DisconnectProfile").await {
            tracing::warn!("call audio: couldn't let the phone go: {e:#}");
        }
    }
}

async fn device(system: &Connection, path: &str, method: &str) -> Result<()> {
    system.call_method(Some("org.bluez"), path, Some("org.bluez.Device1"), method, &(AUDIO_GATEWAY,)).await.map(drop).map_err(|e| anyhow!("{e}"))
}

type Objects = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedVariant>>>;

/// Bonded devices offering the audio gateway: phones.
async fn bonded(system: &Connection) -> Result<Vec<Bonded>> {
    let reply = system.call_method(Some("org.bluez"), "/", Some("org.freedesktop.DBus.ObjectManager"), "GetManagedObjects", &()).await.context("Bluetooth isn't running")?;
    let objects: Objects = reply.body().deserialize()?;
    let text = |p: &HashMap<String, OwnedVariant>, k: &str| p.get(k).and_then(|v| <&str>::try_from(&**v).ok()).unwrap_or("").to_owned();
    let flag = |p: &HashMap<String, OwnedVariant>, k: &str| p.get(k).and_then(|v| bool::try_from(&**v).ok()).unwrap_or(false);
    let mut out: Vec<Bonded> = objects
        .into_iter()
        .filter_map(|(path, ifaces)| {
            let d = ifaces.get("org.bluez.Device1")?;
            let uuids: Vec<String> = d.get("UUIDs").and_then(|v| v.try_clone().ok()).and_then(|v| Vec::<String>::try_from(v).ok()).unwrap_or_default();
            let bonded = flag(d, "Paired") || flag(d, "Bonded");
            (bonded && uuids.iter().any(|u| u.eq_ignore_ascii_case(AUDIO_GATEWAY))).then(|| Bonded {
                path: path.to_string(),
                address: text(d, "Address"),
                name: text(d, "Name"),
                alias: text(d, "Alias"),
                connected: flag(d, "Connected"),
            })
        })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// The phone's card: its name, active profile, and whether it offers [PROFILE].
async fn card(address: &str) -> Option<(String, String, bool)> {
    let out = Command::new("pactl").args(["-f", "json", "list", "cards"]).output().await.ok()?;
    let cards: Value = serde_json::from_slice(&out.stdout).ok()?;
    find_card(&cards, address)
}

fn find_card(cards: &Value, address: &str) -> Option<(String, String, bool)> {
    let c = cards.as_array()?.iter().find(|c| c["properties"]["api.bluez5.address"].as_str().is_some_and(|a| a.eq_ignore_ascii_case(address)))?;
    let has = c["profiles"].as_object().is_some_and(|p| p.get(PROFILE).is_some_and(|p| p["available"].as_bool().unwrap_or(true)));
    Some((c["name"].as_str()?.to_owned(), c["active_profile"].as_str().unwrap_or("").to_owned(), has))
}

async fn pactl(args: &[&str]) -> Result<()> {
    let out = Command::new("pactl").args(args).output().await.context("pactl isn't installed")?;
    if !out.status.success() {
        bail!("pactl: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn phone(n: &str) -> Bonded {
        Bonded { path: format!("/org/bluez/hci0/dev_{n}"), address: n.into(), name: format!("{n} phone"), alias: n.into(), connected: false }
    }

    #[test]
    fn finds_the_phone_by_its_name_else_the_only_one() {
        let one = [phone("A")];
        assert_eq!(pick(&one, None).map(|p| p.alias.as_str()), Some("A"));
        assert_eq!(pick(&one, Some("Other")).map(|p| p.alias.as_str()), Some("A"));
        let two = [phone("A"), phone("B")];
        assert_eq!(pick(&two, Some(" B ")).map(|p| p.alias.as_str()), Some("B"));
        assert_eq!(pick(&two, Some("A phone")).map(|p| p.alias.as_str()), Some("A"));
        assert_eq!(pick(&two, None), None);
        assert_eq!(pick(&[], Some("A")), None);
    }

    #[test]
    fn reads_the_phones_card() {
        let cards = json!([
            { "name": "alsa_card.x", "properties": {}, "active_profile": "on", "profiles": {} },
            { "name": "bluez_card.AA", "properties": { "api.bluez5.address": "aa:bb" }, "active_profile": "a2dp-source",
              "profiles": { "off": {}, "audio-gateway": { "available": true } } },
        ]);
        assert_eq!(find_card(&cards, "AA:BB"), Some(("bluez_card.AA".into(), "a2dp-source".into(), true)));
        assert_eq!(find_card(&cards, "CC"), None);
    }
}
