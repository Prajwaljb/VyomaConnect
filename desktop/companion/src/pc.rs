//! PC controls: screen brightness, the volume of each app playing sound (PipeWire through pactl),
//! and sleep, restart, shut down and log out. Each through the desktop's own service where it has
//! one, so the session is saved and apps are asked to close, exactly as from its menu:
//! - Plasma: its ScreenBrightness service (internal panels and DDC monitors alike), its Shutdown
//!   and Solid power services, `kscreen-doctor` to wake the screens.
//! - GNOME (and other desktops on Mutter): Mutter's backlight for built-in panels, `ddcutil` for
//!   monitors, its session manager to restart, shut down or log out (it shows its own 60 s
//!   countdown, which whoever is at the PC can cancel), Mutter's power-save mode to wake.
//! - Anywhere: logind to sleep, `ddcutil` for monitors.
//! Everything runs as the logged-in user; no root.

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
const MUTTER: &str = "org.gnome.Mutter.DisplayConfig";
const MUTTER_PATH: &str = "/org/gnome/Mutter/DisplayConfig";
/// Ids of Mutter's panels: its connector ("eDP-1") with what isn't a letter or digit as `_`.
const MUTTER_ID: &str = "mutter_";

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
    let kde = |path, iface, method| (Some(("org.kde.Shutdown", path, iface, method)), "Plasma");
    let (call, who) = match action {
        "sleep" if has_owner(&conn, "org.kde.Solid.PowerManagement").await => (
            Some(("org.kde.Solid.PowerManagement", "/org/kde/Solid/PowerManagement/Actions/SuspendSession", "org.kde.Solid.PowerManagement.Actions.SuspendSession", "suspendToRam")),
            "Plasma",
        ),
        // Everywhere else logind, which the desktop's own lock-on-sleep follows.
        "sleep" => {
            let system = Connection::system().await.context("no system bus")?;
            system
                .call_method(Some("org.freedesktop.login1"), "/org/freedesktop/login1", Some("org.freedesktop.login1.Manager"), "Suspend", &(true,))
                .await
                .context("the PC refused to sleep")?;
            return Ok(());
        }
        "restart" | "shutdown" | "logout" if has_owner(&conn, "org.kde.Shutdown").await => match action {
            "restart" => kde("/Shutdown", "org.kde.Shutdown", "logoutAndReboot"),
            "shutdown" => kde("/Shutdown", "org.kde.Shutdown", "logoutAndShutdown"),
            _ => kde("/Shutdown", "org.kde.Shutdown", "logout"),
        },
        "restart" | "shutdown" | "logout" if has_owner(&conn, GNOME_SESSION).await => {
            let gnome = |method| Some((GNOME_SESSION, "/org/gnome/SessionManager", GNOME_SESSION, method));
            match action {
                // Mode 1: no "are you sure", the phone already asked.
                "logout" => {
                    conn.call_method(Some(GNOME_SESSION), "/org/gnome/SessionManager", Some(GNOME_SESSION), "Logout", &(1u32,)).await.context("GNOME refused to log out")?;
                    return Ok(());
                }
                "restart" => (gnome("Reboot"), "GNOME"),
                _ => (gnome("Shutdown"), "GNOME"),
            }
        }
        "restart" | "shutdown" | "logout" => (None, ""),
        other => bail!("unknown action {other}"),
    };
    let Some((service, path, iface, method)) = call else {
        bail!("this desktop can't be asked to {action} from the phone yet");
    };
    conn.call_method(Some(service), path, Some(iface), method, &()).await.with_context(|| format!("{who} refused to {action}"))?;
    Ok(())
}

const GNOME_SESSION: &str = "org.gnome.SessionManager";

/// Whether [name] is served on [conn]'s bus right now.
async fn has_owner(conn: &Connection, name: &str) -> bool {
    match conn.call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "NameHasOwner", &(name,)).await {
        Ok(reply) => reply.body().deserialize::<bool>().unwrap_or(false),
        Err(_) => false,
    }
}

/**
 * `vyoma.pc.wake`, sent with the phone's unlock: kdeconnectd unlocks through logind but leaves a
 * screen that went dark (idle DPMS) dark, so "unlock" looked like it did nothing. Turns the
 * displays on, counts as activity (so they don't go straight back off), and unlocks this session
 * too (the same logind call, in case the stock unlock arrived first while the screen was off).
 */
pub async fn wake() -> Result<()> {
    let session = Connection::session().await.context("no session bus")?;
    if has_owner(&session, MUTTER).await {
        // GNOME: power-save mode 0 is "on".
        let on = zbus::Variant::from(0i32);
        if let Err(e) = session.call_method(Some(MUTTER), MUTTER_PATH, Some("org.freedesktop.DBus.Properties"), "Set", &(MUTTER, "PowerSaveMode", on)).await {
            warn!("couldn't turn the displays on (Mutter): {e}");
        }
    } else {
        let dpms = tokio::time::timeout(Duration::from_secs(4), tokio::process::Command::new("kscreen-doctor").args(["--dpms", "on"]).kill_on_drop(true).output()).await;
        if !matches!(dpms, Ok(Ok(ref o)) if o.status.success()) {
            warn!("couldn't turn the displays on (kscreen-doctor)");
        }
    }
    let _ = session.call_method(Some("org.freedesktop.ScreenSaver"), "/org/freedesktop/ScreenSaver", Some("org.freedesktop.ScreenSaver"), "SimulateUserActivity", &()).await;
    let system = Connection::system().await.context("no system bus")?;
    system
        .call_method(Some("org.freedesktop.login1"), "/org/freedesktop/login1/session/auto", Some("org.freedesktop.login1.Session"), "Unlock", &())
        .await
        .context("logind refused to unlock")?;
    Ok(())
}

