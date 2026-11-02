//! Walk-away rules: what to do on this PC once a phone has been gone for a while (lock, pause,
//! mute), and putting it back when the phone returns. The phone can't do these itself: by the
//! time its rule fires, it can no longer reach the PC. It sends `vyoma.away` with what its rules
//! want whenever they change and on every new link; we act on it when its links have all closed
//! (the keepalive notices a vanished phone within ~30 s) and it hasn't come back in `after`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tracing::{info, warn};
use zbus::Connection;

use crate::media::Media;

#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct Config {
    lock: bool,
    pause: bool,
    mute: bool,
    /// Seconds to wait after the link drops before acting.
    after: u64,
    /// Put back what we changed (unmute, resume) when the phone returns.
    undo: bool,
}

/// What we changed while the phone was away, to put back.
#[derive(Default)]
struct Done {
    unmute: bool,
    resume: Vec<String>,
}

#[derive(Default)]
struct Phone {
    config: Config,
    links: u32,
    /// Bumped on every link, so a timer started before the phone came back knows it's stale.
    generation: u64,
    done: Option<Done>,
}

#[derive(Default)]
pub struct Away {
    phones: Mutex<HashMap<String, Phone>>,
}

impl Away {
    /// `vyoma.away` from [phone]: its walk-away wishes.
    pub fn configure(&self, phone: &str, body: &Value) {
        let config = Config {
            lock: body["lock"].as_bool().unwrap_or(false),
            pause: body["pause"].as_bool().unwrap_or(false),
            mute: body["mute"].as_bool().unwrap_or(false),
            after: body["after"].as_u64().unwrap_or(0).min(24 * 3600),
            undo: body["undo"].as_bool().unwrap_or(false),
        };
        let mut phones = self.phones.lock().unwrap();
        let entry = phones.entry(phone.to_string()).or_default();
        if entry.config != config {
            info!("{phone}: walk-away rules now {config:?}");
        }
        entry.config = config;
    }

    /// A link to [phone] came up: cancels a pending walk-away and puts back what one did.
    pub async fn linked(&self, phone: &str, media: Option<&Arc<Media>>) {
        let done = {
            let mut phones = self.phones.lock().unwrap();
            let p = phones.entry(phone.to_string()).or_default();
            p.links += 1;
            p.generation += 1;
            p.done.take()
        };
        if let Some(done) = done {
            info!("{phone}: back; undoing what its walk-away rules did");
            if done.unmute {
                let _ = pactl(&["set-sink-mute", "@DEFAULT_SINK@", "0"]).await;
            }
            if let Some(media) = media {
                for bus in &done.resume {
                    media.play_bus(bus).await;
                }
            }
        }
    }

    /// A link to [phone] closed. Once none are left and it stays gone for its `after`, runs its rules.
    pub fn unlinked(self: &Arc<Self>, phone: &str, media: Option<Arc<Media>>) {
        let (config, generation) = {
            let mut phones = self.phones.lock().unwrap();
            let Some(p) = phones.get_mut(phone) else { return };
            p.links = p.links.saturating_sub(1);
            if p.links > 0 || !(p.config.lock || p.config.pause || p.config.mute) {
                return;
            }
            (p.config, p.generation)
        };
        let (away, phone) = (self.clone(), phone.to_string());
        tokio::spawn(async move {
            // A Wi-Fi/USB swap or an app update closes the link for a moment: never act on that.
            tokio::time::sleep(Duration::from_secs(config.after.max(MIN_GONE_SECS))).await;
            let still_gone = away.phones.lock().unwrap().get(&phone).is_some_and(|p| p.links == 0 && p.generation == generation);
            if !still_gone {
                return;
            }
            info!("{phone}: gone; running its walk-away rules");
            let mut done = Done::default();
            if config.pause {
                if let Some(media) = &media {
                    done.resume = media.pause_all().await;
                }
            }
            if config.mute {
                let muted = pactl(&["get-sink-mute", "@DEFAULT_SINK@"]).await.is_ok_and(|out| String::from_utf8_lossy(&out).contains("yes"));
                if !muted && pactl(&["set-sink-mute", "@DEFAULT_SINK@", "1"]).await.is_ok() {
                    done.unmute = true;
                }
            }
            if config.lock {
                if let Err(e) = lock().await {
                    warn!("{phone}: couldn't lock: {e:#}");
                }
            }
            if config.undo {
                let mut phones = away.phones.lock().unwrap();
                if let Some(p) = phones.get_mut(&phone).filter(|p| p.generation == generation) {
                    p.done = Some(done);
                }
            }
        });
    }
}

/// Never act on a link that was gone for less than this (on top of the ~30 s keepalive).
const MIN_GONE_SECS: u64 = 10;

/// Locks this session the way kdeconnectd's lock plugin does: through logind.
async fn lock() -> anyhow::Result<()> {
    let system = Connection::system().await?;
    system
        .call_method(Some("org.freedesktop.login1"), "/org/freedesktop/login1/session/auto", Some("org.freedesktop.login1.Session"), "Lock", &())
        .await?;
    Ok(())
}

async fn pactl(args: &[&str]) -> anyhow::Result<Vec<u8>> {
    let out = tokio::time::timeout(Duration::from_secs(4), tokio::process::Command::new("pactl").args(args).kill_on_drop(true).output()).await??;
    anyhow::ensure!(out.status.success(), "pactl {} failed", args.first().unwrap_or(&""));
    Ok(out.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_the_config() {
        let away = Away::default();
        away.configure("p", &json!({ "lock": true, "after": 60, "undo": true }));
        let c = away.phones.lock().unwrap()["p"].config;
        assert_eq!(c, Config { lock: true, pause: false, mute: false, after: 60, undo: true });
    }

    #[tokio::test]
    async fn a_returning_phone_cancels_the_timer() {
        let away = Arc::new(Away::default());
        away.configure("p", &json!({ "mute": true, "after": 0 }));
        away.linked("p", None).await;
        away.unlinked("p", None);
        away.linked("p", None).await;
        let p = &away.phones.lock().unwrap()["p"];
        assert_eq!((p.links, p.generation), (1, 2));
    }
}
