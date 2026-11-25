//! The controller over the USB cable without USB debugging on Windows: Android's accessory mode
//! (AOA), as the Linux app does it. When the phone asks (`vyoma.usb.accessory`), the phones on the
//! cable are switched into accessory mode as "Vyoma / Companion" (the phone's accessory filter opens
//! Vyoma), and the controller's signed frames are read off the accessory's bulk endpoint and passed
//! to the wired listener (127.0.0.1:1766, core `gamepad::serve_wired`) as if adb's tunnel carried them.
//!
//! Windows gives an app no way to talk to a phone in file-transfer mode, nor to one in accessory
//! mode (no driver binds to it). Red Hat's UsbDk (Apache-2.0, signed) does, without replacing the
//! phone's own driver: a device is taken over ("redirected") only while it's used, then handed back.
//! This PC's "Controller over USB" check offers Get driver: UsbDk's pinned installer from GitHub,
//! its SHA-256 checked, opened (Windows asks for an administrator). Its helper DLL (System32) is
//! loaded at run time. Only likely phones are switched: a known Android maker, or a device with a
//! file-transfer (MTP/PTP) or adb interface; taking over a keyboard to ask it would cut it off.
//! UsbDk's last release (1.0.22) is x64 only.
//! Built and tested on CI and under Wine (the descriptors and requests; Wine has no USB): not yet run
//! on a Windows PC with a phone.

