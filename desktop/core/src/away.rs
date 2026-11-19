//! Walk-away rules as a phone sends them (`vyoma.away`): what to do on this PC once the phone has
//! been gone a while. The phone can't do these itself: by the time its rule fires it can no longer
//! reach the PC. Each PC side carries them out in its own way.

use serde_json::Value;

pub const AWAY: &str = "vyoma.away";
/// Never act on a link that was gone for less than this (on top of the ~30 s keepalive).
pub const MIN_GONE_SECS: u64 = 10;

#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Want {
    pub on: bool,
    /// Seconds to wait after the link drops before acting.
    pub after: u64,
    /// Put back what we changed (unmute, resume) when the phone returns.
    pub undo: bool,
}

impl Want {
    fn read(v: &Value) -> Want {
        Want { on: v.is_object(), after: v["after"].as_u64().unwrap_or(0).min(24 * 3600), undo: v["undo"].as_bool().unwrap_or(false) }
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Config {
    pub lock: Want,
    pub pause: Want,
    pub mute: Want,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    Pause,
    Mute,
    Lock,
}

impl Config {
    /// A `vyoma.away` body.
    pub fn read(body: &Value) -> Config {
        Config { lock: Want::read(&body["lock"]), pause: Want::read(&body["pause"]), mute: Want::read(&body["mute"]) }
    }

    /// What to do, soonest first; at the same moment, the sound goes before the screen locks.
    pub fn steps(&self) -> Vec<(Step, Want)> {
        let mut steps: Vec<_> = [(Step::Pause, self.pause), (Step::Mute, self.mute), (Step::Lock, self.lock)].into_iter().filter(|(_, w)| w.on).collect();
        steps.sort_by_key(|(_, w)| w.after);
        steps
    }
}

/// What a PC side does for the rules: each one in its own way.
pub trait Hands: Send + Sync + 'static {
    /// Pauses whatever is playing; what to resume later (the side's own names for its players).
    fn pause_playing(&self) -> impl Future<Output = Vec<String>> + Send;
    fn resume(&self, players: &[String]) -> impl Future<Output = ()> + Send;
    /// Mutes the sound; true if that changed anything.
    fn mute_if_on(&self) -> impl Future<Output = bool> + Send;
    fn unmute(&self) -> impl Future<Output = ()> + Send;
    fn lock(&self) -> impl Future<Output = anyhow::Result<()>> + Send;
}

/// What we changed while the phone was away, to put back.
#[derive(Default)]
struct Done {
    unmute: bool,
    resume: Vec<String>,
}

impl Done {
    async fn put_back<H: Hands>(&self, hands: &H) {
        if self.unmute {
            hands.unmute().await;
        }
        if !self.resume.is_empty() {
            hands.resume(&self.resume).await;
        }
    }
}

#[derive(Default)]
struct Phone {
    config: Config,
    links: u32,
    /// Bumped on every link, so a wait started before the phone came back knows it's stale.
    generation: u64,
    done: Option<Done>,
}

/// Every phone's rules and where they stand: the same on every PC side. It acts once a phone has
/// no link left (the keepalive notices a vanished phone within ~30 s) and it hasn't come back by
/// each rule's `after`, and puts back what the rules said to when it returns.
#[derive(Default)]
pub struct Rules {
    phones: std::sync::Mutex<std::collections::HashMap<String, Phone>>,
}

impl Rules {
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

    /// A link to [phone] came up: cancels a pending walk-away and puts back what one did.
    pub async fn linked<H: Hands>(&self, phone: &str, hands: &H) {
        let done = {
            let mut phones = self.phones.lock().unwrap();
            let p = phones.entry(phone.to_owned()).or_default();
            p.links += 1;
            p.generation += 1;
            p.done.take()
        };
        if let Some(done) = done {
            tracing::info!("{phone}: back; undoing what its walk-away rules did");
            done.put_back(hands).await;
        }
    }

    /// One link to [phone] closed (for sides that count each link).
    pub fn unlinked<H: Hands>(self: &std::sync::Arc<Self>, phone: &str, hands: std::sync::Arc<H>) {
        self.drop_links(phone, hands, false);
    }

    /// [phone] has no link left (for sides told once, when it's gone).
    pub fn gone<H: Hands>(self: &std::sync::Arc<Self>, phone: &str, hands: std::sync::Arc<H>) {
        self.drop_links(phone, hands, true);
    }

