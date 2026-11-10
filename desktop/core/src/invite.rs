//! Pairing by QR code: the PC's window shows a code holding this PC's address, its certificate's
//! fingerprint and a fresh secret; a phone that scans it dials that address, checks the certificate
//! against the fingerprint and sends its pair request with a proof made from the secret. The PC
//! pairs it on that proof, with no code to compare: seeing the code on this PC's screen is the
//! consent, and both certificates are bound into the proof, so a relay can't use it for itself.
//!
//! A secret serves one phone and lasts [LIFETIME]; showing the code again makes a new one. Stock
//! KDE Connect ignores the field, and a request without a good proof is prompted for as before.

use std::time::{Duration, Instant};

use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::Value;

/// The pair request's field that carries the proof.
pub const FIELD: &str = "vyomaInvite";
/// How long a code on screen can be used.
pub const LIFETIME: Duration = Duration::from_secs(10 * 60);
pub const SECRET_LEN: usize = 16;

/// One code's secret, while it lasts.
pub struct Invite {
    secret: [u8; SECRET_LEN],
    until: Instant,
}

impl std::fmt::Debug for Invite {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("Invite(..)")
    }
}

impl Invite {
    pub fn new(now: Instant) -> Option<Self> {
        let mut secret = [0u8; SECRET_LEN];
        SystemRandom::new().fill(&mut secret).ok()?;
        Some(Self { secret, until: now + LIFETIME })
    }

    pub fn secret(&self) -> &[u8; SECRET_LEN] {
        &self.secret
    }

    /// Whether a pair request's [body], over a link showing [phone_cert], proves its sender scanned
    /// this code: the proof over its timestamp and both certificates, made with the secret.
    pub fn proves(&self, body: &Value, phone_cert: &[u8], pc_cert: &[u8], now: Instant) -> bool {
        if now >= self.until {
            return false;
        }
        let (Some(proof), Some(timestamp)) = (body[FIELD].as_str(), body["timestamp"].as_i64()) else { return false };
        let Some(proof) = from_hex(proof) else { return false };
        let key = hmac::Key::new(hmac::HMAC_SHA256, &self.secret);
        hmac::verify(&key, &message(timestamp, phone_cert, pc_cert), &proof).is_ok()
    }
}

/// The proof a phone sends, as lowercase hex: HMAC-SHA256 with the code's secret over
/// "vyoma-invite-v1|<timestamp>|<phone cert SHA-256>|<PC cert SHA-256>" (fingerprints in lowercase hex).
pub fn proof(secret: &[u8], timestamp: i64, phone_cert: &[u8], pc_cert: &[u8]) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret);
    hex(hmac::sign(&key, &message(timestamp, phone_cert, pc_cert)).as_ref())
}

/// A certificate's SHA-256, as the code carries it and the phone checks it.
pub fn fingerprint(cert: &[u8]) -> String {
    hex(ring::digest::digest(&ring::digest::SHA256, cert).as_ref())
}

fn message(timestamp: i64, phone_cert: &[u8], pc_cert: &[u8]) -> Vec<u8> {
    format!("vyoma-invite-v1|{timestamp}|{}|{}", fingerprint(phone_cert), fingerprint(pc_cert)).into_bytes()
}

/// This PC's addresses a phone on the same network can dial, best first: IPv4 on interfaces that
/// are up, private (home and office) networks before others. Not loopback, and not containers',
/// VMs' or VPNs' own interfaces, which a phone can't reach. At most [MAX_ADDRESSES], to keep the code small.
pub fn local_addresses() -> Vec<std::net::IpAddr> {
    let mut found: Vec<(bool, std::net::Ipv4Addr)> = Vec::new();
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills a list we walk read-only and free once.
    unsafe {
        if libc::getifaddrs(&mut list) != 0 {
            return Vec::new();
        }
        let mut at = list;
        while let Some(i) = at.as_ref() {
            at = i.ifa_next;
            if i.ifa_addr.is_null() || i32::from((*i.ifa_addr).sa_family) != libc::AF_INET || i.ifa_flags & libc::IFF_UP as u32 == 0 || i.ifa_flags & libc::IFF_LOOPBACK as u32 != 0 {
                continue;
            }
            let name = std::ffi::CStr::from_ptr(i.ifa_name).to_string_lossy();
            if SKIP.iter().any(|p| name.starts_with(p)) {
                continue;
            }
            let sin = &*(i.ifa_addr as *const libc::sockaddr_in);
            let ip = std::net::Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
            if !ip.is_link_local() && !found.iter().any(|(_, f)| *f == ip) {
                found.push((ip.is_private(), ip));
            }
        }
        libc::freeifaddrs(list);
    }
    found.sort_by_key(|(private, _)| !private);
    found.into_iter().take(MAX_ADDRESSES).map(|(_, ip)| ip.into()).collect()
}

pub const MAX_ADDRESSES: usize = 3;
/// Interfaces of containers, VMs and VPNs, by name.
const SKIP: &[&str] = &["docker", "br-", "veth", "virbr", "vmnet", "vboxnet", "lxc", "lxd", "podman", "cni", "flannel", "tun", "tap", "wg", "tailscale", "zt"];

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 || s.len() > 128 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn proves_only_with_its_secret_and_both_certificates() {
        let now = Instant::now();
        let invite = Invite::new(now).unwrap();
        let (phone, pc) = (b"phone cert".as_slice(), b"pc cert".as_slice());
        let good = proof(invite.secret(), 1_800_000_000, phone, pc);
        assert!(invite.proves(&json!({ "pair": true, "timestamp": 1_800_000_000, FIELD: good }), phone, pc, now));
        // Another time, another phone's certificate, another PC's, another secret: no.
        assert!(!invite.proves(&json!({ "timestamp": 1_800_000_001, FIELD: good }), phone, pc, now));
        assert!(!invite.proves(&json!({ "timestamp": 1_800_000_000, FIELD: good }), b"other phone", pc, now));
        assert!(!invite.proves(&json!({ "timestamp": 1_800_000_000, FIELD: good }), phone, b"relay", now));
        let other = Invite::new(now).unwrap();
        assert!(!other.proves(&json!({ "timestamp": 1_800_000_000, FIELD: good }), phone, pc, now));
        // Run out, or missing and malformed proofs: no.
        assert!(!invite.proves(&json!({ "timestamp": 1_800_000_000, FIELD: good }), phone, pc, now + LIFETIME));
        assert!(!invite.proves(&json!({ "timestamp": 1_800_000_000 }), phone, pc, now));
        assert!(!invite.proves(&json!({ "timestamp": 1_800_000_000, FIELD: "zz" }), phone, pc, now));
    }

    #[test]
    fn matches_the_phone() {
        // Worked out apart (Python's hmac), and checked by the phone's QrInviteTest too.
        assert_eq!(proof(&[7u8; 16], 1_700_000_000, b"phone", b"pc"), "1b9f3d973c838fe866cb806b72ef20448ccb55c320a514b35e9e03808d00069b");
    }
}