use std::ffi::c_void;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0};
use windows_sys::Win32::System::IO::{GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

use crate::win;

/// This PC's check, and its fix (FixCheck: Get driver).
pub const CHECK: &str = "Controller over USB";
/// The installer's option: UsbDk's installer downloaded and run, then exit.
pub const GET_ARG: &str = "--get-usb-driver";
/// UsbDk's release Get driver installs, and its installer's SHA-256 (taken on 2026-11-25).
const USBDK_VERSION: &str = "1.0.22";
const USBDK_URL: &str = "https://github.com/daynix/UsbDk/releases/download/v1.00-22/UsbDk_1.0.22_x64.msi";
const USBDK_SHA256: &str = "91f6f695e1e13c656024e6d3b55620bf08d8835ef05ee0496935ba6bb62466a5";
const FETCH_WAIT: Duration = Duration::from_secs(300);
const INSTALL_WAIT: Duration = Duration::from_secs(15 * 60);

const AOA_VENDOR: u16 = 0x18d1;
const AOA_PRODUCTS: std::ops::RangeInclusive<u16> = 0x2d00..=0x2d05;
/// Manufacturer, model, description, version, URI, serial: what the phone's accessory filter matches.
const STRINGS: [&str; 6] = ["Vyoma", "Companion", "Vyoma's PC companion", "1", "https://github.com/", "vyoma"];
/// Android makers' USB vendor ids: Google, Samsung, Xiaomi, OnePlus, OPPO/realme, vivo, Motorola,
/// LG, Huawei, Honor, Sony, HTC, ASUS, Lenovo, ZTE, Nothing, Fairphone, Meizu, Nokia (HMD), TCL.
const ANDROID_VENDORS: [u16; 20] =
    [0x18d1, 0x04e8, 0x2717, 0x2a70, 0x22d9, 0x2d95, 0x22b8, 0x1004, 0x12d1, 0x339b, 0x0fce, 0x0bb4, 0x0b05, 0x17ef, 0x19d2, 0x2b0e, 0x2ae5, 0x2a45, 0x2e04, 0x1bbb];
/// The wired listener the frames go to (core `gamepad::serve_wired`).
const WIRED_PORT: u16 = 1766;

/// A bridge is running (one phone plays at a time).
static RUNNING: AtomicBool = AtomicBool::new(false);
/// Set when the controller closes: the bridge stops reading (the phone stays an accessory) and ends.
static STOP: AtomicBool = AtomicBool::new(false);

// ---- UsbDk's helper (UsbDkHelper.h, UsbDkData.h)

const MAX_DEVICE_ID_LEN: usize = 200;
const CONTROL: u64 = 0;
const BULK: u64 = 1;
const TRANSFER_SUCCESS: i32 = 1;
const TRANSFER_ASYNC: i32 = 2;

#[repr(C)]
#[derive(Clone, Copy)]
struct DeviceId {
    device: [u16; MAX_DEVICE_ID_LEN],
    instance: [u16; MAX_DEVICE_ID_LEN],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct DeviceInfo {
    id: DeviceId,
    filter: u64,
    port: u64,
    speed: u64,
    /// USB_DEVICE_DESCRIPTOR, packed.
    descriptor: [u8; 18],
}

#[repr(C)]
struct TransferRequest {
    endpoint: u64,
    buffer: *mut c_void,
    length: u64,
    kind: u64,
    iso_count: u64,
    iso_packets: *mut c_void,
    bytes: u64,
    usbd_status: u64,
    iso_results: *mut c_void,
}

#[repr(C)]
struct ConfigRequest {
    id: DeviceId,
    index: u64,
}

const _: () = assert!(size_of::<DeviceInfo>() == 848);
const _: () = assert!(size_of::<TransferRequest>() == 72);

type Bool = i32;

struct Helper {
    list: unsafe extern "C" fn(*mut *mut DeviceInfo, *mut u32) -> Bool,
    release_list: unsafe extern "C" fn(*mut DeviceInfo),
    config: unsafe extern "C" fn(*mut ConfigRequest, *mut *mut u8, *mut u32) -> Bool,
    release_config: unsafe extern "C" fn(*mut u8),
    redirect: unsafe extern "C" fn(*mut DeviceId) -> HANDLE,
    stop_redirect: unsafe extern "C" fn(HANDLE) -> Bool,
    write: unsafe extern "C" fn(HANDLE, *mut TransferRequest, *mut OVERLAPPED) -> i32,
    read: unsafe extern "C" fn(HANDLE, *mut TransferRequest, *mut OVERLAPPED) -> i32,
    abort: unsafe extern "C" fn(HANDLE, u64) -> Bool,
    reset: unsafe extern "C" fn(HANDLE) -> Bool,
    system_handle: unsafe extern "C" fn(HANDLE) -> HANDLE,
}

impl Helper {
    /// UsbDk's helper, if UsbDk is installed.
    fn load() -> Option<Helper> {
        // SAFETY: UsbDk's own DLL by name; each function looked up by its exported name and given
        // the signature UsbDkHelper.h declares. The DLL stays loaded.
        unsafe {
            let lib = LoadLibraryW(win::wide("UsbDkHelper.dll").as_ptr());
            if lib.is_null() {
                return None;
            }
            macro_rules! f {
                ($name:literal) => {
                    std::mem::transmute(GetProcAddress(lib, concat!($name, "\0").as_ptr())?)
                };
            }
            Some(Helper {
                list: f!("UsbDk_GetDevicesList"),
                release_list: f!("UsbDk_ReleaseDevicesList"),
                config: f!("UsbDk_GetConfigurationDescriptor"),
                release_config: f!("UsbDk_ReleaseConfigurationDescriptor"),
                redirect: f!("UsbDk_StartRedirect"),
                stop_redirect: f!("UsbDk_StopRedirect"),
                write: f!("UsbDk_WritePipe"),
                read: f!("UsbDk_ReadPipe"),
                abort: f!("UsbDk_AbortPipe"),
                reset: f!("UsbDk_ResetDevice"),
                system_handle: f!("UsbDk_GetRedirectorSystemHandle"),
            })
        }
    }

    fn devices(&self) -> Vec<Device> {
        let (mut list, mut count) = (std::ptr::null_mut(), 0u32);
        // SAFETY: UsbDk allocates the list; it's copied out and released.
        unsafe {
            if (self.list)(&mut list, &mut count) == 0 || list.is_null() {
                return Vec::new();
            }
            let found = std::slice::from_raw_parts(list, count as usize).iter().map(|d| Device { id: d.id, descriptor: d.descriptor }).collect();
            (self.release_list)(list);
            found
        }
    }

    /// The device's first configuration descriptor (all of it: interfaces, endpoints).
    fn configuration(&self, d: &Device) -> Option<Vec<u8>> {
        let mut request = ConfigRequest { id: d.id, index: 0 };
        let (mut desc, mut len) = (std::ptr::null_mut(), 0u32);
        // SAFETY: UsbDk allocates the descriptor; it's copied out and released.
        unsafe {
            if (self.config)(&mut request, &mut desc, &mut len) == 0 || desc.is_null() {
                return None;
            }
            let bytes = std::slice::from_raw_parts(desc, len as usize).to_vec();
            (self.release_config)(desc);
            Some(bytes)
        }
    }

    /// Takes [d] over from Windows until the returned handle is dropped.
    fn open(&self, d: &Device) -> Result<Redirected<'_>> {
        let mut id = d.id;
        // SAFETY: a device id from UsbDk's own list.
        let handle = unsafe { (self.redirect)(&mut id) };
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            bail!("couldn't take over {:04x}:{:04x}: {}", d.vendor(), d.product(), std::io::Error::last_os_error());
        }
        Ok(Redirected { helper: self, handle })
    }
}

