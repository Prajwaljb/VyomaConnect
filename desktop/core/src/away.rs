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
