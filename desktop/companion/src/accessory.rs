//! The controller over the USB cable without USB debugging: Android's accessory mode (AOA). When the
//! phone asks (`vyoma.usb.accessory`, sent when its controller opens on a cable but adb's wired
//! channel isn't there), every plugged-in Android device that speaks AOA is switched into accessory
//! mode as "Vyoma / Companion" (the phone's accessory filter opens Vyoma for it). The phone then
//! writes the controller's signed frames to the accessory; this reads them off the bulk endpoint and
//! passes them to the wired listener (gamepad.rs), exactly as if they came over the adb reverse.
//!
//! Accessory mode takes the phone's USB over (no file transfer meanwhile) until the cable is pulled.
//! The phone is deliberately not reset back when the controller closes: on the S24 FE a reset while
//! the app held the accessory open made the phone stop answering on USB until it was replugged
//! (2026-11-03). Staying in accessory mode also makes the next controller session start at once.
//! Only a phone that can't open an accessory it already has (its permission went with a reinstall)
//! is reset and switched again, and then no app holds it.
//!
//! Access: the device nodes need to be writable by this user. With USB debugging on, systemd's
//! uaccess rule covers them; without it, 70-vyoma-usb.rules (desktop/companion/udev) does.
//! Raw usbdevfs ioctls, no libusb.

use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use tracing::{info, warn};

const AOA_VENDOR: u16 = 0x18d1;
const AOA_PRODUCTS: std::ops::RangeInclusive<u16> = 0x2d00..=0x2d05;
/// Manufacturer, model, description, version, URI, serial: what the phone's accessory filter matches.
const STRINGS: [&str; 6] = ["Vyoma", "Companion", "Vyoma's PC companion", "1", "https://github.com/", "vyoma"];

const USBDEVFS_CONTROL: libc::c_ulong = 0xc018_5500;
const USBDEVFS_BULK: libc::c_ulong = 0xc018_5502;
const USBDEVFS_CLAIMINTERFACE: libc::c_ulong = 0x8004_550f;
const USBDEVFS_RESET: libc::c_ulong = 0x5514;

#[repr(C)]
struct CtrlTransfer {
    request_type: u8,
    request: u8,
    value: u16,
    index: u16,
    length: u16,
    timeout: u32,
    data: *mut libc::c_void,
}

#[repr(C)]
struct BulkTransfer {
    ep: u32,
    len: u32,
    timeout: u32,
    data: *mut libc::c_void,
}

const _: () = assert!(size_of::<CtrlTransfer>() == 24);
const _: () = assert!(size_of::<BulkTransfer>() == 24);

/// A bridge is running (one at a time: one phone plays at a time).
static RUNNING: AtomicBool = AtomicBool::new(false);
/// Set when the controller closes: the bridge stops reading (the phone stays an accessory) and ends.
static STOP: AtomicBool = AtomicBool::new(false);

struct UsbDevice {
    sys: PathBuf,
    vendor: u16,
    product: u16,
    node: PathBuf,
}

fn read_hex(p: &Path) -> Option<u16> {
    u16::from_str_radix(std::fs::read_to_string(p).ok()?.trim(), 16).ok()
}

fn devices() -> Vec<UsbDevice> {
    let Ok(entries) = std::fs::read_dir("/sys/bus/usb/devices") else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|e| {
            let sys = e.path();
            let vendor = read_hex(&sys.join("idVendor"))?;
            let product = read_hex(&sys.join("idProduct"))?;
            // Hubs never are phones.
            if std::fs::read_to_string(sys.join("bDeviceClass")).is_ok_and(|c| c.trim() == "09") {
                return None;
            }
            let bus: u32 = std::fs::read_to_string(sys.join("busnum")).ok()?.trim().parse().ok()?;
            let dev: u32 = std::fs::read_to_string(sys.join("devnum")).ok()?.trim().parse().ok()?;
            Some(UsbDevice { sys, vendor, product, node: PathBuf::from(format!("/dev/bus/usb/{bus:03}/{dev:03}")) })
        })
        .collect()
}

fn is_accessory(d: &UsbDevice) -> bool {
    d.vendor == AOA_VENDOR && AOA_PRODUCTS.contains(&d.product)
}

