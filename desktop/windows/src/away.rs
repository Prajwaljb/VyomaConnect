//! Walk-away rules on Windows (core's [Rules](vyoma_core::away::Rules), the same on every PC side):
//! once a phone has been gone a while, pause what's playing in Windows' media controls, mute the
//! speaker, lock the PC, and put the sound back when it returns if the rule says so.

use anyhow::Result;
use vyoma_core::away;

use crate::{media, win};

pub struct Hands;

/// Runs [f] off the async threads (COM and WinRT wait on their answers).
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    tokio::task::spawn_blocking(f).await.ok()
}

impl away::Hands for Hands {
    async fn pause_playing(&self) -> Vec<String> {
        blocking(media::pause_playing).await.unwrap_or_default()
    }

    async fn resume(&self, apps: &[String]) {
        let apps = apps.to_vec();
        blocking(move || media::resume(&apps)).await;
    }

    async fn mute_if_on(&self) -> bool {
        blocking(media::mute_if_on).await.unwrap_or(false)
    }

    async fn unmute(&self) {
        blocking(media::unmute).await;
    }

    async fn lock(&self) -> Result<()> {
        blocking(win::lock).await.unwrap_or_else(|| anyhow::bail!("couldn't lock"))
    }
}