#[derive(Clone, Copy)]
struct Device {
    id: DeviceId,
    descriptor: [u8; 18],
}

impl Device {
    fn class(&self) -> u8 {
        self.descriptor[4]
    }
    fn vendor(&self) -> u16 {
        u16::from_le_bytes([self.descriptor[8], self.descriptor[9]])
    }
    fn product(&self) -> u16 {
        u16::from_le_bytes([self.descriptor[10], self.descriptor[11]])
    }
    fn is_accessory(&self) -> bool {
        self.vendor() == AOA_VENDOR && AOA_PRODUCTS.contains(&self.product())
    }
}

/// A device taken over from Windows; handed back when dropped.
struct Redirected<'a> {
    helper: &'a Helper,
    handle: HANDLE,
}

impl Drop for Redirected<'_> {
    fn drop(&mut self) {
        // SAFETY: our own redirection, ended once.
        unsafe { (self.helper.stop_redirect)(self.handle) };
    }
}

impl Redirected<'_> {
    /// One transfer, waited for up to [timeout] (then aborted): the bytes moved.
    fn transfer(&self, req: &mut TransferRequest, read: bool, timeout: Duration) -> Result<usize> {
        // SAFETY: an event for this transfer, closed below; the request and its buffer outlive the
        // wait (an aborted transfer is waited for before returning).
        unsafe {
            let event = CreateEventW(std::ptr::null(), 1, 0, std::ptr::null());
            if event.is_null() {
                bail!("no event: {}", std::io::Error::last_os_error());
            }
            let mut ov: OVERLAPPED = std::mem::zeroed();
            ov.hEvent = event;
            let started = if read { (self.helper.read)(self.handle, req, &mut ov) } else { (self.helper.write)(self.handle, req, &mut ov) };
            let result = match started {
                TRANSFER_SUCCESS => Ok(req.bytes as usize),
                TRANSFER_ASYNC => {
                    if WaitForSingleObject(event, timeout.as_millis() as u32) != WAIT_OBJECT_0 {
                        (self.helper.abort)(self.handle, req.endpoint);
                        WaitForSingleObject(event, 2000);
                        Err(anyhow::anyhow!("timed out"))
                    } else {
                        let mut n = 0u32;
                        let done = GetOverlappedResult((self.helper.system_handle)(self.handle), &ov, &mut n, 0) != 0;
                        if done && req.usbd_status == 0 { Ok(req.bytes as usize) } else { Err(anyhow::anyhow!("the transfer failed ({:#x})", req.usbd_status)) }
                    }
                }
                _ => Err(anyhow::anyhow!("the transfer failed: {}", std::io::Error::last_os_error())),
            };
            CloseHandle(event);
            result
        }
    }

    /// A control transfer: the setup packet, then [data] out or in.
    fn control(&self, request_type: u8, request: u8, index: u16, data: &mut [u8]) -> Result<usize> {
        let mut buf = setup(request_type, request, 0, index, data);
        let read = request_type & 0x80 != 0;
        let mut req = TransferRequest {
            endpoint: 0,
            buffer: buf.as_mut_ptr().cast(),
            length: buf.len() as u64,
            kind: CONTROL,
            iso_count: 0,
            iso_packets: std::ptr::null_mut(),
            bytes: 0,
            usbd_status: 0,
            iso_results: std::ptr::null_mut(),
        };
        let n = self.transfer(&mut req, read, Duration::from_secs(1))?;
        if read {
            data.copy_from_slice(&buf[8..]);
        }
        Ok(n)
    }

    fn bulk_in(&self, endpoint: u8, buf: &mut [u8], timeout: Duration) -> Result<usize> {
        let mut req = TransferRequest {
            endpoint: endpoint as u64,
            buffer: buf.as_mut_ptr().cast(),
            length: buf.len() as u64,
            kind: BULK,
            iso_count: 0,
            iso_packets: std::ptr::null_mut(),
            bytes: 0,
            usbd_status: 0,
            iso_results: std::ptr::null_mut(),
        };
        self.transfer(&mut req, true, timeout)
    }

    fn reset(&self) {
        // SAFETY: our own redirection.
        unsafe { (self.helper.reset)(self.handle) };
    }
}

