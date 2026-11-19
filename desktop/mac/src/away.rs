//! Walk-away rules on this Mac: once a phone has been gone for a while, pause Music and Spotify,
//! mute the sound, turn the display off (which locks the Mac under its usual setting), and put the
//! sound back when the phone returns if the rule says so.

use std::sync::Arc;

use serde_json::Value;
use vyoma_core::away::{self, Rules};

use crate::{mac, media};

/// How the Mac carries the rules out: Music and Spotify, the output's mute, the display's sleep.
struct Hands;

impl away::Hands for Hands {
    async fn pause_playing(&self) -> Vec<String> {
        media::pause_playing().await.into_iter().map(|p| p.name().to_owned()).collect()
    }

    async fn resume(&self, players: &[String]) {
        let players: Vec<media::Player> = players.iter().filter_map(|n| media::Player::named(n)).collect();
        media::resume(&players).await;
    }

    async fn mute_if_on(&self) -> bool {
        media::mute_if_on().await
    }

    async fn unmute(&self) {
        media::set_muted(false).await;
    }

    async fn lock(&self) -> anyhow::Result<()> {
        mac::sleep_display().await
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

    /// [phone] linked: cancels a pending walk-away and puts back what one did.
    pub async fn linked(&self, phone: &str) {
        self.rules.linked(phone, &Hands).await;
    }

    /// [phone]'s links are gone. If it stays gone, each rule runs after its own wait.
    pub fn unlinked(self: &Arc<Self>, phone: &str) {
        self.rules.gone(phone, Arc::new(Hands));
    }
}
