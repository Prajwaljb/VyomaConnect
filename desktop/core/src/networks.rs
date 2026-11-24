//! The networks this PC is on, for telling phones it's there: each one's broadcast address (a
//! limited broadcast, 255.255.255.255, leaves only by the default route, so a PC on Ethernet that
//! also joined the phone's hotspot by Wi-Fi never reached the phone), and a watch that says when
//! to announce again: a new address (the hotspot, another Wi-Fi, a cable) or a wake from sleep.

use std::collections::HashSet;
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

/// How often the addresses are looked at (a cheap system call).
const EVERY: Duration = Duration::from_secs(4);
/// A look this late means the computer slept in between.
const SLEPT: Duration = Duration::from_secs(30);

/// This PC's IPv4 addresses on reachable networks (not loopback, containers', VMs' or VPNs'),
/// each with its network's broadcast address.
pub fn addresses() -> Vec<(Ipv4Addr, Ipv4Addr)> {
    imp::addresses()
}

/// Each network's broadcast address.
pub fn broadcasts() -> Vec<Ipv4Addr> {
    let mut out: Vec<Ipv4Addr> = Vec::new();
    for (_, b) in addresses() {
        if !b.is_unspecified() && !b.is_broadcast() && !out.contains(&b) {
            out.push(b);
        }
    }
    out
}

#[cfg_attr(not(windows), allow(dead_code))]
fn broadcast_of(ip: Ipv4Addr, prefix: u8) -> Ipv4Addr {
    if prefix == 0 || prefix >= 32 {
        return Ipv4Addr::UNSPECIFIED;
    }
    Ipv4Addr::from(u32::from(ip) | (u32::MAX >> prefix))
}

/// Sends on [out] when this PC should announce itself again: it gained an address, or woke
/// (its looks came far apart). Runs until [out]'s receiver goes.
pub async fn follow(out: tokio::sync::mpsc::UnboundedSender<&'static str>) {
    let look = || async { tokio::task::spawn_blocking(|| addresses().into_iter().map(|(ip, _)| ip).collect::<HashSet<_>>()).await.unwrap_or_default() };
    let mut known = look().await;
    let mut last = Instant::now();
    loop {
        tokio::time::sleep(EVERY).await;
        let now = look().await;
        let woke = last.elapsed() > SLEPT;
        last = Instant::now();
        let gained = now.difference(&known).next().is_some();
        known = now;
        let why = if woke { "awake again" } else if gained { "on a new network" } else { continue };
        if out.send(why).is_err() {
            return;
        }
    }
}

#[cfg(unix)]
mod imp {
    use std::net::Ipv4Addr;

    pub fn addresses() -> Vec<(Ipv4Addr, Ipv4Addr)> {
        let mut out = Vec::new();
        let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
        // SAFETY: getifaddrs fills a list we walk read-only and free once.
        unsafe {
            if libc::getifaddrs(&mut list) != 0 {
                return out;
            }
            let mut at = list;
            while let Some(i) = at.as_ref() {
                at = i.ifa_next;
                if i.ifa_addr.is_null() || i32::from((*i.ifa_addr).sa_family) != libc::AF_INET || i.ifa_flags & libc::IFF_UP as u32 == 0 || i.ifa_flags & libc::IFF_LOOPBACK as u32 != 0 {
                    continue;
                }
                let name = std::ffi::CStr::from_ptr(i.ifa_name).to_string_lossy();
                if crate::invite::SKIP.iter().any(|p| name.starts_with(p)) {
                    continue;
                }
                let ip = Ipv4Addr::from(u32::from_be((*(i.ifa_addr as *const libc::sockaddr_in)).sin_addr.s_addr));
                if ip.is_link_local() {
                    continue;
                }
                #[cfg(any(target_os = "linux", target_os = "android"))]
                let other = i.ifa_ifu;
                #[cfg(not(any(target_os = "linux", target_os = "android")))]
                let other = i.ifa_dstaddr;
                let broadcast = if i.ifa_flags & libc::IFF_BROADCAST as u32 != 0 && !other.is_null() && i32::from((*other).sa_family) == libc::AF_INET {
                    Ipv4Addr::from(u32::from_be((*(other as *const libc::sockaddr_in)).sin_addr.s_addr))
                } else {
                    Ipv4Addr::UNSPECIFIED
                };
                if !out.iter().any(|(known, _)| *known == ip) {
                    out.push((ip, broadcast));
                }
            }
            libc::freeifaddrs(list);
        }
        out
    }
}