/// A control transfer's buffer for UsbDk: the 8-byte setup packet, then the data (out) or room for it (in).
fn setup(request_type: u8, request: u8, value: u16, index: u16, data: &[u8]) -> Vec<u8> {
    let mut buf = vec![request_type, request];
    buf.extend(value.to_le_bytes());
    buf.extend(index.to_le_bytes());
    buf.extend((data.len() as u16).to_le_bytes());
    if request_type & 0x80 == 0 {
        buf.extend_from_slice(data);
    } else {
        buf.resize(8 + data.len(), 0);
    }
    buf
}

/// What a configuration descriptor's interfaces say: whether one is a phone's (file transfer:
/// MTP/PTP, or adb), and interface 0's bulk endpoints (in, out).
#[derive(Debug, Default, PartialEq)]
struct Interfaces {
    phone_like: bool,
    bulk: (Option<u8>, Option<u8>),
}

fn interfaces(config: &[u8]) -> Interfaces {
    let mut out = Interfaces::default();
    let (mut at, mut current) = (0usize, None);
    while at + 2 <= config.len() {
        let len = config[at] as usize;
        if len < 2 || at + len > config.len() {
            break;
        }
        let d = &config[at..at + len];
        match d[1] {
            // Interface: number, alternate setting, class, subclass, protocol.
            4 if len >= 9 => {
                current = Some((d[2], d[3]));
                let (class, sub, proto) = (d[5], d[6], d[7]);
                if class == 0x06 || (class == 0xff && sub == 0xff && proto == 0) || (class == 0xff && sub == 0x42 && proto == 1) {
                    out.phone_like = true;
                }
            }
            // Endpoint: address, attributes (bulk = 2).
            5 if len >= 7 && current == Some((0, 0)) && d[3] & 3 == 2 => {
                if d[2] & 0x80 != 0 {
                    out.bulk.0.get_or_insert(d[2]);
                } else {
                    out.bulk.1.get_or_insert(d[2]);
                }
            }
            _ => {}
        }
        at += len;
    }
    out
}

fn likely_phone(helper: &Helper, d: &Device) -> bool {
    if d.class() == 0x09 || d.is_accessory() {
        return false;
    }
    ANDROID_VENDORS.contains(&d.vendor()) || helper.configuration(d).is_some_and(|c| interfaces(&c).phone_like)
}

/// Switches [d] into accessory mode if it speaks AOA; false if it doesn't.
fn switch(helper: &Helper, d: &Device) -> Result<bool> {
    let dev = helper.open(d)?;
    let mut version = [0u8; 2];
    // GET_PROTOCOL: only Android devices answer it, with 1 or 2.
    if dev.control(0xc0, 51, 0, &mut version).is_err() || u16::from_le_bytes(version) == 0 {
        return Ok(false);
    }
    for (i, s) in STRINGS.iter().enumerate() {
        let mut bytes = s.as_bytes().to_vec();
        bytes.push(0);
        dev.control(0x40, 52, i as u16, &mut bytes)?;
    }
    dev.control(0x40, 53, 0, &mut [])?;
    Ok(true)
}

pub fn installed() -> bool {
    Helper::load().is_some()
}

/// This PC's check.
pub fn check() -> Value {
    if installed() {
        json!({ "name": CHECK, "state": "ok", "detail": "The phone's controller works over the cable without USB debugging", "action": "" })
    } else {
        json!({
            "name": CHECK,
            "state": "warn",
            "detail": "For the controller over the cable without USB debugging, Vyoma needs UsbDk, Red Hat's free USB driver. Get driver downloads it from GitHub and opens its installer",
            "action": "Get driver",
        })
    }
}

/// The phone asked: switch it and bridge its frames. With [reswitch], a phone already in accessory
/// mode is reset and switched again (Android only hands the accessory to an app when it attaches). Blocking.
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