    fn drop_links<H: Hands>(self: &std::sync::Arc<Self>, phone: &str, hands: std::sync::Arc<H>, all: bool) {
        let (steps, generation) = {
            let mut phones = self.phones.lock().unwrap();
            let Some(p) = phones.get_mut(phone) else { return };
            p.links = if all { 0 } else { p.links.saturating_sub(1) };
            if p.links > 0 || p.config.steps().is_empty() {
                return;
            }
            (p.config.steps(), p.generation)
        };
        let (rules, phone) = (self.clone(), phone.to_owned());
        tokio::spawn(async move {
            let left = tokio::time::Instant::now();
            for (step, want) in steps {
                // A network change or an app update closes the link for a moment: never act on that.
                tokio::time::sleep_until(left + std::time::Duration::from_secs(want.after.max(MIN_GONE_SECS))).await;
                if !rules.still_gone(&phone, generation) {
                    return;
                }
                tracing::info!("{phone}: gone; walk-away rules: {step:?}");
                let mut done = Done::default();
                match step {
                    Step::Pause => done.resume = hands.pause_playing().await,
                    Step::Mute => done.unmute = hands.mute_if_on().await,
                    Step::Lock => {
                        if let Err(e) = hands.lock().await {
                            tracing::warn!("{phone}: couldn't lock: {e:#}");
                        }
                    }
                }
                if !want.undo {
                    continue;
                }
                // Back while that ran: nothing will come to undo it later.
                let back_already = {
                    let mut phones = rules.phones.lock().unwrap();
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
                    done.put_back(hands.as_ref()).await;
                }
            }
        });
    }

    fn still_gone(&self, phone: &str, generation: u64) -> bool {
        self.phones.lock().unwrap().get(phone).is_some_and(|p| p.links == 0 && p.generation == generation)
    }

    #[cfg(test)]
    fn config(&self, phone: &str) -> Config {
        self.phones.lock().unwrap()[phone].config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;
    use std::sync::Mutex;

    /// Remembers what it was asked to do.
    #[derive(Default)]
    struct Log(Mutex<Vec<&'static str>>);

    impl Hands for Log {
        async fn pause_playing(&self) -> Vec<String> {
            self.0.lock().unwrap().push("pause");
            vec!["player".into()]
        }
        async fn resume(&self, _: &[String]) {
            self.0.lock().unwrap().push("resume");
        }
        async fn mute_if_on(&self) -> bool {
            self.0.lock().unwrap().push("mute");
            true
        }
        async fn unmute(&self) {
            self.0.lock().unwrap().push("unmute");
        }
        async fn lock(&self) -> anyhow::Result<()> {
            self.0.lock().unwrap().push("lock");
            Ok(())
        }
    }

    #[test]
    fn reads_the_config() {
        let rules = Rules::default();
        rules.configure("p", &json!({ "lock": { "after": 60, "undo": true } }));
        assert_eq!(rules.config("p"), Config { lock: Want { on: true, after: 60, undo: true }, ..Config::default() });
    }

    #[test]
    fn each_action_keeps_its_own_time() {
        let rules = Rules::default();
        rules.configure("p", &json!({ "lock": { "after": 600 }, "mute": { "after": 0, "undo": true } }));
        let order: Vec<_> = rules.config("p").steps().iter().map(|(s, w)| (*s, w.after)).collect();
        assert_eq!(order, [(Step::Mute, 0), (Step::Lock, 600)]);
    }

    #[tokio::test(start_paused = true)]
    async fn runs_once_gone_and_puts_back_on_return() {
        let (rules, hands) = (Arc::new(Rules::default()), Arc::new(Log::default()));
        rules.configure("p", &json!({ "pause": { "after": 0, "undo": true }, "mute": { "after": 30, "undo": true }, "lock": { "after": 60 } }));
        rules.linked("p", hands.as_ref()).await;
        rules.gone("p", hands.clone());
        tokio::time::sleep(std::time::Duration::from_secs(61)).await;
        assert_eq!(*hands.0.lock().unwrap(), ["pause", "mute", "lock"]);
        rules.linked("p", hands.as_ref()).await;
        assert_eq!(*hands.0.lock().unwrap(), ["pause", "mute", "lock", "unmute", "resume"]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_phone_back_in_time_cancels_the_wait() {
        let (rules, hands) = (Arc::new(Rules::default()), Arc::new(Log::default()));
        rules.configure("p", &json!({ "mute": { "after": 60 } }));
        rules.linked("p", hands.as_ref()).await;
        rules.gone("p", hands.clone());
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        rules.linked("p", hands.as_ref()).await;
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        assert!(hands.0.lock().unwrap().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn counted_links_wait_for_the_last_one() {
        let (rules, hands) = (Arc::new(Rules::default()), Arc::new(Log::default()));
        rules.configure("p", &json!({ "mute": { "after": 0 } }));
        rules.linked("p", hands.as_ref()).await;
        rules.linked("p", hands.as_ref()).await;
        rules.unlinked("p", hands.clone());
        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
        assert!(hands.0.lock().unwrap().is_empty(), "one link is still up");
        rules.unlinked("p", hands.clone());
        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
        assert_eq!(*hands.0.lock().unwrap(), ["mute"]);
    }

    #[test]
    fn a_phone_never_heard_from_is_nobody_gone() {
        assert!(!Rules::default().still_gone("p", 0));
    }
}
