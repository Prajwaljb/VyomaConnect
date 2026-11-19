//! Walk-away rules: what to do on this PC once a phone has been gone for a while (lock, pause,
//! mute), and putting it back when the phone returns. The phone can't do these itself: by the
//! time its rule fires, it can no longer reach the PC. It sends `vyoma.away` with what its rules
//! want whenever they change and on every new link; we act on it when its links have all closed
//! (the keepalive notices a vanished phone within ~30 s) and it hasn't come back in `after`.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use zbus::Connection;

use vyoma_core::away::{self, Rules};

use crate::media::Media;

/// How Linux carries the rules out: MPRIS players, PipeWire's default sink, logind's lock.
struct Hands {
    media: Option<Arc<Media>>,
}

impl away::Hands for Hands {
    async fn pause_playing(&self) -> Vec<String> {
        match &self.media {
            Some(media) => media.pause_all().await,
            None => Vec::new(),
        }
    }

    async fn resume(&self, buses: &[String]) {
        if let Some(media) = &self.media {
            for bus in buses {
                media.play_bus(bus).await;
            }
        }
    }

    async fn mute_if_on(&self) -> bool {
        let muted = pactl(&["get-sink-mute", "@DEFAULT_SINK@"]).await.is_ok_and(|out| String::from_utf8_lossy(&out).contains("yes"));
        !muted && pactl(&["set-sink-mute", "@DEFAULT_SINK@", "1"]).await.is_ok()
    }

    async fn unmute(&self) {
        let _ = pactl(&["set-sink-mute", "@DEFAULT_SINK@", "0"]).await;
    }

    async fn lock(&self) -> anyhow::Result<()> {
        lock().await
    }
}

/// Every phone's walk-away rules: the shared [Rules], carried out with [Hands].
#[derive(Default)]
pub struct Away {
    rules: Arc<Rules>,
}

impl Away {
    /// `vyoma.away` from [phone]: its walk-away wishes.
    pub fn configure(&self, phone: &str, body: &Value) {
        self.rules.configure(phone, body);
    }

    /// A link to [phone] came up: cancels a pending walk-away and puts back what one did.
    pub async fn linked(&self, phone: &str, media: Option<&Arc<Media>>) {
        self.rules.linked(phone, &Hands { media: media.cloned() }).await;
    }

    /// A link to [phone] closed. Once none are left and it stays gone for its `after`, runs its rules.
    pub fn unlinked(self: &Arc<Self>, phone: &str, media: Option<Arc<Media>>) {
        self.rules.unlinked(phone, Arc::new(Hands { media }));
    }
}

/// Locks this session the way kdeconnectd's lock plugin does: through logind. GNOME through its
/// own screensaver, as the phone's Lock button does there (its lock screen is GNOME Shell's).
async fn lock() -> anyhow::Result<()> {
    if crate::desktop::is_gnome() {
        let conn = Connection::session().await?;
        let call = conn.call_method(Some("org.gnome.ScreenSaver"), "/org/gnome/ScreenSaver", Some("org.gnome.ScreenSaver"), "Lock", &());
        // It answers once the lock screen is up; without GDM GNOME has none and says so.
        if let Ok(done) = tokio::time::timeout(Duration::from_secs(3), call).await {
            done?;
        }
        return Ok(());
    }
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
