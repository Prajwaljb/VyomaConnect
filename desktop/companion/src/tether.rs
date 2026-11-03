//! USB tethering, no USB debugging needed: a phone sharing its connection over the cable is a
//! small network of two, which mDNS doesn't reliably cross (Android doesn't announce on its
//! tethering interface everywhere). Its address there is this PC's gateway on that link, or
//! failing that the one neighbour on it, both read from /proc; main.rs dials it like a phone
//! found over mDNS, and only if its certificate is one kdeconnectd trusts.
//!
//! A phone's Wi-Fi hotspot is the same story over the air (the PC on the phone's hotspot during a
//! power cut, its home Wi-Fi gone): the phone is the gateway of the PC's wireless link. Any Wi-Fi
//! gateway is tried, since a hotspot can't be told from a router; a router just refuses.

use std::net::{IpAddr, Ipv4Addr};

/// Drivers of the USB network a phone's tethering shows up as.
const DRIVERS: [&str; 3] = ["rndis_host", "cdc_ncm", "cdc_ether"];

/// Where a phone might be: over USB tethering, as the gateway of the PC's Wi-Fi (a hotspot, maybe),
/// or through an adb forward over the cable (usb.rs).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Via {
    Usb,
    Hotspot,
    Cable,
}

/// Addresses of phones tethered over USB right now, and Wi-Fi gateways that may be phone hotspots.
pub fn phones() -> Vec<(IpAddr, Via)> {
    let Ok(entries) = std::fs::read_dir("/sys/class/net") else { return Vec::new() };
    let routes = std::fs::read_to_string("/proc/net/route").unwrap_or_default();
    let arp = std::fs::read_to_string("/proc/net/arp").unwrap_or_default();
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.path().join("wireless").exists() {
            if std::fs::read_to_string(entry.path().join("operstate")).is_ok_and(|s| s.trim() == "up")
                && let Some(ip) = gateway(&routes, &name)
            {
                out.push((IpAddr::V4(ip), Via::Hotspot));
            }
            continue;
        }
        let driver = std::fs::read_link(entry.path().join("device/driver")).ok();
        let Some(driver) = driver.as_ref().and_then(|d| d.file_name()).map(|d| d.to_string_lossy().into_owned()) else { continue };
        if !DRIVERS.contains(&driver.as_str()) {
            continue;
        }
        if std::fs::read_to_string(entry.path().join("operstate")).is_ok_and(|s| s.trim() == "down") {
            continue;
        }
        if let Some(ip) = gateway(&routes, &name).or_else(|| neighbour(&arp, &name)) {
            out.push((IpAddr::V4(ip), Via::Usb));
        }
    }
    out
}

/// The default route's gateway through [iface] (the phone hands it out over DHCP).
fn gateway(routes: &str, iface: &str) -> Option<Ipv4Addr> {
    routes.lines().skip(1).find_map(|line| {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 3 || f[0] != iface || f[1] != "00000000" {
            return None;
        }
        // Hex, in the kernel's (little-endian) byte order.
        let g = u32::from_str_radix(f[2], 16).ok().filter(|&g| g != 0)?;
        Some(Ipv4Addr::from(g.to_le_bytes()))
    })
}

/// The one other machine on [iface], when the PC kept another default route.
fn neighbour(arp: &str, iface: &str) -> Option<Ipv4Addr> {
    arp.lines().skip(1).find_map(|line| {
        let f: Vec<&str> = line.split_whitespace().collect();
        // IP, HW type, flags (0x2: complete), HW address, mask, device.
        if f.len() < 6 || f[5] != iface || f[2] == "0x0" {
            return None;
        }
        f[0].parse().ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_gateway_and_neighbour() {
        let routes = "Iface\tDestination\tGateway \tFlags\nwlan0\t00000000\t0100A8C0\t0003\nenp0s1\t00000000\t812AA8C0\t0003\n";
        assert_eq!(gateway(routes, "enp0s1"), Some(Ipv4Addr::new(192, 168, 42, 129)));
        assert_eq!(gateway(routes, "usb0"), None);
        let arp = "IP address       HW type     Flags       HW address            Mask     Device\n10.12.5.7        0x1         0x2         aa:bb:cc:dd:ee:ff     *        usb0\n";
        assert_eq!(neighbour(arp, "usb0"), Some(Ipv4Addr::new(10, 12, 5, 7)));
    }
}