#[cfg(windows)]
mod imp {
    use std::net::Ipv4Addr;
    use windows_sys::Win32::NetworkManagement::IpHelper::{GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, GetAdaptersAddresses, IP_ADAPTER_ADDRESSES_LH};
    use windows_sys::Win32::Networking::WinSock::{AF_INET, SOCKADDR_IN};

    pub fn addresses() -> Vec<(Ipv4Addr, Ipv4Addr)> {
        const UP: i32 = 1; // IfOperStatusUp
        const LOOPBACK: u32 = 24; // IF_TYPE_SOFTWARE_LOOPBACK
        let wide = |p: *const u16| {
            // SAFETY: a NUL-terminated UTF-16 string the adapter list owns.
            unsafe { (!p.is_null()).then(|| String::from_utf16_lossy(std::slice::from_raw_parts(p, (0..).take_while(|&i| *p.add(i) != 0).count()))).unwrap_or_default() }
        };
        let mut size: u32 = 16 * 1024;
        let mut buf: Vec<u64> = Vec::new();
        let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
        let mut done = false;
        for _ in 0..3 {
            buf.resize((size as usize).div_ceil(8), 0);
            // SAFETY: the buffer is [size] bytes, aligned for the structures written into it.
            match unsafe { GetAdaptersAddresses(AF_INET as u32, flags, std::ptr::null(), buf.as_mut_ptr().cast(), &mut size) } {
                0 => {
                    done = true;
                    break;
                }
                111 => continue, // ERROR_BUFFER_OVERFLOW: [size] says how much
                _ => return Vec::new(),
            }
        }
        if !done {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut at = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
        // SAFETY: walking the list GetAdaptersAddresses wrote, read-only, while [buf] lives.
        unsafe {
            while let Some(a) = at.as_ref() {
                at = a.Next;
                if a.OperStatus != UP || a.IfType == LOOPBACK || crate::invite::windows_adapter_skipped(&wide(a.FriendlyName), &wide(a.Description)) {
                    continue;
                }
                let mut u = a.FirstUnicastAddress;
                while let Some(one) = u.as_ref() {
                    u = one.Next;
                    let sa = one.Address.lpSockaddr;
                    if sa.is_null() || (*sa).sa_family != AF_INET {
                        continue;
                    }
                    let ip = Ipv4Addr::from(u32::from_be((*(sa as *const SOCKADDR_IN)).sin_addr.S_un.S_addr));
                    if ip.is_link_local() || ip.is_loopback() || out.iter().any(|(known, _)| *known == ip) {
                        continue;
                    }
                    out.push((ip, super::broadcast_of(ip, one.OnLinkPrefixLength)));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn works_out_broadcast_addresses() {
        assert_eq!(broadcast_of(Ipv4Addr::new(192, 168, 43, 17), 24), Ipv4Addr::new(192, 168, 43, 255));
        assert_eq!(broadcast_of(Ipv4Addr::new(10, 7, 3, 9), 20), Ipv4Addr::new(10, 7, 15, 255));
        assert_eq!(broadcast_of(Ipv4Addr::new(10, 0, 0, 1), 0), Ipv4Addr::UNSPECIFIED);
    }

    #[test]
    fn lists_this_computers_networks() {
        for (ip, b) in addresses() {
            assert!(!ip.is_loopback());
            println!("{ip} broadcast {b}");
        }
        assert!(broadcasts().iter().all(|b| !b.is_unspecified() && !b.is_broadcast()));
    }

    #[tokio::test(start_paused = true)]
    async fn says_nothing_while_nothing_changes() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let watch = tokio::spawn(follow(tx));
        tokio::time::sleep(Duration::from_secs(20)).await;
        assert!(rx.try_recv().is_err());
        watch.abort();
    }
}