/// Every screen whose brightness can be set: Plasma's list where it runs, else Mutter's panels
/// and the monitors ddcutil reaches.
async fn displays() -> Result<Vec<Value>> {
    let conn = Connection::session().await?;
    if has_owner(&conn, BRIGHTNESS).await {
        return kde_displays(&conn).await;
    }
    let mut out = mutter_displays(&conn).await.unwrap_or_default();
    for m in crate::ddc::monitors().await {
        if let Ok(value) = crate::ddc::brightness(m.bus).await {
            out.push(json!({ "id": crate::ddc::id(m.bus), "label": m.label, "internal": false, "value": value }));
        }
    }
    Ok(out)
}

/// Mutter's backlights: built-in panels, as `(serial, [{connector, active, min, max, value}])`.
async fn mutter_backlights(conn: &Connection) -> Result<(u32, Vec<std::collections::HashMap<String, OwnedVariant>>)> {
    let v: OwnedVariant = conn
        .call_method(Some(MUTTER), MUTTER_PATH, Some("org.freedesktop.DBus.Properties"), "Get", &(MUTTER, "Backlight"))
        .await?
        .body()
        .deserialize()?;
    Ok(v.try_into()?)
}

fn mutter_id(connector: &str) -> String {
    format!("{MUTTER_ID}{}", connector.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect::<String>())
}

async fn mutter_displays(conn: &Connection) -> Result<Vec<Value>> {
    let (_, lights) = mutter_backlights(conn).await?;
    let mut out = Vec::new();
    for l in lights {
        let int = |k: &str| l.get(k).and_then(|v| i32::try_from(v).ok());
        let (Some(min), Some(max), Some(value)) = (int("min"), int("max"), int("value")) else { continue };
        let Some(connector) = l.get("connector").and_then(|v| String::try_from(v.try_clone().ok()?).ok()) else { continue };
        if max <= min || !l.get("active").and_then(|v| bool::try_from(v).ok()).unwrap_or(true) {
            continue;
        }
        out.push(json!({ "id": mutter_id(&connector), "label": "Built-in display", "internal": true, "value": (value - min) as f64 / (max - min) as f64 }));
    }
    Ok(out)
}

/// Sets it and says which output it was ("eDP-1").
async fn set_mutter_brightness(conn: &Connection, id: &str, level: f64) -> Result<String> {
    let (serial, lights) = mutter_backlights(conn).await?;
    for l in lights {
        let int = |k: &str| l.get(k).and_then(|v| i32::try_from(v).ok());
        let Some(connector) = l.get("connector").and_then(|v| String::try_from(v.try_clone().ok()?).ok()) else { continue };
        if mutter_id(&connector) != id {
            continue;
        }
        let (Some(min), Some(max)) = (int("min"), int("max")) else { bail!("no brightness range") };
        let value = min + ((level.clamp(0.01, 1.0)) * (max - min) as f64).round() as i32;
        conn.call_method(Some(MUTTER), MUTTER_PATH, Some(MUTTER), "SetBacklight", &(serial, connector.as_str(), value.clamp(min, max)))
            .await
            .context("GNOME refused the brightness")?;
        return Ok(connector);
    }
    bail!("that display is gone")
}

async fn kde_displays(conn: &Connection) -> Result<Vec<Value>> {
    let names: Vec<String> = prop(conn, BRIGHTNESS_PATH, "DisplaysDBusNames").await?.try_into()?;
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
    // GNOME shows no pop-up for these (Plasma's own service, below, does): ask its shell for one,
    // on that monitor, so whoever is at the PC sees why it changed.
    if let Some(bus) = crate::ddc::bus_of(display) {
        crate::ddc::set_brightness(bus, level).await?;
        brightness_osd(level, crate::ddc::connector(bus).as_deref()).await;
        return Ok(());
    }
    let conn = Connection::session().await.context("no session bus")?;
    if display.starts_with(MUTTER_ID) {
        let connector = set_mutter_brightness(&conn, display, level).await?;
        brightness_osd(level, Some(&connector)).await;
        return Ok(());
    }
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

async fn brightness_osd(level: f64, connector: Option<&str>) {
    if crate::desktop::is_gnome() {
        let _ = crate::shell::show_osd("display-brightness-symbolic", level.clamp(0.0, 1.0), 1.0, connector).await;
    }
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
        // Only one at 1% or more: below that the phone's floor would change it for real.
        if let Some(d) = s["displays"].as_array().and_then(|d| d.iter().find(|d| d["value"].as_f64().is_some_and(|v| v >= 0.01))) {
            super::set(&serde_json::json!({ "display": d["id"], "brightness": d["value"] })).await.unwrap();
            let after = super::state().await;
            let same = after["displays"].as_array().unwrap().iter().find(|e| e["id"] == d["id"]).unwrap()["value"].clone();
            assert_eq!(same, d["value"]);
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
