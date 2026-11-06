//! PC controls: screen brightness (Plasma's ScreenBrightness service, internal panels and DDC
//! monitors alike), the volume of each app playing sound (PipeWire through pactl), and sleep,
//! restart, shut down and log out (Plasma's own, so the session is saved and apps are asked to
//! close, exactly as from the desktop's menu). Everything runs as the logged-in user; no root.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tracing::{info, warn};
use zbus::Connection;
use zbus::OwnedVariant;

use crate::link::{Sender, packet};

const BRIGHTNESS: &str = "org.kde.ScreenBrightness";
const BRIGHTNESS_PATH: &str = "/org/kde/ScreenBrightness";

/// One per link: handles `vyoma.pc.request`/`vyoma.pc.set`/`vyoma.pc.power` packets in the order they
/// came, so a slider's last value always lands last. A burst of sets for the same slider (the phone
/// sends while dragging) collapses to its newest, and all of a burst gets one `vyoma.pc` reply.
pub fn worker(sender: Sender, name: String) -> mpsc::UnboundedSender<Value> {
    let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
    tokio::spawn(async move {
        while let Some(first) = rx.recv().await {
            let mut batch = vec![first];
            while let Ok(p) = rx.try_recv() {
                batch.push(p);
            }
            let mut reply = false;
            for (i, p) in batch.iter().enumerate() {
                let body = &p["body"];
                match p["type"].as_str().unwrap_or_default() {
                    "vyoma.pc.request" => reply = true,
                    "vyoma.pc.set" => {
                        reply = true;
                        let target = |b: &Value| (b["display"].clone(), b["app"].clone(), b["volume"].is_null(), b["muted"].is_null());
                        if batch[i + 1..].iter().any(|q| q["type"] == "vyoma.pc.set" && target(&q["body"]) == target(body)) {
                            continue;
                        }
                        if let Err(e) = set(body).await {
                            warn!("{name}: PC control failed: {e:#}");
                        }
                    }
                    "vyoma.pc.wake" => match wake().await {
                        Ok(()) => info!("{name}: woke and unlocked the PC"),
                        Err(e) => warn!("{name}: {e:#}"),
                    },
                    "vyoma.pc.power" => {
                        let action = body["action"].as_str().unwrap_or_default();
                        info!("{name}: asked the PC to {action}");
                        let result = power(action).await;
                        let mut out = json!({ "action": action });
                        if let Err(e) = &result {
                            warn!("{name}: {e:#}");
                            out["error"] = json!(format!("{e:#}"));
                        }
                        if sender.send(&packet("vyoma.pc.powered", out)).await.is_err() {
                            return;
                        }
                    }
                    _ => {}
                }
            }
            if reply && sender.send(&packet("vyoma.pc", state().await)).await.is_err() {
                return;
            }
        }
    });
    tx
}

/// Everything the phone's PC controls show: `{displays:[{id,label,internal,value}], apps:[{id,name,binary,title,volume,muted}]}`.
/// Values are fractions (volume may pass 1 when the app was boosted on the PC). A part the PC can't
/// report (no brightness service, no PipeWire) is just empty.
pub async fn state() -> Value {
    let (displays, apps) = tokio::join!(displays(), apps());
    json!({ "displays": displays.unwrap_or_default(), "apps": apps.unwrap_or_default() })
}

/// Applies one change from a `vyoma.pc.set` body: `{display, brightness}` or `{app, volume?, muted?}`.
pub async fn set(body: &Value) -> Result<()> {
    if let (Some(display), Some(level)) = (body["display"].as_str(), body["brightness"].as_f64()) {
        return set_brightness(display, level).await;
    }
    if let Some(app) = body["app"].as_u64() {
        if let Some(volume) = body["volume"].as_f64() {
            // pactl takes a percentage; 150% is as loud as Plasma's own mixer lets an app go.
            let pct = (volume.clamp(0.0, 1.5) * 100.0).round() as u32;
            pactl(&["set-sink-input-volume", &app.to_string(), &format!("{pct}%")]).await?;
        }
        if let Some(muted) = body["muted"].as_bool() {
            pactl(&["set-sink-input-mute", &app.to_string(), if muted { "1" } else { "0" }]).await?;
        }
        return Ok(());
    }
    bail!("nothing to change")
}

