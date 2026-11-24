//! New network addresses on this PC, from the kernel (netlink's IPv4 address group): joining a
//! network (the phone's hotspot, another Wi-Fi, a cable) is when phones there need to hear from
//! this PC. They only looked for it every few minutes before (15 with their screen off).

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use anyhow::{Result, bail};

/// RTMGRP_IPV4_IFADDR, RTM_NEWADDR (linux/rtnetlink.h).
const GROUP_IPV4_ADDRESSES: u32 = 0x10;
const NEW_ADDRESS: u16 = 20;
/// Addresses come in bursts (DHCP, then IPv6's, a second interface): one word after they settle.
const SETTLE: std::time::Duration = std::time::Duration::from_secs(2);

/// Calls [gained] once each burst of new IPv4 addresses has settled, for as long as the
/// receiver lives. Returns at once; the reading happens on its own thread.
pub fn watch(gained: tokio::sync::mpsc::UnboundedSender<()>) -> Result<()> {
    let socket = open()?;
    let (raw_tx, mut raw_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    std::thread::Builder::new().name("vyoma-addresses".into()).spawn(move || {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            // SAFETY: reading into our own buffer from a socket we own.
            let n = unsafe { libc::recv(socket.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len(), 0) };
            if n < 0 {
                if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                tracing::warn!("stopped following this PC's network addresses: {}", std::io::Error::last_os_error());
                return;
            }
            if new_address(&buf[..n as usize]) && raw_tx.send(()).is_err() {
                return;
            }
        }
    })?;
    tokio::spawn(async move {
        while raw_rx.recv().await.is_some() {
            // The rest of the burst, then one call.
            while tokio::time::timeout(SETTLE, raw_rx.recv()).await.is_ok_and(|m| m.is_some()) {}
            if gained.send(()).is_err() {
                return;
            }
        }
    });
    Ok(())
}

fn open() -> Result<OwnedFd> {
    // SAFETY: plain socket calls; the descriptor is owned right after.
    unsafe {
        let fd = libc::socket(libc::AF_NETLINK, libc::SOCK_RAW | libc::SOCK_CLOEXEC, libc::NETLINK_ROUTE);
        if fd < 0 {
            bail!("no netlink socket: {}", std::io::Error::last_os_error());
        }
        let socket = OwnedFd::from_raw_fd(fd);
        let mut address: libc::sockaddr_nl = std::mem::zeroed();
        address.nl_family = libc::AF_NETLINK as libc::sa_family_t;
        address.nl_groups = GROUP_IPV4_ADDRESSES;
        if libc::bind(fd, (&address as *const libc::sockaddr_nl).cast(), std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t) < 0 {
            bail!("can't follow network addresses: {}", std::io::Error::last_os_error());
        }
        Ok(socket)
    }
}

/// Whether a netlink datagram holds an RTM_NEWADDR message (each: u32 length, u16 type, ...).
fn new_address(mut data: &[u8]) -> bool {
    while data.len() >= 16 {
        let length = u32::from_ne_bytes([data[0], data[1], data[2], data[3]]) as usize;
        let kind = u16::from_ne_bytes([data[4], data[5]]);
        if kind == NEW_ADDRESS {
            return true;
        }
        // Messages are 4-byte aligned; a bad length ends the walk.
        let step = (length + 3) & !3;
        if length < 16 || step > data.len() {
            return false;
        }
        data = &data[step..];
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(kind: u16) -> Vec<u8> {
        let mut m = vec![0u8; 20];
        m[..4].copy_from_slice(&20u32.to_ne_bytes());
        m[4..6].copy_from_slice(&kind.to_ne_bytes());
        m
    }

    #[test]
    fn finds_new_addresses_among_messages() {
        assert!(new_address(&message(NEW_ADDRESS)));
        assert!(!new_address(&message(21))); // RTM_DELADDR
        assert!(new_address(&[message(21), message(NEW_ADDRESS)].concat()));
        assert!(!new_address(&[0u8; 8]));
    }

    #[test]
    fn opens_the_kernel_socket() {
        open().expect("netlink");
    }
}
