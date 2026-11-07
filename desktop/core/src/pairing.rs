//! KDE Connect pairing for one device, as the phone does it (`Device.kt`): a request carries a
//! timestamp, both screens show a code made from both public keys and that timestamp, and the
//! certificate whose code was shown is the only one that can end up trusted.
//!
//! No sockets or clocks here: the caller passes the time in, sends what comes back, and keeps
//! the trust store, so every rule can be tested on its own.

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::cert;
use crate::link::packet;

pub const PAIR: &str = "kdeconnect.pair";
/// How long a request waits for an answer.
pub const TIMEOUT: Duration = Duration::from_secs(30);
/// A peer that let a prompt run out waits this long before it can show another.
pub const PROMPT_COOLDOWN: Duration = Duration::from_secs(60);
/// The timestamp goes into the code, so it must be close to ours: left free, a sender could try
/// timestamps until the 8 characters come out as it needs.
pub const CLOCK_SKEW_S: i64 = 30 * 60;

/// The 8 characters both sides show: SHA-256 over the larger public key, the smaller one (compared
/// as unsigned bytes) and the decimal timestamp, as hex in capitals. None if a certificate's unreadable.
pub fn verification_key(ours: &[u8], theirs: &[u8], timestamp: Option<i64>) -> Option<String> {
    let (mut a, mut b) = (cert::public_key(ours)?, cert::public_key(theirs)?);
    if a < b {
        std::mem::swap(&mut a, &mut b);
    }
    let mut ctx = ring::digest::Context::new(&ring::digest::SHA256);
    ctx.update(a);
    ctx.update(b);
    if let Some(ts) = timestamp {
        ctx.update(ts.to_string().as_bytes());
    }
    let hex: String = ctx.finish().as_ref().iter().take(4).map(|b| format!("{b:02X}")).collect();
    Some(hex)
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Unpaired,
    /// We asked; [cert] is the peer's certificate the code was made with.
    RequestedByUs { timestamp: i64, cert: Vec<u8>, since: Instant },
    /// They asked and the user is being shown the code.
    RequestedByPeer { timestamp: i64, cert: Vec<u8>, since: Instant },
    Paired,
}

/// What the caller has to do about a change.
#[derive(Debug, PartialEq)]
pub enum Change {
    /// Show the user [key] with Accept and Reject.
    Prompt { key: String },
    /// Pin this certificate.
    Paired { cert: Vec<u8> },
    /// Forget the device: unpaired, or the pairing was refused or ran out.
    Unpaired,
}

#[derive(Debug, Default, PartialEq)]
pub struct Outcome {
    pub reply: Option<Value>,
    pub change: Option<Change>,
}

pub struct Pairing {
    state: State,
    /// Our own certificate, for the code.
    ours: Vec<u8>,
    prompt_expired: Option<Instant>,
}

fn pair(yes: bool) -> Value {
    packet(PAIR, json!({ "pair": yes }))
}