fn open(node: &Path) -> Result<OwnedFd> {
    let path = std::ffi::CString::new(node.as_os_str().as_encoded_bytes()).unwrap();
    let raw = unsafe { libc::open(path.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
    if raw < 0 {
        bail!("open {}: {}", node.display(), std::io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

fn control(fd: &OwnedFd, request_type: u8, request: u8, index: u16, data: &mut [u8]) -> Result<usize> {
    let mut c = CtrlTransfer {
        request_type,
        request,
        value: 0,
        index,
        length: data.len() as u16,
        timeout: 1000,
        data: if data.is_empty() { std::ptr::null_mut() } else { data.as_mut_ptr().cast() },
    };
    let n = unsafe { libc::ioctl(fd.as_raw_fd(), USBDEVFS_CONTROL, &mut c) };
    if n < 0 {
        bail!("usb control {request}: {}", std::io::Error::last_os_error());
    }
    Ok(n as usize)
}

/// Switches [d] into accessory mode if it speaks AOA (an Android device); false if it doesn't.
fn switch(d: &UsbDevice) -> Result<bool> {
    let fd = open(&d.node)?;
    let mut version = [0u8; 2];
    // GET_PROTOCOL: only Android devices answer it, with 1 or 2.
    if control(&fd, 0xc0, 51, 0, &mut version).is_err() || u16::from_le_bytes(version) == 0 {
        return Ok(false);
    }
    for (i, s) in STRINGS.iter().enumerate() {
        let mut bytes = s.as_bytes().to_vec();
        bytes.push(0);
        control(&fd, 0x40, 52, i as u16, &mut bytes)?;
    }
    control(&fd, 0x40, 53, 0, &mut [])?;
    Ok(true)
}

/// Interface 0's bulk endpoints (in, out), from sysfs.
fn bulk_endpoints(d: &UsbDevice) -> Option<(u8, u8)> {
    let name = d.sys.file_name()?.to_string_lossy().into_owned();
    let iface = d.sys.join(format!("{name}:1.0"));
    let (mut input, mut output) = (None, None);
    for e in std::fs::read_dir(&iface).ok()?.flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        let Some(hex) = n.strip_prefix("ep_") else { continue };
        if std::fs::read_to_string(e.path().join("type")).ok()?.trim() != "Bulk" {
            continue;
        }
        let addr = u8::from_str_radix(hex, 16).ok()?;
        if addr & 0x80 != 0 { input = Some(addr) } else { output = Some(addr) }
    }
    Some((input?, output?))
}

/// The phone asked: switch it (and any other AOA device, rare) and bridge its frames. With [reswitch],
/// a phone already in accessory mode is reset and switched again (Android only hands the accessory to
/// an app when it attaches). Blocking.
pub fn start(reswitch: bool) -> Result<()> {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    STOP.store(false, Ordering::SeqCst);
    let r = run(reswitch);
    RUNNING.store(false, Ordering::SeqCst);
    r
}

/// The controller closed: stop bridging.
pub fn release() {
    if RUNNING.load(Ordering::SeqCst) {
        STOP.store(true, Ordering::SeqCst);
    }
}

fn reset(d: &UsbDevice) {
    if let Ok(fd) = open(&d.node) {
        unsafe { libc::ioctl(fd.as_raw_fd(), USBDEVFS_RESET, 0) };
    }
}

fn run(reswitch: bool) -> Result<()> {
    if reswitch && devices().iter().any(is_accessory) {
        devices().iter().filter(|d| is_accessory(d)).for_each(reset);
        let deadline = Instant::now() + Duration::from_secs(5);
        while devices().iter().any(is_accessory) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(200));
        }
        // Back as a phone; give Android a moment before asking it to switch again.
        std::thread::sleep(Duration::from_millis(1000));
    }
    if !devices().iter().any(is_accessory) {
        let mut switched = 0;
        for d in devices().iter().filter(|d| !is_accessory(d)) {
            match switch(d) {
                Ok(true) => switched += 1,
                Ok(false) => {}
                // Not ours to open (no access): most USB devices on the PC.
                Err(_) => {}
            }
        }
        if switched == 0 {
            bail!("no phone on the USB cable could be switched to accessory mode (allowed by the udev rule?)");
        }
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    let acc = loop {
        if let Some(d) = devices().into_iter().find(is_accessory) {
            break d;
        }
        if Instant::now() > deadline {
            bail!("the phone didn't come back as an accessory");
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    // Its interfaces show up in sysfs a moment after the device.
    let mut endpoints = None;
    for _ in 0..25 {
        endpoints = bulk_endpoints(&acc);
        if endpoints.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let (ep_in, _) = endpoints.context("accessory has no bulk endpoints")?;
    // udev grants this user access a moment after the node appears (measured: the first open failed).
    let deadline = Instant::now() + Duration::from_secs(3);
    let fd = loop {
        match open(&acc.node) {
            Ok(fd) => break fd,
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(e).context("open the accessory (needs the udev rule without USB debugging)"),
        }
    };
    let mut iface: libc::c_uint = 0;
    if unsafe { libc::ioctl(fd.as_raw_fd(), USBDEVFS_CLAIMINTERFACE, &mut iface) } < 0 {
        bail!("claim accessory interface: {}", std::io::Error::last_os_error());
    }
    info!("phone in USB accessory mode: controller over the cable");
    let mut wired: Option<std::net::TcpStream> = None;
    let mut buf = [0u8; 512];
    let result = loop {
        if STOP.load(Ordering::SeqCst) {
            break Ok(());
        }
        let mut b = BulkTransfer { ep: ep_in as u32, len: buf.len() as u32, timeout: 500, data: buf.as_mut_ptr().cast() };
        let n = unsafe { libc::ioctl(fd.as_raw_fd(), USBDEVFS_BULK, &mut b) };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::ETIMEDOUT) {
                continue;
            }
            // Unplugged (ENODEV) or reset.
            break Err(e).context("accessory read");
        }
        if n == 0 {
            continue;
        }
        // Each phone write is one frame, one transfer; the wired listener checks each frame's tag.
        if wired.is_none() {
            wired = std::net::TcpStream::connect(("127.0.0.1", 1766)).ok();
            if let Some(w) = &wired {
                let _ = w.set_nodelay(true);
            }
        }
        if let Some(w) = wired.as_mut()
            && w.write_all(&buf[..n as usize]).is_err()
        {
            wired = None;
        }
    };
    match &result {
        Ok(()) => info!("controller closed: stopped reading the USB accessory"),
        Err(e) => warn!("accessory bridge ended: {e:#}"),
    }
    result
}
