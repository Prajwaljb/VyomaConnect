//! What this PC does for the stock plugins, the freedesktop way, so it's the same on KDE, GNOME,
//! Cinnamon and COSMIC: its battery from sysfs, the screen lock through logind (every desktop's
//! locker follows logind's Lock and Unlock), and a ring through PipeWire or PulseAudio.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::StreamExt;
use zbus::{Connection, MatchRule, MessageStream, OwnedObjectPath, OwnedVariant};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Battery {
    pub charge: i64,
    pub charging: bool,
}

/// The PC's own batteries as one (a laptop may have two), or None on a desktop.
pub fn battery() -> Option<Battery> {
    battery_in(Path::new("/sys/class/power_supply"))
}

fn battery_in(dir: &Path) -> Option<Battery> {
    let mut charges = Vec::new();
    let mut charging = false;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let read = |f: &str| std::fs::read_to_string(entry.path().join(f)).unwrap_or_default().trim().to_owned();
        // A mouse's or a phone's battery says scope Device.
        if read("type") != "Battery" || read("scope") == "Device" {
            continue;
        }
        let Ok(charge) = read("capacity").parse::<i64>() else { continue };
        charges.push(charge.clamp(0, 100));
        charging |= read("status") == "Charging";
    }
    if charges.is_empty() {
        return None;
    }
    Some(Battery { charge: charges.iter().sum::<i64>() / charges.len() as i64, charging })
}

const LOGIN: &str = "org.freedesktop.login1";
const SESSION: &str = "org.freedesktop.login1.Session";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

/// The user's graphical session, as logind knows it: looked up on each use, never kept, as it
/// changes under a running service (a new login while the last one is still closing; the service
/// started before logind had moved the user's display over to the new one).
///
/// On GNOME the lock goes through GNOME's own screensaver interface instead: GNOME Shell picks its
/// logind session once as it starts, and after a quick new login that can be the old one still
/// closing, so logind's Lock to the current one went unheard (seen 2026-11-10).
#[derive(Clone)]
pub struct Session {
    conn: Connection,
    gnome: Option<Connection>,
}

const SCREENSAVER: &str = "org.gnome.ScreenSaver";
/// GNOME Shell's lock screen, which it makes only when GDM is the login screen (`canLock`).
const SHIELD: &str = "org.gnome.Shell.ScreenShield";

/// Why the PC can't lock, if it can't: GNOME without GDM (started from a text login, or another
/// login screen) has no lock screen at all.
pub async fn cannot_lock() -> Option<&'static str> {
    if !vyoma_companion::desktop::is_gnome() {
        return None;
    }
    let conn = Connection::session().await.ok()?;
    let reply = conn.call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "NameHasOwner", &(SHIELD,)).await.ok()?;
    (!reply.body().deserialize::<bool>().ok()?).then_some("GNOME locks the screen only when you sign in through its login screen (GDM), so locking from your phone waits for that.")
}
const SCREENSAVER_PATH: &str = "/org/gnome/ScreenSaver";

impl Session {
    pub async fn find(system: Connection) -> Result<Self> {
        let gnome = if vyoma_companion::desktop::is_gnome() { Connection::session().await.ok() } else { None };
        let session = Self { conn: system, gnome };
        session.path().await?;
        Ok(session)
    }

    /// `session/auto` resolves to the user's display session even from a service outside it;
    /// signals come from its real path.
    async fn path(&self) -> Result<OwnedObjectPath> {
        let id: String = Self::get(&self.conn, "/org/freedesktop/login1/session/auto", "Id").await?.try_into()?;
        Ok(self.conn.call_method(Some(LOGIN), "/org/freedesktop/login1", Some("org.freedesktop.login1.Manager"), "GetSession", &(id,)).await?.body().deserialize()?)
    }

    async fn get(conn: &Connection, path: &str, name: &str) -> Result<OwnedVariant> {
        Ok(conn.call_method(Some(LOGIN), path, Some(PROPERTIES), "Get", &(SESSION, name)).await?.body().deserialize()?)
    }

    pub async fn locked(&self) -> Result<bool> {
        if let Some(conn) = &self.gnome {
            if cannot_lock().await.is_some() {
                return Ok(false);
            }
            return Ok(conn.call_method(Some(SCREENSAVER), SCREENSAVER_PATH, Some(SCREENSAVER), "GetActive", &()).await?.body().deserialize::<bool>()?);
        }
        Ok(Self::get(&self.conn, self.path().await?.as_str(), "LockedHint").await?.try_into()?)
    }