fn wait_for(helper: &Helper, present: bool, within: Duration) -> Option<Device> {
    let deadline = Instant::now() + within;
    loop {
        let found = helper.devices().into_iter().find(Device::is_accessory);
        if found.is_some() == present {
            return found;
        }
        if Instant::now() > deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn run(reswitch: bool) -> Result<()> {
    let helper = Helper::load().context("UsbDk isn't installed: on the PC, open This PC in Vyoma and press Get driver by Controller over USB")?;
    if reswitch && let Some(acc) = helper.devices().into_iter().find(Device::is_accessory) {
        if let Ok(dev) = helper.open(&acc) {
            dev.reset();
        }
        wait_for(&helper, false, Duration::from_secs(5));
        // Back as a phone; give Android a moment before asking it to switch again.
        std::thread::sleep(Duration::from_secs(1));
    }
    if !helper.devices().iter().any(Device::is_accessory) {
        let switched = helper.devices().iter().filter(|d| likely_phone(&helper, d)).filter(|d| matches!(switch(&helper, d), Ok(true))).count();
        if switched == 0 {
            bail!("no phone on the USB cable could be switched to accessory mode");
        }
    }
    let acc = wait_for(&helper, true, Duration::from_secs(5)).context("the phone didn't come back as an accessory")?;
    let (ep_in, _) = helper.configuration(&acc).map(|c| interfaces(&c).bulk).context("the accessory has no configuration")?;
    let ep_in = ep_in.context("the accessory has no bulk endpoint")?;
    let deadline = Instant::now() + Duration::from_secs(3);
    let dev = loop {
        match helper.open(&acc) {
            Ok(d) => break d,
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(e).context("open the accessory"),
        }
    };
    tracing::info!("phone in USB accessory mode: controller over the cable");
    let mut wired: Option<std::net::TcpStream> = None;
    let mut buf = [0u8; 512];
    let result = loop {
        if STOP.load(Ordering::SeqCst) {
            break Ok(());
        }
        let n = match dev.bulk_in(ep_in, &mut buf, Duration::from_millis(500)) {
            Ok(n) => n,
            Err(e) if e.to_string() == "timed out" => continue,
            // Unplugged, or reset.
            Err(e) => break Err(e).context("accessory read"),
        };
        if n == 0 {
            continue;
        }
        // Each phone write is one frame, one transfer; the wired listener checks each frame's tag.
        if wired.is_none() {
            wired = std::net::TcpStream::connect(("127.0.0.1", WIRED_PORT)).ok();
            if let Some(w) = &wired {
                let _ = w.set_nodelay(true);
                drain_echoes(w);
            }
        }
        if let Some(w) = wired.as_mut()
            && w.write_all(&buf[..n]).is_err()
        {
            wired = None;
        }
    };
    match &result {
        Ok(()) => tracing::info!("controller closed: stopped reading the USB accessory"),
        Err(e) => tracing::warn!("accessory bridge ended: {e:#}"),
    }
    result
}

/// The wired listener echoes every frame for the adb route; nothing here wants them, and left
/// unread they'd fill the socket until each echo held up the controller.
fn drain_echoes(wired: &std::net::TcpStream) {
    let Ok(mut echoes) = wired.try_clone() else { return };
    let _ = std::thread::Builder::new().name("vyoma-usb-echoes".into()).spawn(move || {
        let mut buf = [0u8; 256];
        while matches!(echoes.read(&mut buf), Ok(n) if n > 0) {}
    });
}

/// UsbDk's pinned installer downloaded into Vyoma's folder and its SHA-256 checked.
async fn download() -> Result<std::path::PathBuf> {
    let dir = vyoma_core::identity::dir();
    std::fs::create_dir_all(&dir)?;
    let setup = dir.join(format!("UsbDk_{USBDK_VERSION}_x64.msi"));
    let fetched = tokio::time::timeout(FETCH_WAIT, crate::usb::quiet("curl.exe").args(["-fsSL", "--proto", "=https", "-o"]).arg(&setup).arg(USBDK_URL).status())
        .await
        .context("the download took too long")?
        .context("Windows has no curl")?;
    if !fetched.success() {
        let _ = std::fs::remove_file(&setup);
        bail!("Couldn't download UsbDk. Check the internet connection");
    }
    let bytes = tokio::fs::read(&setup).await?;
    if !crate::gamepad::sum_matches(&bytes, USBDK_SHA256) {
        let _ = std::fs::remove_file(&setup);
        bail!("The download wasn't the UsbDk Vyoma expects, so it wasn't opened");
    }
    Ok(setup)
}

/// Get driver: the installer downloaded, checked, opened (it asks for an administrator); then
/// watched for, [on_installed] once UsbDk is in place.
pub async fn fetch_driver(on_installed: impl FnOnce() + Send + 'static) -> Result<()> {
    if installed() {
        on_installed();
        return Ok(());
    }
    let setup = download().await?;
    tracing::info!("opening the UsbDk {USBDK_VERSION} installer");
    win::open(&setup.to_string_lossy());
    tokio::spawn(async move {
        let until = tokio::time::Instant::now() + INSTALL_WAIT;
        while tokio::time::Instant::now() < until {
            tokio::time::sleep(Duration::from_secs(3)).await;
            if tokio::task::spawn_blocking(installed).await.unwrap_or(false) {
                tracing::info!("UsbDk is in place");
                tokio::time::sleep(Duration::from_secs(30)).await;
                let _ = std::fs::remove_file(&setup);
                on_installed();
                return;
            }
        }
    });
    Ok(())
}

/// `--get-usb-driver` (the installer's "Controller over USB"): UsbDk's installer downloaded,
/// checked and run, waited for (Windows Installer asks for an administrator itself).
pub fn get_from_installer() -> Result<()> {
    if installed() {
        return Ok(());
    }
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let setup = rt.block_on(download())?;
    let status = std::process::Command::new("msiexec.exe").arg("/i").arg(&setup).arg("/passive").status();
    let _ = std::fs::remove_file(&setup);
    anyhow::ensure!(status.context("couldn't run Windows Installer")?.success(), "UsbDk wasn't installed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_packets_are_usbs() {
        assert_eq!(setup(0xc0, 51, 0, 0, &[0, 0]), [0xc0, 51, 0, 0, 0, 0, 2, 0, 0, 0]);
        assert_eq!(setup(0x40, 52, 0, 3, b"1\0"), [0x40, 52, 0, 0, 3, 0, 2, 0, b'1', 0]);
        assert_eq!(setup(0x40, 53, 0, 0, &[]), [0x40, 53, 0, 0, 0, 0, 0, 0]);
    }

    fn interface(number: u8, class: u8, sub: u8, proto: u8) -> [u8; 9] {
        [9, 4, number, 0, 2, class, sub, proto, 0]
    }

    fn endpoint(address: u8, attributes: u8) -> [u8; 7] {
        [7, 5, address, attributes, 0, 2, 0]
    }

    #[test]
    fn reads_configurations() {
        // A phone in file transfer: MTP (class 6) and adb.
        let mut phone = vec![9, 2, 0, 0, 2, 1, 0, 0x80, 50];
        phone.extend(interface(0, 0x06, 1, 1));
        phone.extend(endpoint(0x81, 2));
        phone.extend(interface(1, 0xff, 0x42, 1));
        assert!(interfaces(&phone).phone_like);
        // The accessory: interface 0 with bulk in and out (an interrupt endpoint is skipped).
        let mut acc = vec![9, 2, 0, 0, 1, 1, 0, 0x80, 50];
        acc.extend(interface(0, 0xff, 0xff, 0));
        acc.extend(endpoint(0x83, 3));
        acc.extend(endpoint(0x81, 2));
        acc.extend(endpoint(0x02, 2));
        assert_eq!(interfaces(&acc).bulk, (Some(0x81), Some(0x02)));
        // A keyboard isn't one.
        let mut keyboard = vec![9, 2, 0, 0, 1, 1, 0, 0xa0, 50];
        keyboard.extend(interface(0, 0x03, 1, 1));
        keyboard.extend(endpoint(0x81, 3));
        assert_eq!(interfaces(&keyboard), Interfaces::default());
        // Cut short: what's whole is read.
        assert!(interfaces(&phone[..20]).phone_like);
    }

    #[test]
    fn devices_say_what_they_are() {
        let mut descriptor = [0u8; 18];
        descriptor[8..10].copy_from_slice(&0x18d1u16.to_le_bytes());
        descriptor[10..12].copy_from_slice(&0x2d01u16.to_le_bytes());
        let d = Device { id: DeviceId { device: [0; MAX_DEVICE_ID_LEN], instance: [0; MAX_DEVICE_ID_LEN] }, descriptor };
        assert!(d.is_accessory());
        assert_eq!((d.vendor(), d.product()), (0x18d1, 0x2d01));
    }

    /// Wine has no UsbDk: the check offers it, and a request says how to get it.
    #[test]
    fn without_usbdk_the_phone_hears_how() {
        if !installed() {
            assert_eq!(check()["action"], "Get driver");
            let e = start(false).unwrap_err();
            assert!(format!("{e:#}").contains("Get driver"));
        }
    }
}