impl Pairing {
    pub fn new(ours: Vec<u8>, paired: bool) -> Self {
        Self { state: if paired { State::Paired } else { State::Unpaired }, ours, prompt_expired: None }
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn is_paired(&self) -> bool {
        self.state == State::Paired
    }

    /// The code on screen while a pairing is pending.
    pub fn key(&self) -> Option<String> {
        match &self.state {
            State::RequestedByUs { timestamp, cert, .. } | State::RequestedByPeer { timestamp, cert, .. } => verification_key(&self.ours, cert, Some(*timestamp)),
            _ => None,
        }
    }

    /// Asks [peer_cert]'s owner to pair; the packet to send, or None when already paired or asking.
    pub fn request(&mut self, peer_cert: &[u8], now_s: i64, now: Instant) -> Option<Value> {
        if matches!(self.state, State::Paired | State::RequestedByUs { .. }) {
            return None;
        }
        self.state = State::RequestedByUs { timestamp: now_s, cert: peer_cert.to_vec(), since: now };
        Some(packet(PAIR, json!({ "pair": true, "timestamp": now_s })))
    }

    /// The user accepted the code on screen.
    pub fn accept(&mut self) -> Outcome {
        let State::RequestedByPeer { cert, .. } = std::mem::replace(&mut self.state, State::Paired) else {
            return Outcome::default();
        };
        Outcome { reply: Some(pair(true)), change: Some(Change::Paired { cert }) }
    }

    pub fn reject(&mut self) -> Outcome {
        if !matches!(self.state, State::RequestedByPeer { .. }) {
            return Outcome::default();
        }
        self.state = State::Unpaired;
        Outcome { reply: Some(pair(false)), change: Some(Change::Unpaired) }
    }

    pub fn unpair(&mut self) -> Outcome {
        self.state = State::Unpaired;
        Outcome { reply: Some(pair(false)), change: Some(Change::Unpaired) }
    }

    /// The device's last link closed: a pending request can't finish.
    pub fn link_lost(&mut self) -> Option<Change> {
        if matches!(self.state, State::RequestedByUs { .. } | State::RequestedByPeer { .. }) {
            self.state = State::Unpaired;
            return Some(Change::Unpaired);
        }
        None
    }

    /// When a pending request runs out, if one is.
    pub fn deadline(&self) -> Option<Instant> {
        match &self.state {
            State::RequestedByUs { since, .. } | State::RequestedByPeer { since, .. } => Some(*since + TIMEOUT),
            _ => None,
        }
    }

    /// Ends a request that has run out.
    pub fn tick(&mut self, now: Instant) -> Option<Change> {
        if self.deadline().is_none_or(|d| now < d) {
            return None;
        }
        if matches!(self.state, State::RequestedByPeer { .. }) {
            self.prompt_expired = Some(now);
        }
        self.state = State::Unpaired;
        Some(Change::Unpaired)
    }

    /// A `kdeconnect.pair` from the peer, over a link showing [peer_cert]. [protocol]: its version.
    pub fn on_packet(&mut self, body: &Value, peer_cert: &[u8], protocol: i64, now_s: i64, now: Instant) -> Outcome {
        let Some(wants) = body["pair"].as_bool() else { return Outcome::default() };
        if !wants {
            let was = std::mem::replace(&mut self.state, State::Unpaired);
            return Outcome { reply: None, change: (was != State::Unpaired).then_some(Change::Unpaired) };
        }
        match &self.state {
            State::RequestedByUs { cert, .. } => {
                let cert = cert.clone();
                self.state = State::Paired;
                Outcome { reply: None, change: Some(Change::Paired { cert }) }
            }
            // It lost its trust (reinstalled, data cleared) while we kept ours: confirm again.
            State::Paired => Outcome { reply: Some(pair(true)), change: None },
            State::Unpaired | State::RequestedByPeer { .. } => {
                let timestamp = body["timestamp"].as_i64().or_else(|| body["timestamp"].as_str()?.parse().ok());
                let allowed = match timestamp {
                    Some(ts) => (now_s - ts).abs() <= CLOCK_SKEW_S,
                    None => protocol < 8,
                };
                if !allowed {
                    self.state = State::Unpaired;
                    return Outcome { reply: Some(pair(false)), change: None };
                }
                // Asking again doesn't restart the prompt, and one left to run out has to wait:
                // otherwise a peer could hold the screen for good.
                let (repeat, since) = match &self.state {
                    State::RequestedByPeer { timestamp: t, since, .. } => (Some(*t), *since),
                    _ => (None, now),
                };
                if repeat.is_some() && repeat == timestamp {
                    return Outcome::default();
                }
                if repeat.is_none() && self.prompt_expired.is_some_and(|at| now.duration_since(at) < PROMPT_COOLDOWN) {
                    return Outcome { reply: Some(pair(false)), change: None };
                }
                // A protocol 7 peer sends no time: its code is made without one.
                self.state = State::RequestedByPeer { timestamp: timestamp.unwrap_or(0), cert: peer_cert.to_vec(), since };
                let key = if timestamp.is_some() { self.key() } else { verification_key(&self.ours, peer_cert, None) };
                match key {
                    Some(key) => Outcome { reply: None, change: Some(Change::Prompt { key }) },
                    None => {
                        self.state = State::Unpaired;
                        Outcome { reply: Some(pair(false)), change: None }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::{CertificateParams, DnType, KeyPair};

    fn cert() -> Vec<u8> {
        let key = KeyPair::generate().unwrap();
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.distinguished_name.push(DnType::CommonName, "0123456789abcdef");
        params.self_signed(&key).unwrap().der().to_vec()
    }

    const NOW: i64 = 1_800_000_000;

    #[test]
    fn both_sides_get_the_same_code() {
        let (a, b) = (cert(), cert());
        let ab = verification_key(&a, &b, Some(NOW)).unwrap();
        assert_eq!(ab, verification_key(&b, &a, Some(NOW)).unwrap());
        assert_eq!(ab.len(), 8);
        assert!(ab.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_lowercase()));
        assert_ne!(ab, verification_key(&a, &b, Some(NOW + 1)).unwrap());
    }

    #[test]
    fn a_request_prompts_then_pairs_with_that_certificate() {
        let (ours, theirs) = (cert(), cert());
        let mut p = Pairing::new(ours.clone(), false);
        let t = Instant::now();
        let out = p.on_packet(&json!({"pair": true, "timestamp": NOW}), &theirs, 8, NOW, t);
        assert_eq!(out.change, Some(Change::Prompt { key: verification_key(&ours, &theirs, Some(NOW)).unwrap() }));
        // Asking again with the same time changes nothing.
        assert_eq!(p.on_packet(&json!({"pair": true, "timestamp": NOW}), &theirs, 8, NOW, t), Outcome::default());
        let out = p.accept();
        assert_eq!(out.reply.unwrap()["body"], json!({"pair": true}));
        assert_eq!(out.change, Some(Change::Paired { cert: theirs }));
        assert!(p.is_paired());
    }

    #[test]
    fn a_far_off_or_missing_time_is_refused() {
        let theirs = cert();
        let mut p = Pairing::new(cert(), false);
        let t = Instant::now();
        for body in [json!({"pair": true, "timestamp": NOW - CLOCK_SKEW_S - 1}), json!({"pair": true})] {
            let out = p.on_packet(&body, &theirs, 8, NOW, t);
            assert_eq!(out.reply.unwrap()["body"]["pair"], false);
            assert_eq!(*p.state(), State::Unpaired);
        }
    }

    #[test]
    fn our_request_pairs_on_yes_and_runs_out() {
        let theirs = cert();
        let mut p = Pairing::new(cert(), false);
        let t = Instant::now();
        assert_eq!(p.request(&theirs, NOW, t).unwrap()["body"]["timestamp"], NOW);
        assert_eq!(p.request(&theirs, NOW, t), None);
        assert_eq!(p.on_packet(&json!({"pair": true}), &theirs, 8, NOW, t).change, Some(Change::Paired { cert: theirs.clone() }));

        let mut p = Pairing::new(cert(), false);
        p.request(&theirs, NOW, t);
        assert_eq!(p.tick(t + TIMEOUT / 2), None);
        assert_eq!(p.tick(t + TIMEOUT), Some(Change::Unpaired));
    }

    #[test]
    fn a_prompt_left_to_run_out_waits_before_the_next() {
        let theirs = cert();
        let mut p = Pairing::new(cert(), false);
        let t = Instant::now();
        p.on_packet(&json!({"pair": true, "timestamp": NOW}), &theirs, 8, NOW, t);
        p.tick(t + TIMEOUT);
        let soon = p.on_packet(&json!({"pair": true, "timestamp": NOW + 31}), &theirs, 8, NOW + 31, t + TIMEOUT + Duration::from_secs(1));
        assert_eq!(soon.reply.unwrap()["body"]["pair"], false);
        let later = p.on_packet(&json!({"pair": true, "timestamp": NOW + 91}), &theirs, 8, NOW + 91, t + TIMEOUT + PROMPT_COOLDOWN);
        assert!(matches!(later.change, Some(Change::Prompt { .. })));
    }

    #[test]
    fn paired_peers_are_confirmed_and_can_unpair() {
        let mut p = Pairing::new(cert(), true);
        let t = Instant::now();
        assert_eq!(p.on_packet(&json!({"pair": true}), &cert(), 8, NOW, t).reply.unwrap()["body"]["pair"], true);
        assert_eq!(p.on_packet(&json!({"pair": false}), &cert(), 8, NOW, t).change, Some(Change::Unpaired));
        assert_eq!(p.on_packet(&json!({"pair": false}), &cert(), 8, NOW, t).change, None);
    }
}
