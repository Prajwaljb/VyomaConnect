//! The PC's outputs for the phone's volume page, as kdeconnectd's systemvolume plugin: the list
//! (`requestSinks`), then one sink's volume, mute or being the default. Volumes are PulseAudio's
//! (65536 = 100%). Through pactl, which PipeWire's pulse server answers too.

use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

pub use vyoma_engine::media::{Sink, VOLUME, VOLUME_REQUEST};
use vyoma_engine::media::{self, NORMAL, sink_changes, sinks_packet};

/// The most a device may set, whatever it sends.
const MAX_VOLUME: i64 = NORMAL * 3 / 2;

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
                enabled: name == default,
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

/// Follows the outputs and answers [requests] (device id, packet). [out]: (to whom, or every
/// device that takes it; packet).
pub async fn run(mut requests: mpsc::UnboundedReceiver<(String, Value)>, out: mpsc::UnboundedSender<(Option<String>, Value)>) -> Result<()> {
    let mut known = sinks().await?;
    let mut events = Command::new("pactl").arg("subscribe").stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true).spawn().context("no pactl")?;
    let mut lines = BufReader::new(events.stdout.take().unwrap()).lines();
    loop {
        tokio::select! {
            Some((id, p)) = requests.recv() => {
                let r = media::sink_request(&p["body"]);
                if r.list {
                    let _ = out.send((Some(id.clone()), sinks_packet(&known)));
                }
                if r.name.is_some() {
                    apply(&known, &id, &r).await;
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
                for p in sink_changes(&known, &now) {
                    let _ = out.send((None, p));
                }
                known = now;
            }
        }
    }
}

/// A device's change to sink [name], which must be one we listed.
async fn apply(known: &[Sink], id: &str, r: &media::SinkRequest) {
    let Some(name) = r.name.as_deref() else { return };
    let Some(sink) = known.iter().find(|s| s.name == name) else {
        tracing::info!("{id}: no sink {name}");
        return;
    };
    let volume = r.volume.map(|v| v.clamp(0, MAX_VOLUME));
    let mut done = Ok(String::new());
    if let Some(v) = volume {
        done = done.and(pactl(&["set-sink-volume", name, &v.to_string()]).await);
    }
    if let Some(m) = r.muted {
        done = done.and(pactl(&["set-sink-mute", name, if m { "1" } else { "0" }]).await);
    }
    if r.enable {
        done = done.and(pactl(&["set-default-sink", name]).await);
    }
    if let Err(e) = done {
        tracing::warn!("{id}: {e:#}");
        return;
    }
    // GNOME shows no pop-up for a change it didn't make (Plasma does): ask its shell for one, for
    // the output that's playing, as its volume keys show it.
    if vyoma_companion::desktop::is_gnome() && (sink.enabled || r.enable) && (volume.is_some() || r.muted.is_some()) {
        let (icon, level, max) = osd(volume.unwrap_or(sink.volume), r.muted.unwrap_or(sink.muted));
        let _ = vyoma_companion::shell::show_osd(icon, level, max, None).await;
    }
}

/// GNOME's volume pop-up for [volume]: its icon, level and top, as GNOME's own picks them.
fn osd(volume: i64, muted: bool) -> (&'static str, f64, f64) {
    let level = volume as f64 / NORMAL as f64;
    let icon = match level {
        _ if muted || volume == 0 => "audio-volume-muted-symbolic",
        l if l <= 1.0 / 3.0 => "audio-volume-low-symbolic",
        l if l <= 2.0 / 3.0 => "audio-volume-medium-symbolic",
        l if l <= 1.0 => "audio-volume-high-symbolic",
        _ => "audio-volume-overamplified-symbolic",
    };
    // Above 100% the bar grows a boosted part, as with GNOME's own "over-amplification".
    let max = if level > 1.0 { MAX_VOLUME as f64 / NORMAL as f64 } else { 1.0 };
    (icon, if muted { 0.0 } else { level }, max)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use super::*;

    const LIST: &str = r#"[
        {"name": "hdmi", "description": "HDMI", "mute": false, "volume": {"front-left": {"value": 30000}, "front-right": {"value": 32000}}},
        {"name": "speakers", "description": "", "mute": true, "volume": {"mono": {"value": 65536}}}
    ]"#;

    #[test]
    fn picks_gnomes_volume_popup() {
        assert_eq!(osd(NORMAL / 2, false), ("audio-volume-medium-symbolic", 0.5, 1.0));
        assert_eq!(osd(NORMAL, true), ("audio-volume-muted-symbolic", 0.0, 1.0));
        assert_eq!(osd(NORMAL / 4, false).0, "audio-volume-low-symbolic");
        assert_eq!(osd(NORMAL * 5 / 4, false), ("audio-volume-overamplified-symbolic", 1.25, 1.5));
    }

    #[test]
    fn sinks_and_their_changes() {
        let sinks = parse(LIST, "speakers").unwrap();
        assert_eq!(sinks[0], Sink { name: "hdmi".into(), description: "HDMI".into(), volume: 32000, muted: false, enabled: false });
        assert_eq!((sinks[1].description.as_str(), sinks[1].muted, sinks[1].enabled), ("speakers", true, true));
        let list = sinks_packet(&sinks);
        assert_eq!(list["body"]["sinkList"][1]["maxVolume"], NORMAL);

        let mut louder = sinks.clone();
        louder[0].volume = 40000;
        let c = sink_changes(&sinks, &louder);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0]["body"], json!({ "name": "hdmi", "volume": 40000, "muted": false, "enabled": false }));
        assert!(sink_changes(&sinks, &sinks).is_empty());
        assert!(sink_changes(&sinks, &sinks[..1]).first().unwrap()["body"]["sinkList"].is_array(), "a sink went: the whole list");
    }
}

/// Drives a silent test sink through [run]. Run by hand: `cargo test --release -- --ignored test_sink`
/// (a "Vyoma test" output appears briefly; the PC's own outputs are only listed).
#[cfg(test)]
mod live {
    use super::*;
    use serde_json::json;
    use vyoma_core::link::packet;

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