/// `vyoma.pc.power {action}`: sleep, restart, shutdown or logout.
pub async fn power(action: &str) -> Result<()> {
    let conn = Connection::session().await.context("no session bus")?;
    let (service, path, iface, method) = match action {
        "sleep" => ("org.kde.Solid.PowerManagement", "/org/kde/Solid/PowerManagement/Actions/SuspendSession", "org.kde.Solid.PowerManagement.Actions.SuspendSession", "suspendToRam"),
        "restart" => ("org.kde.Shutdown", "/Shutdown", "org.kde.Shutdown", "logoutAndReboot"),
        "shutdown" => ("org.kde.Shutdown", "/Shutdown", "org.kde.Shutdown", "logoutAndShutdown"),
        "logout" => ("org.kde.Shutdown", "/Shutdown", "org.kde.Shutdown", "logout"),
        other => bail!("unknown action {other}"),
    };
    conn.call_method(Some(service), path, Some(iface), method, &()).await.with_context(|| format!("Plasma refused to {action}"))?;
    Ok(())
}

/**
 * `vyoma.pc.wake`, sent with the phone's unlock: kdeconnectd unlocks through logind but leaves a
 * screen that went dark (idle DPMS) dark, so "unlock" looked like it did nothing. Turns the
 * displays on, counts as activity (so they don't go straight back off), and unlocks this session
 * too (the same logind call, in case the stock unlock arrived first while the screen was off).
 */
pub async fn wake() -> Result<()> {
    let dpms = tokio::time::timeout(Duration::from_secs(4), tokio::process::Command::new("kscreen-doctor").args(["--dpms", "on"]).kill_on_drop(true).output()).await;
    if !matches!(dpms, Ok(Ok(ref o)) if o.status.success()) {
        warn!("couldn't turn the displays on (kscreen-doctor)");
    }
    let session = Connection::session().await.context("no session bus")?;
    let _ = session.call_method(Some("org.freedesktop.ScreenSaver"), "/org/freedesktop/ScreenSaver", Some("org.freedesktop.ScreenSaver"), "SimulateUserActivity", &()).await;
    let system = Connection::system().await.context("no system bus")?;
    system
        .call_method(Some("org.freedesktop.login1"), "/org/freedesktop/login1/session/auto", Some("org.freedesktop.login1.Session"), "Unlock", &())
        .await
        .context("logind refused to unlock")?;
    Ok(())
}

async fn displays() -> Result<Vec<Value>> {
    let conn = Connection::session().await?;
    let names: Vec<String> = prop(&conn, BRIGHTNESS_PATH, "DisplaysDBusNames").await?.try_into()?;
    let mut out = Vec::new();
    for name in names {
        let path = format!("{BRIGHTNESS_PATH}/{name}");
        let all: std::collections::HashMap<String, OwnedVariant> = conn
            .call_method(Some(BRIGHTNESS), path.as_str(), Some("org.freedesktop.DBus.Properties"), "GetAll", &format!("{BRIGHTNESS}.Display"))
            .await?
            .body()
            .deserialize()?;
        let int = |k: &str| all.get(k).and_then(|v| i32::try_from(v).ok());
        let (Some(value), Some(max)) = (int("Brightness"), int("MaxBrightness")) else { continue };
        if max <= 0 {
            continue;
        }
        out.push(json!({
            "id": name,
            "label": all.get("Label").and_then(|v| String::try_from(v.try_clone().ok()?).ok()).unwrap_or_default(),
            "internal": all.get("IsInternal").and_then(|v| bool::try_from(v).ok()).unwrap_or(false),
            "value": value as f64 / max as f64,
        }));
    }
    Ok(out)
}

async fn set_brightness(display: &str, level: f64) -> Result<()> {
    // Only names the service itself lists: the phone's id becomes part of a D-Bus path.
    if !display.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        bail!("bad display id");
    }
    let conn = Connection::session().await.context("no session bus")?;
    let path = format!("{BRIGHTNESS_PATH}/{display}");
    let max: i32 = conn
        .call_method(Some(BRIGHTNESS), path.as_str(), Some("org.freedesktop.DBus.Properties"), "Get", &(format!("{BRIGHTNESS}.Display"), "MaxBrightness"))
        .await?
        .body()
        .deserialize::<OwnedVariant>()?
        .try_into()?;
    // Never fully dark from the phone: a black screen with no way back from the PC itself is a trap.
    let value = ((level.clamp(0.01, 1.0)) * max as f64).round() as i32;
    // Flags 0: the PC shows its usual brightness indicator, so whoever is at it sees why it changed.
    conn.call_method(Some(BRIGHTNESS), path.as_str(), Some(format!("{BRIGHTNESS}.Display").as_str()), "SetBrightness", &(value, 0u32))
        .await
        .context("Plasma refused the brightness")?;
    Ok(())
}

