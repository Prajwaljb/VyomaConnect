//! The PC's outputs for the phone's volume page, as kdeconnectd's systemvolume plugin: the list
//! (`requestSinks`), then one sink's volume, mute or being the default. Volumes are PulseAudio's
//! (65536 = 100%). Through pactl, which PipeWire's pulse server answers too.

use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use vyoma_core::link::packet;

pub const VOLUME: &str = "kdeconnect.systemvolume";
pub const VOLUME_REQUEST: &str = "kdeconnect.systemvolume.request";

/// 100%: the top of the phone's slider, as kdeconnectd sends it, so its percent is the PC's.
const NORMAL: i64 = 65536;
/// The most a device may set, whatever it sends.
const MAX_VOLUME: i64 = NORMAL * 3 / 2;

#[derive(Clone, Debug, PartialEq)]
pub struct Sink {
    name: String,
    description: String,
    volume: i64,
    muted: bool,
    default: bool,
}

/// `pactl -f json list sinks` with the default's name: the loudest channel is the volume.
fn parse(json: &str, default: &str) -> Result<Vec<Sink>> {
    let all: Vec<Value> = serde_json::from_str(json).context("pactl gave no sink list")?;
    Ok(all
        .iter()
        .filter_map(|s| {
            let name = s["name"].as_str()?.to_owned();
            let volume = s["volume"].as_object().map_or(0, |ch| ch.values().filter_map(|c| c["value"].as_i64()).max().unwrap_or(0));
            Some(Sink {
                description: s["description"].as_str().filter(|d| !d.is_empty()).unwrap_or(&name).to_owned(),
                volume,
                muted: s["mute"].as_bool().unwrap_or(false),
                default: name == default,
                name,
            })
        })
        .collect())
}

