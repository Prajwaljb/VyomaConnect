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
/** How long a code still works once it's off the screen: a phone may be mid-scan. */
pub const AFTER_SHOWN: Duration = Duration::from_secs(20);

/// One code's secret, while it lasts.
pub struct Invite {
    secret: [u8; SECRET_LEN],
    until: Instant,
    hidden: bool,
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
        Some(Self { secret, until: now + LIFETIME, hidden: false })
    }

    /// Off the screen: it works [AFTER_SHOWN] longer, for a phone that was already scanning.
    pub fn hide(&mut self, now: Instant) {
        self.until = self.until.min(now + AFTER_SHOWN);
        self.hidden = true;
    }

    /// Still on the screen (not [hide]n): whoever's at the PC sees what a scan does.
    pub fn on_screen(&self) -> bool {
        !self.hidden
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
#[cfg(unix)]
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

/// Windows: the adapters that are up, by their own names (Hyper-V's, VMs' and VPNs' skipped).
#[cfg(windows)]
pub fn local_addresses() -> Vec<std::net::IpAddr> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, GetAdaptersAddresses, IP_ADAPTER_ADDRESSES_LH};
    use windows_sys::Win32::Networking::WinSock::{AF_INET, SOCKADDR_IN};
    const UP: i32 = 1; // IfOperStatusUp
    const LOOPBACK: u32 = 24; // IF_TYPE_SOFTWARE_LOOPBACK
    let wide = |p: *const u16| {
        // SAFETY: a NUL-terminated UTF-16 string the adapter list owns.
        unsafe { (!p.is_null()).then(|| String::from_utf16_lossy(std::slice::from_raw_parts(p, (0..).take_while(|&i| *p.add(i) != 0).count()))).unwrap_or_default() }
    };
    let mut size: u32 = 16 * 1024;
    let mut buf: Vec<u64> = Vec::new();
    let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
    for _ in 0..3 {
        buf.resize((size as usize).div_ceil(8), 0);
        // SAFETY: the buffer is [size] bytes, aligned for the structures written into it.
        match unsafe { GetAdaptersAddresses(AF_INET as u32, flags, std::ptr::null(), buf.as_mut_ptr().cast(), &mut size) } {
            0 => break,
            111 => continue, // ERROR_BUFFER_OVERFLOW: [size] says how much
            _ => return Vec::new(),
        }
    }
    let mut found: Vec<(bool, std::net::Ipv4Addr)> = Vec::new();
    let mut at = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
    // SAFETY: walking the list GetAdaptersAddresses wrote, read-only, while [buf] lives.
    unsafe {
        while let Some(a) = at.as_ref() {
            at = a.Next;
            if a.OperStatus != UP || a.IfType == LOOPBACK || windows_adapter_skipped(&wide(a.FriendlyName), &wide(a.Description)) {
                continue;
            }
            let mut u = a.FirstUnicastAddress;
            while let Some(one) = u.as_ref() {
                u = one.Next;
                let sa = one.Address.lpSockaddr;
                if sa.is_null() || (*sa).sa_family != AF_INET {
                    continue;
                }
                let sin = &*(sa as *const SOCKADDR_IN);
                let ip = std::net::Ipv4Addr::from(u32::from_be(sin.sin_addr.S_un.S_addr));
                if !ip.is_link_local() && !ip.is_loopback() && !found.iter().any(|(_, f)| *f == ip) {
                    found.push((ip.is_private(), ip));
                }
            }
        }
    }
    found.sort_by_key(|(private, _)| !private);
    found.into_iter().take(MAX_ADDRESSES).map(|(_, ip)| ip.into()).collect()
}

/// Windows names its virtual adapters in words: Hyper-V's and WSL's "vEthernet (...)", VirtualBox's
/// and VMware's, and VPNs' (their descriptions say TAP, WireGuard, Tailscale, ZeroTier...).
#[cfg_attr(not(windows), allow(dead_code))]
fn windows_adapter_skipped(name: &str, description: &str) -> bool {
    const WORDS: &[&str] = &["vethernet", "hyper-v", "virtualbox", "vmware", "virtual", "tap-", "wireguard", "tailscale", "zerotier", "openvpn", "wintun", "vpn", "bluetooth"];
    let all = format!("{name} {description}").to_lowercase();
    WORDS.iter().any(|w| all.contains(w))
}

pub const MAX_ADDRESSES: usize = 3;
/// Interfaces of containers, VMs and VPNs, by name.
#[cfg(unix)]
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
        // Taken off the screen: a little longer, then no.
        let mut hidden = Invite::new(now).unwrap();
        let late = proof(hidden.secret(), 1_800_000_000, phone, pc);
        assert!(hidden.on_screen());
        hidden.hide(now);
        assert!(!hidden.on_screen());
        assert!(hidden.proves(&json!({ "timestamp": 1_800_000_000, FIELD: late }), phone, pc, now + AFTER_SHOWN / 2));
        assert!(!hidden.proves(&json!({ "timestamp": 1_800_000_000, FIELD: late }), phone, pc, now + AFTER_SHOWN));
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

    #[test]
    fn windows_virtual_adapters_are_skipped() {
        assert!(windows_adapter_skipped("vEthernet (WSL)", "Hyper-V Virtual Ethernet Adapter"));
        assert!(windows_adapter_skipped("Ethernet 3", "TAP-Windows Adapter V9"));
        assert!(windows_adapter_skipped("Tailscale", "Tailscale Tunnel"));
        assert!(!windows_adapter_skipped("Wi-Fi", "Intel(R) Wi-Fi 6 AX201 160MHz"));
        assert!(!windows_adapter_skipped("Ethernet", "Realtek PCIe GbE Family Controller"));
    }

}