async fn prop(conn: &Connection, path: &str, name: &str) -> Result<OwnedVariant> {
    Ok(conn
        .call_method(Some(BRIGHTNESS), path, Some("org.freedesktop.DBus.Properties"), "Get", &(BRIGHTNESS, name))
        .await?
        .body()
        .deserialize()?)
}

/// Streams playing to an output, one per app (a browser's tabs are folded into one row, at the
/// loudest; setting it sets them all, see the phone).
async fn apps() -> Result<Vec<Value>> {
    let out = pactl(&["-f", "json", "list", "sink-inputs"]).await?;
    let list: Vec<Value> = serde_json::from_slice(&out).context("pactl's list isn't JSON")?;
    let mut apps = Vec::new();
    for s in list {
        let props = &s["properties"];
        // Event sounds (notifications, the volume feedback pop) come and go; not something to mix.
        if props["media.role"].as_str() == Some("event") {
            continue;
        }
        let Some(id) = s["index"].as_u64() else { continue };
        let name = props["application.name"].as_str().or(props["application.process.binary"].as_str()).unwrap_or("App");
        // The loudest channel, as a fraction of 100%.
        let volume = s["volume"]
            .as_object()
            .map(|chs| chs.values().filter_map(|c| c["value_percent"].as_str()?.trim_end_matches('%').trim().parse::<f64>().ok()).fold(0.0, f64::max) / 100.0)
            .unwrap_or(1.0);
        apps.push(json!({
            "id": id,
            "name": name,
            "binary": props["application.process.binary"].as_str(),
            "title": props["media.name"].as_str(),
            "volume": volume,
            "muted": s["mute"].as_bool().unwrap_or(false),
        }));
    }
    Ok(apps)
}

async fn pactl(args: &[&str]) -> Result<Vec<u8>> {
    let out = tokio::time::timeout(Duration::from_secs(4), tokio::process::Command::new("pactl").args(args).kill_on_drop(true).output())
        .await
        .context("pactl took too long")?
        .context("pactl isn't available")?;
    if !out.status.success() {
        bail!("pactl {}: {}", args.first().unwrap_or(&""), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(out.stdout)
}

#[cfg(test)]
mod tests {
    /// Against this PC: reads the controls, then sets the first display to the brightness it
    /// already has (nothing visibly changes). `cargo test -- --ignored pc`
    #[tokio::test]
    #[ignore]
    async fn reads_and_rewrites_brightness() {
        let s = super::state().await;
        println!("{s:#}");
        if let Some(d) = s["displays"].as_array().and_then(|d| d.first()) {
            super::set(&serde_json::json!({ "display": d["id"], "brightness": d["value"] })).await.unwrap();
            assert_eq!(super::state().await["displays"][0]["value"], d["value"]);
        }
    }

    /// Plays silence as its own stream, then sets and mutes it through `set` (no real app touched).
    #[tokio::test]
    #[ignore]
    async fn sets_an_apps_volume() {
        let mut silence = tokio::process::Command::new("pacat")
            .args(["--raw", "--client-name=VyomaTest", "/dev/zero"])
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        let apps = super::state().await["apps"].clone();
        let app = apps.as_array().unwrap().iter().find(|a| a["name"] == "VyomaTest").expect("test stream listed").clone();
        super::set(&serde_json::json!({ "app": app["id"], "volume": 0.4, "muted": true })).await.unwrap();
        let after = super::state().await["apps"].as_array().unwrap().iter().find(|a| a["id"] == app["id"]).unwrap().clone();
        println!("{app}\n{after}");
        assert!((after["volume"].as_f64().unwrap() - 0.4).abs() < 0.01 && after["muted"] == true);
        silence.kill().await.unwrap();
    }
}