async fn pactl(args: &[&str]) -> Result<String> {
    let out = tokio::time::timeout(Duration::from_secs(3), Command::new("pactl").args(args).stderr(Stdio::null()).kill_on_drop(true).output()).await.context("pactl hung")?.context("no pactl")?;
    if !out.status.success() {
        bail!("pactl {} failed", args.first().unwrap_or(&""));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

async fn sinks() -> Result<Vec<Sink>> {
    let default = pactl(&["get-default-sink"]).await.unwrap_or_default();
    parse(&pactl(&["-f", "json", "list", "sinks"]).await?, default.trim())
}

pub fn list_packet(sinks: &[Sink]) -> Value {
    let list: Vec<Value> = sinks.iter().map(|s| json!({ "name": s.name, "description": s.description, "volume": s.volume, "maxVolume": NORMAL, "muted": s.muted, "enabled": s.default })).collect();
    packet(VOLUME, json!({ "sinkList": list }))
}

/// What the devices are told after [old] became [new]: the whole list if sinks came or went,
/// else one packet per sink that changed.
fn changes(old: &[Sink], new: &[Sink]) -> Vec<Value> {
    let names = |s: &[Sink]| s.iter().map(|s| s.name.clone()).collect::<Vec<_>>();
    if names(old) != names(new) {
        return vec![list_packet(new)];
    }
    old.iter().zip(new).filter(|(a, b)| a != b).map(|(_, s)| packet(VOLUME, json!({ "name": s.name, "volume": s.volume, "muted": s.muted, "enabled": s.default }))).collect()
}

/// Follows the outputs and answers [requests] (device id, packet). [out]: (to whom, or every
/// device that takes it; packet).
pub async fn run(mut requests: mpsc::UnboundedReceiver<(String, Value)>, out: mpsc::UnboundedSender<(Option<String>, Value)>) -> Result<()> {
    let mut known = sinks().await?;
    let mut events = Command::new("pactl").arg("subscribe").stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true).spawn().context("no pactl")?;
    let mut lines = BufReader::new(events.stdout.take().unwrap()).lines();
    loop {
        tokio::select! {
            Some((id, p)) = requests.recv() => {
                let body = &p["body"];
                if body["requestSinks"].as_bool() == Some(true) {
                    let _ = out.send((Some(id.clone()), list_packet(&known)));
                }
                if let Some(name) = body["name"].as_str() {
                    apply(&known, &id, name, body).await;
                }
            }
            line = lines.next_line() => {
                let Some(line) = line? else { bail!("pactl subscribe stopped") };
                // "Event 'change' on sink #52", "… on server #…" (the default moved).
                if !(line.contains(" sink #") || line.contains(" server")) {
                    continue;
                }
                // A slider sends a burst: read once it settles.
                tokio::time::sleep(Duration::from_millis(80)).await;
                let Ok(now) = sinks().await else { continue };
                for p in changes(&known, &now) {
                    let _ = out.send((None, p));
                }
                known = now;
            }
        }
    }
}

/// A device's change to sink [name], which must be one we listed.
async fn apply(known: &[Sink], id: &str, name: &str, body: &Value) {
    if !known.iter().any(|s| s.name == name) {
        tracing::info!("{id}: no sink {name}");
        return;
    }
    let mut done = Ok(String::new());
    if let Some(v) = body["volume"].as_i64() {
        done = done.and(pactl(&["set-sink-volume", name, &v.clamp(0, MAX_VOLUME).to_string()]).await);
    }
    if let Some(m) = body["muted"].as_bool() {
        done = done.and(pactl(&["set-sink-mute", name, if m { "1" } else { "0" }]).await);
    }
    if body["enabled"].as_bool() == Some(true) {
        done = done.and(pactl(&["set-default-sink", name]).await);
    }
    if let Err(e) = done {
        tracing::warn!("{id}: {e:#}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = r#"[
        {"name": "hdmi", "description": "HDMI", "mute": false, "volume": {"front-left": {"value": 30000}, "front-right": {"value": 32000}}},
        {"name": "speakers", "description": "", "mute": true, "volume": {"mono": {"value": 65536}}}
    ]"#;

    #[test]
    fn sinks_and_their_changes() {
        let sinks = parse(LIST, "speakers").unwrap();
        assert_eq!(sinks[0], Sink { name: "hdmi".into(), description: "HDMI".into(), volume: 32000, muted: false, default: false });
        assert_eq!((sinks[1].description.as_str(), sinks[1].muted, sinks[1].default), ("speakers", true, true));
        let list = list_packet(&sinks);
        assert_eq!(list["body"]["sinkList"][1]["maxVolume"], NORMAL);

        let mut louder = sinks.clone();
        louder[0].volume = 40000;
        let c = changes(&sinks, &louder);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0]["body"], json!({ "name": "hdmi", "volume": 40000, "muted": false, "enabled": false }));
        assert!(changes(&sinks, &sinks).is_empty());
        assert!(changes(&sinks, &sinks[..1]).first().unwrap()["body"]["sinkList"].is_array(), "a sink went: the whole list");
    }
}

/// Drives a silent test sink through [run]. Run by hand: `cargo test --release -- --ignored test_sink`
/// (a "Vyoma test" output appears briefly; the PC's own outputs are only listed).
#[cfg(test)]
mod live {
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_sink() {
        let module = pactl(&["load-module", "module-null-sink", "sink_name=vyoma_test", "sink_properties=device.description=Vyoma-test"]).await.unwrap();
        let module = module.trim().to_owned();
        let result = tokio::spawn(async {
            let (requests, rx) = mpsc::unbounded_channel();
            let (tx, mut out) = mpsc::unbounded_channel();
            tokio::spawn(run(rx, tx));
            let ask = |body: Value| requests.send(("phone".into(), packet(VOLUME_REQUEST, body))).unwrap();
            let mut next = async move || tokio::time::timeout(Duration::from_secs(5), out.recv()).await.expect("nothing sent").unwrap();

            ask(json!({ "requestSinks": true }));
            let (to, list) = next().await;
            assert_eq!(to.as_deref(), Some("phone"));
            let ours = list["body"]["sinkList"].as_array().unwrap().iter().find(|s| s["name"] == "vyoma_test").cloned().expect("test sink listed");
            assert_eq!(ours["description"], "Vyoma-test");

            ask(json!({ "name": "vyoma_test", "volume": 32768, "muted": true }));
            ask(json!({ "name": "-h", "volume": 1 }));
            loop {
                let (to, p) = next().await;
                if to.is_none() && p["body"]["name"] == "vyoma_test" && p["body"]["volume"] == 32768 && p["body"]["muted"] == true {
                    break;
                }
            }
        })
        .await;
        pactl(&["unload-module", &module]).await.unwrap();
        result.unwrap();
        println!("ok: list, volume and mute, change sent back");
    }
}
