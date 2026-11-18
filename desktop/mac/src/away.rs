//! Walk-away rules on this Mac: once a phone has been gone for a while, pause Music and Spotify,
//! mute the sound, turn the display off (which locks the Mac under its usual setting), and put the
//! sound back when the phone returns if the rule says so.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use vyoma_core::away::{Config, MIN_GONE_SECS, Step};

use crate::{mac, media};

/// What we changed while the phone was away, to put back.
#[derive(Default)]
struct Done {
    unmute: bool,
    resume: Vec<media::Player>,
}

impl Done {
    async fn put_back(&self) {
        if self.unmute {
            media::set_muted(false).await;
        }
        media::resume(&self.resume).await;
    }
}

#[derive(Default)]
struct Phone {
    config: Config,
    linked: bool,
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
        let config = Config::read(body);
        let mut phones = self.phones.lock().unwrap();
        let entry = phones.entry(phone.to_owned()).or_default();
        if entry.config != config {
            tracing::info!("{phone}: walk-away rules now {config:?}");
        }
        entry.config = config;
    }

    /// [phone] linked: cancels a pending walk-away and puts back what one did.
    pub async fn linked(&self, phone: &str) {
        let done = {
            let mut phones = self.phones.lock().unwrap();
            let p = phones.entry(phone.to_owned()).or_default();
            p.linked = true;
            p.generation += 1;
            p.done.take()
        };
        if let Some(done) = done {
            tracing::info!("{phone}: back; undoing what its walk-away rules did");
            done.put_back().await;
        }
    }

    /// [phone]'s links are gone. If it stays gone, each rule runs after its own wait.
    pub fn unlinked(self: &Arc<Self>, phone: &str) {
        let (steps, generation) = {
            let mut phones = self.phones.lock().unwrap();
            let Some(p) = phones.get_mut(phone) else { return };
            p.linked = false;
            (p.config.steps(), p.generation)
        };
        if steps.is_empty() {
            return;
        }
        let (away, phone) = (self.clone(), phone.to_owned());
        tokio::spawn(async move {
            let left = tokio::time::Instant::now();
            for (step, want) in steps {
                // A network change or an app update closes the link for a moment: never act on that.
                tokio::time::sleep_until(left + Duration::from_secs(want.after.max(MIN_GONE_SECS))).await;
                if !away.still_gone(&phone, generation) {
                    return;
                }
                tracing::info!("{phone}: gone; walk-away rules: {step:?}");
                let mut done = Done::default();
                match step {
                    Step::Pause => done.resume = media::pause_playing().await,
                    Step::Mute => done.unmute = media::mute_if_on().await,
                    Step::Lock => {
                        if let Err(e) = mac::sleep_display().await {
                            tracing::warn!("{phone}: couldn't lock: {e:#}");
                        }
                    }
                }
                if !want.undo {
                    continue;
                }
                // Back while that ran: nothing will come to undo it later.
                let back_already = {
                    let mut phones = away.phones.lock().unwrap();
                    match phones.get_mut(&phone).filter(|p| p.generation == generation) {
                        Some(p) => {
                            let all = p.done.get_or_insert_default();
                            all.unmute |= done.unmute;
                            all.resume.append(&mut done.resume);
                            false
                        }
                        None => true,
                    }
                };
                if back_already {
                    done.put_back().await;
                }
            }
        });
    }

    fn still_gone(&self, phone: &str, generation: u64) -> bool {
        self.phones.lock().unwrap().get(phone).is_some_and(|p| !p.linked && p.generation == generation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn a_returning_phone_cancels_the_wait() {
        let away = Arc::new(Away::default());
        away.configure("p", &json!({ "lock": { "after": 60 } }));
        away.linked("p").await;
        away.unlinked("p");
        let left = away.phones.lock().unwrap()["p"].generation;
        assert!(away.still_gone("p", left));
        away.linked("p").await;
        assert!(!away.still_gone("p", left));
    }

    #[test]
    fn a_phone_never_heard_from_is_nobody_gone() {
        assert!(!Away::default().still_gone("p", 0));
    }
}