    pub async fn set_locked(&self, lock: bool) -> Result<()> {
        if let Some(conn) = &self.gnome {
            if let Some(why) = cannot_lock().await {
                anyhow::bail!("{why}");
            }
            if lock {
                // It answers once the lock screen shows; the state goes out from the watch anyway.
                let call = conn.call_method(Some(SCREENSAVER), SCREENSAVER_PATH, Some(SCREENSAVER), "Lock", &());
                if let Ok(done) = tokio::time::timeout(Duration::from_secs(3), call).await {
                    done.context("GNOME's Lock")?;
                }
            } else {
                conn.call_method(Some(SCREENSAVER), SCREENSAVER_PATH, Some(SCREENSAVER), "SetActive", &(false,)).await.context("GNOME's unlock")?;
            }
            return Ok(());
        }
        let method = if lock { "Lock" } else { "Unlock" };
        let path = self.path().await?;
        self.conn.call_method(Some(LOGIN), path.as_str(), Some(SESSION), method, &()).await.with_context(|| format!("logind {method}"))?;
        Ok(())
    }

    /// Each change of the lock, as it happens: any session's, kept if it's the display session now.
    pub async fn watch(&self, out: tokio::sync::mpsc::UnboundedSender<bool>) -> Result<()> {
        if let Some(conn) = &self.gnome {
            let rule = MatchRule::builder().msg_type(zbus::message::Type::Signal).interface(SCREENSAVER).member("ActiveChanged").path(SCREENSAVER_PATH).build()?;
            let mut stream = MessageStream::for_match_rule(rule, conn, None).await?;
            while let Some(Ok(msg)) = stream.next().await {
                if let Ok(active) = msg.body().deserialize::<bool>()
                    && out.send(active).is_err()
                {
                    break;
                }
            }
            return Ok(());
        }
        let rule = MatchRule::builder().msg_type(zbus::message::Type::Signal).sender(LOGIN).interface(PROPERTIES).member("PropertiesChanged").path_namespace("/org/freedesktop/login1/session").build()?;
        let mut stream = MessageStream::for_match_rule(rule, &self.conn, None).await?;
        while let Some(Ok(msg)) = stream.next().await {
            let Ok((iface, changed, _)) = msg.body().deserialize::<(String, HashMap<String, OwnedVariant>, Vec<String>)>() else { continue };
            if iface != SESSION {
                continue;
            }
            let Some(locked) = changed.get("LockedHint").and_then(|v| bool::try_from(v).ok()) else { continue };
            let from = msg.header().path().map(|p| p.to_string());
            if self.path().await.ok().map(|p| p.to_string()) != from {
                continue;
            }
            if out.send(locked).is_err() {
                break;
            }
        }
        Ok(())
    }
}

/// How long a ring goes on if nobody stops it.
const RING_FOR: Duration = Duration::from_secs(30);
const RING_SOUND: &str = "/usr/share/sounds/freedesktop/stereo/phone-incoming-call.oga";

/// Rings until stopped or [RING_FOR] is up. Dropping it stops the sound.
#[derive(Default)]
pub struct Ringer(Option<tokio::task::JoinHandle<()>>);

impl Ringer {
    pub fn start(&mut self) {
        self.stop();
        self.0 = Some(tokio::spawn(async {
            let _ = tokio::time::timeout(RING_FOR, async {
                loop {
                    if !play(RING_SOUND).await {
                        // No player worked: a silent loop would only spin.
                        break;
                    }
                }
            })
            .await;
        }));
    }

    pub fn stop(&mut self) {
        if let Some(task) = self.0.take() {
            // The player is killed with its task (kill_on_drop).
            task.abort();
        }
    }
}

impl Drop for Ringer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Plays [file] once with the first player there is (PipeWire, PulseAudio, libcanberra).
async fn play(file: &str) -> bool {
    for (player, args) in [("pw-play", vec![file]), ("paplay", vec![file]), ("canberra-gtk-play", vec!["-f", file])] {
        let status = tokio::process::Command::new(player).args(&args).kill_on_drop(true).status().await;
        match status {
            Ok(s) if s.success() => return true,
            Ok(_) => return false,
            Err(_) => continue,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batteries_are_the_systems_own() {
        let dir = std::env::temp_dir().join(format!("vyoma-battery-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let supply = |name: &str, files: &[(&str, &str)]| {
            std::fs::create_dir_all(dir.join(name)).unwrap();
            for (f, v) in files {
                std::fs::write(dir.join(name).join(f), format!("{v}\n")).unwrap();
            }
        };
        supply("AC", &[("type", "Mains"), ("online", "1")]);
        supply("hid-mouse", &[("type", "Battery"), ("scope", "Device"), ("capacity", "5"), ("status", "Discharging")]);
        assert_eq!(battery_in(&dir), None);
        supply("BAT0", &[("type", "Battery"), ("capacity", "80"), ("status", "Charging")]);
        supply("BAT1", &[("type", "Battery"), ("capacity", "60"), ("status", "Not charging")]);
        assert_eq!(battery_in(&dir), Some(Battery { charge: 70, charging: true }));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
