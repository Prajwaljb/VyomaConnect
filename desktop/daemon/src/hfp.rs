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
    /// The default microphone before, when it was a Bluetooth headset's: put back after.
    mic_was: Option<String>,
    /// The microphone this chose instead.
    mic: Option<String>,
}

/// Takes the call's sound from the phone called [name] over Bluetooth.
pub async fn take(name: Option<&str>) -> Result<Audio> {
    let system = Connection::system().await.context("no system bus")?;
    let phones = bonded(&system).await?;
    let Some(phone) = pick(&phones, name) else {
        bail!(if phones.is_empty() { "This PC isn't paired with the phone over Bluetooth" } else { "Couldn't tell which Bluetooth device is the phone" });
    };
    // A Bluetooth headset as the microphone would go into its own call mode for it: a second call
    // link on the same Bluetooth chip, which many can't carry (silence both ways), and its music
    // mode gone meanwhile. The PC's own microphone instead, for the call, keeps the headphones as they are.
    let (mic_was, mic) = match away_from_headset().await {
        Some((was, mic)) if pactl(&["set-default-source", &mic]).await.is_ok() => {
            tracing::info!("call audio: this PC's microphone for the call, not the headset's");
            (Some(was), Some(mic))
        }
        _ => (None, None),
    };
    let audio = Audio { path: phone.path.clone(), mic_was, mic };
    match device(&system, &phone.path, "ConnectProfile").await {
        Ok(()) => {}
        Err(e) if e.to_string().contains("AlreadyConnected") => {}
        Err(e) => {
            release(audio).await;
            return Err(e).context("the phone didn't take this PC as its headset");
        }
    }
    tracing::info!("call audio: this PC is the phone's headset");
    // The card shows up a moment after the profile connects.
    for _ in 0..20 {
        if let Some((card, active, has)) = card(&phone.address).await {
            if active != PROFILE && has {
                pactl(&["set-card-profile", &card, PROFILE]).await?;
            }
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    // Its microphone stream, if it linked before the default changed.
    if let Some(mic) = &audio.mic {
        for _ in 0..20 {
            if move_phone_capture(&phone.address, mic).await {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
    Ok(audio)
}

/// Gives the call's sound back to the phone. Always lets the phone go: a PC still connected as its
/// headset would take every later call too, answered there or not.
pub async fn release(audio: Audio) {
    tracing::info!("call audio: back to the phone");
    if let Ok(system) = Connection::system().await
        && let Err(e) = device(&system, &audio.path, "DisconnectProfile").await
    {
        tracing::warn!("call audio: couldn't let the phone go: {e:#}");
    }
    // The microphone back, unless someone chose another meanwhile.
    if let (Some(was), Some(mic)) = (&audio.mic_was, &audio.mic)
        && default_source().await.as_deref() == Some(mic.as_str())
    {
        let _ = pactl(&["set-default-source", was]).await;
    }
}

async fn default_source() -> Option<String> {
    let out = Command::new("pactl").arg("get-default-source").output().await.ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_owned()).filter(|s| !s.is_empty())
}

/// When the default microphone is a Bluetooth device's: it, and the PC's own one to use instead.
async fn away_from_headset() -> Option<(String, String)> {
    let default = default_source().await?;
    let out = Command::new("pactl").args(["-f", "json", "list", "sources"]).output().await.ok()?;
    let sources: Value = serde_json::from_slice(&out.stdout).ok()?;
    pick_mic(&sources, &default).map(|mic| (default, mic))
}

fn pick_mic(sources: &Value, default: &str) -> Option<String> {
    let list = sources.as_array()?;
    let api = |s: &Value| s["properties"]["device.api"].as_str().unwrap_or("").to_owned();
    let current = list.iter().find(|s| s["name"] == default)?;
    if api(current) != "bluez5" {
        return None;
    }
    let mics: Vec<&Value> = list
        .iter()
        .filter(|s| s["properties"]["media.class"] == "Audio/Source" && api(s) != "bluez5" && !s["name"].as_str().unwrap_or("").ends_with(".monitor"))
        .collect();
    // One that's there (an empty jack's port says "not available"), a microphone over a line input or HDMI.
    let score = |s: &Value| {
        let text = format!("{} {}", s["name"].as_str().unwrap_or(""), s["description"].as_str().unwrap_or("")).to_lowercase();
        let ports = s["ports"].as_array().map(Vec::as_slice).unwrap_or_default();
        let there = ports.is_empty() || ports.iter().any(|p| p["availability"] != "not available");
        (there, text.contains("mic"), !text.contains("hdmi") && !text.contains("line"))
    };
    mics.into_iter().max_by_key(|s| score(s)).and_then(|s| s["name"].as_str()).map(str::to_owned)
}

/// Moves the phone's microphone stream (what this PC sends into the call) to [mic]; whether it was there.
async fn move_phone_capture(address: &str, mic: &str) -> bool {
    let Ok(out) = Command::new("pactl").args(["-f", "json", "list", "source-outputs"]).output().await else { return false };
    let Ok(list) = serde_json::from_slice::<Value>(&out.stdout) else { return false };
    let ours = list.as_array().into_iter().flatten().find(|o| o["properties"]["api.bluez5.address"].as_str().is_some_and(|a| a.eq_ignore_ascii_case(address)));
    let Some(index) = ours.and_then(|o| o["index"].as_u64()) else { return false };
    pactl(&["move-source-output", &index.to_string(), mic]).await.is_ok()
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
    fn leaves_a_headsets_microphone_for_the_pcs_own() {
        let src = |name: &str, api: &str, desc: &str| json!({ "name": name, "description": desc, "properties": { "device.api": api, "media.class": "Audio/Source" } });
        let all = json!([src("bluez_input.x", "bluez5", "Headphones"), src("alsa_input.hdmi", "alsa", "HDMI"), src("alsa_input.analog", "alsa", "Digital Microphone"), src("alsa_output.a.monitor", "alsa", "Monitor")]);
        assert_eq!(pick_mic(&all, "bluez_input.x").as_deref(), Some("alsa_input.analog"));
        assert_eq!(pick_mic(&all, "alsa_input.hdmi"), None);
        assert_eq!(pick_mic(&json!([src("bluez_input.x", "bluez5", "H")]), "bluez_input.x"), None);
        let mut jack = src("alsa_input.jack", "alsa", "Stereo Microphone");
        jack["ports"] = json!([{ "availability": "not available" }]);
        let all = json!([src("bluez_input.x", "bluez5", "H"), jack, src("alsa_input.dmic", "alsa", "Digital Microphone")]);
        assert_eq!(pick_mic(&all, "bluez_input.x").as_deref(), Some("alsa_input.dmic"));
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
