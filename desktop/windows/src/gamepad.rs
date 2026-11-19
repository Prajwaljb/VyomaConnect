//! The phone as a game controller on Windows: an Xbox 360 pad or a DualSense made in software
//! (core's [xbox360] and [dualsense], as the phone asks) that Windows plugs in through usbip-win2, a
//! free (BSD), signed USB/IP driver. Windows' own drivers then bind to it as to a real pad: the Xbox
//! pad reaches every XInput game, Steam and SDL; the DualSense games and libraries that know one
//! (SDL, Steam, PlayStation ports), with its button names. A game's rumble comes back to the phone. The engine's [drive] runs it (the states, the fast
//! routes, the gyro mouse as relative pointer moves here).
//!
//! Each pad serves its device on its own port on 127.0.0.1 (1767-1770, else any) and asks the
//! driver (its device interface, open to every user: no administrator) to attach `127.0.0.1:<port>`
//! bus id 1-1; the driver connects, imports it and plugs it in. When the pad ends it's unplugged
//! first (else the driver would keep trying to reattach), then its connection closes.
//!
//! Without the driver the phone hears why, and This PC's Game controller check offers Get driver:
//! the pinned usbip-win2 release from GitHub, its SHA-256 checked, its installer opened (Windows
//! asks for an administrator).
//! Built and tested on CI and under Wine only (Wine has no USB/IP driver); not yet run on a real Windows PC.

use std::ffi::c_void;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, watch};
use vyoma_core::gamepad::{Kind, State};
use vyoma_core::link::Sender;
use vyoma_core::usbip;
use vyoma_core::dualsense::{self, DualSense};
use vyoma_core::usbip::Gadget;
use vyoma_core::xbox360::{self, Xbox360};
use vyoma_engine::GamepadInput;
use vyoma_engine::gamepad::{GyroMouse, Pads, VirtualPad, drive};
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_INVALID_FUNCTION, ERROR_INVALID_USER_BUFFER, GENERIC_READ, GENERIC_WRITE, GetLastError, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::core::GUID;

use crate::input::{Act, Pointer};
use crate::win;

/// usbip-win2's host controller interface (GUID_DEVINTERFACE_USBIP_VHCI).
const VHCI: GUID = GUID::from_u128(0xB4030C06_DC5F_4FCC_87EB_E5515A0935C0);
/// The ports pads are served on, so ones left behind by a crash can be told to stop retrying.
const PORTS: std::ops::RangeInclusive<u16> = 1767..=1770;
const HOST: &str = "127.0.0.1";
const BUSID: &str = "1-1";
/// How long the driver has to connect and import the pad.
const IMPORT_WAIT: Duration = Duration::from_secs(10);

/// This PC's check: the window's This PC page, and its fix (FixCheck: Get driver).
pub const CHECK: &str = "Game controller";
/// The phone's reason when the driver is missing (it shows plain sentences as they are).
const NO_DRIVER: &str = "This PC needs a free driver for the controller. In Vyoma on the PC, open This PC and press Get driver";

/// The usbip-win2 release Get driver installs, and each installer's SHA-256 (GitHub's own digest).
const DRIVER_VERSION: &str = "0.9.8.1";
const DRIVER_X64: (&str, &str) = ("x64", "38cad6d4432b52d5bb9409d9ad03b72fdffc4ada4cd3a48fbeca1a2752a8518a");
const DRIVER_ARM64: (&str, &str) = ("arm64", "cab7ff97f79275eeb5c5b8bb2eb3111ff6b6c4eead07d9bec9be5bd1e3a35800");
const FETCH_WAIT: Duration = Duration::from_secs(300);
/// How long after opening the installer to watch for the driver.
const INSTALL_WAIT: Duration = Duration::from_secs(15 * 60);

// The driver's IOCTLs: CTL_CODE(FILE_DEVICE_UNKNOWN, function, METHOD_BUFFERED, FILE_READ_DATA | FILE_WRITE_DATA).
const fn ioctl(function: u32) -> u32 {
    (0x22 << 16) | (3 << 14) | (function << 2)
}
const PLUGIN_HARDWARE: u32 = ioctl(0x800);
const PLUGOUT_HARDWARE: u32 = ioctl(0x801);
const STOP_ATTACH_ATTEMPTS: u32 = ioctl(0x805);
const PLUGIN_HARDWARE_ONCE: u32 = ioctl(0x806);

/// imported_device_location: bus id, service (the port), host.
const LOCATION_LEN: usize = 32 + 32 + 1025;

/// plugin_hardware's layouts, newest first: (where the location starts, the struct's size). Since
/// 0.9.8.1: size, port, location hash, location, serial[16], wsk events (bool); before, without the
/// hash; before that without wsk events; at first the location alone. A driver that doesn't know a
/// layout says ERROR_INVALID_USER_BUFFER, and the next is tried.
const LAYOUTS: [(usize, usize); 4] = [(12, 1120), (8, 1116), (8, 1116), (8, 1100)];

fn location(port: u16) -> [u8; LOCATION_LEN] {
    let mut l = [0u8; LOCATION_LEN];
    l[..BUSID.len()].copy_from_slice(BUSID.as_bytes());
    let service = port.to_string();
    l[32..32 + service.len()].copy_from_slice(service.as_bytes());
    l[64..64 + HOST.len()].copy_from_slice(HOST.as_bytes());
    l
}

/// The request to attach the pad served on [port], in [layout].
fn plugin_request((at, size): (usize, usize), port: u16) -> Vec<u8> {
    let mut r = vec![0u8; size];
    r[..4].copy_from_slice(&(size as u32).to_le_bytes());
    r[at..at + LOCATION_LEN].copy_from_slice(&location(port));
    r
}

/// The driver's control device, while open.
struct Vhci(HANDLE);

// SAFETY: a file handle, usable from any thread.
unsafe impl Send for Vhci {}

impl Vhci {
    fn open() -> Result<Vhci> {
        let path = win::device_interfaces(&VHCI).into_iter().next().ok_or_else(|| anyhow!(NO_DRIVER))?;
        // SAFETY: a NUL-terminated path; the handle is closed on drop.
        let h = unsafe { CreateFileW(win::wide(&path).as_ptr(), GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE, std::ptr::null(), OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, std::ptr::null_mut()) };
        if h == INVALID_HANDLE_VALUE {
            bail!("couldn't open the USB/IP driver: {}", std::io::Error::last_os_error());
        }
        Ok(Vhci(h))
    }

    /// One IOCTL with [buf] in and out; the bytes written, or Windows' error code.
    fn call(&self, code: u32, buf: &mut [u8], out: usize) -> std::result::Result<usize, u32> {
        let mut written = 0u32;
        // SAFETY: [buf] is valid for its length both ways; a synchronous call.
        let ok = unsafe { DeviceIoControl(self.0, code, buf.as_ptr() as *const c_void, buf.len() as u32, buf.as_mut_ptr() as *mut c_void, out as u32, &mut written, std::ptr::null_mut()) };
        // SAFETY: read right after the call that failed.
        if ok == 0 { Err(unsafe { GetLastError() }) } else { Ok(written as usize) }
    }

    /// Attaches the pad served on [port]: the driver connects, imports it and plugs it in. Its port
    /// on the driver's hub.
    fn attach(&self, port: u16) -> Result<i32> {
        for layout in LAYOUTS {
            for code in [PLUGIN_HARDWARE_ONCE, PLUGIN_HARDWARE] {
                let mut r = plugin_request(layout, port);
                match self.call(code, &mut r, 8) {
                    Ok(_) => {
                        let hub = i32::from_le_bytes(r[4..8].try_into().unwrap());
                        if hub <= 0 {
                            bail!("the USB/IP driver gave port {hub}");
                        }
                        return Ok(hub);
                    }
                    // An older driver: without one-attempt attaching, or with an older layout.
                    Err(ERROR_INVALID_FUNCTION) if code == PLUGIN_HARDWARE_ONCE => continue,
                    Err(ERROR_INVALID_USER_BUFFER) => break,
                    Err(e) => bail!("the USB/IP driver couldn't attach the controller: {}", std::io::Error::from_raw_os_error(e as i32)),
                }
            }
        }
        bail!("this USB/IP driver is too old or too new for Vyoma: install usbip-win2 {DRIVER_VERSION}")
    }

    /// Unplugs the device on hub port [hub] (without reattaching it).
    fn detach(&self, hub: i32) {
        let mut r = [0u8; 8];
        r[..4].copy_from_slice(&8u32.to_le_bytes());
        r[4..].copy_from_slice(&hub.to_le_bytes());
        let _ = self.call(PLUGOUT_HARDWARE, &mut r, 0);
    }

    /// Stops the driver retrying a pad on [port] (one left by a crash, or the connection closing).
    fn stop_attempts(&self, port: u16) {
        let size = 8 + LOCATION_LEN + 3; // size, count, location; padded to 4
        let mut r = vec![0u8; size];
        r[..4].copy_from_slice(&(size as u32).to_le_bytes());
        r[8..8 + LOCATION_LEN].copy_from_slice(&location(port));
        let _ = self.call(STOP_ATTACH_ATTEMPTS, &mut r, size);
    }
}

impl Drop for Vhci {
    fn drop(&mut self) {
        // SAFETY: our handle, closed once.
        unsafe { CloseHandle(self.0) };
    }
}

/// At start: the driver stops retrying pads a previous run left plugged in (it quit or crashed).
pub fn tidy() {
    if let Ok(vhci) = Vhci::open() {
        for port in PORTS {
            vhci.stop_attempts(port);
        }
    }
}

/// Whether usbip-win2's driver is installed (its interface is there).
pub fn installed() -> bool {
    !win::device_interfaces(&VHCI).is_empty()
}

/// A pad plugged in through the driver; dropping it unplugs it.
pub struct Pad {
    asked: Kind,
    hub: i32,
    port: u16,
    /// The phone's newest state, which [feed] turns into the pad's reports.
    states: watch::Sender<State>,
    rumbles: mpsc::UnboundedReceiver<(u16, u16, u16)>,
    served: tokio::task::AbortHandle,
    fed: tokio::task::AbortHandle,
}

impl Pad {
    pub async fn create(asked: Kind) -> Result<Pad> {
        let vhci = Vhci::open()?;
        let listener = bind().await?;
        let port = listener.local_addr()?.port();
        vhci.stop_attempts(port);
        let (states, state_rx) = watch::channel(State { kind: asked, ..Default::default() });
        let first = match asked {
            Kind::Xbox => xbox360::report(&State::default()),
            Kind::DualSense => dualsense::Reports::default().report(&State::default()),
        };
        let (reports_tx, rx) = watch::channel(first);
        let (rumble_tx, rumbles) = mpsc::unbounded_channel();
        let gadget: Box<dyn Gadget> = match asked {
            Kind::Xbox => Box::new(Xbox360::new(rumble_tx)),
            Kind::DualSense => Box::new(DualSense::new(rumble_tx, rx.clone())),
        };
        let fed = tokio::spawn(feed(asked, state_rx, reports_tx)).abort_handle();
        let (imported_tx, imported) = tokio::sync::oneshot::channel();
        let served = serve_pad(listener, gadget, rx, imported_tx).abort_handle();
        let attached = tokio::task::spawn_blocking(move || vhci.attach(port).map(|hub| (vhci, hub))).await?;
        let (vhci, hub) = match attached {
            Ok(a) => a,
            Err(e) => {
                served.abort();
                fed.abort();
                return Err(e);
            }
        };
        if tokio::time::timeout(IMPORT_WAIT, imported).await.is_err() {
            vhci.detach(hub);
            served.abort();
            fed.abort();
            bail!("the USB/IP driver attached something else");
        }
        tracing::info!("controller plugged in ({asked:?}, USB/IP port {hub})");
        Ok(Pad { asked, hub, port, states, rumbles, served, fed })
    }
}

/// Turns the phone's states into the pad's reports: the Xbox pad's as they change, the DualSense's
/// streamed as the real pad does (each counted and timed, the newest state in each).
async fn feed(kind: Kind, mut states: watch::Receiver<State>, reports: watch::Sender<Vec<u8>>) {
    match kind {
        Kind::Xbox => loop {
            let report = xbox360::report(&states.borrow_and_update());
            reports.send_if_modified(|r| std::mem::replace(r, report.clone()) != report);
            if states.changed().await.is_err() {
                return;
            }
        },
        Kind::DualSense => {
            let mut builder = dualsense::Reports::default();
            let mut every = tokio::time::interval(dualsense::REPORT_EVERY);
            every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                every.tick().await;
                let report = builder.report(&states.borrow());
                if reports.send(report).is_err() {
                    return;
                }
            }
        }
    }
}

/// Serves the pad to the first connection that imports it (the driver), until it lets go.
fn serve_pad(listener: TcpListener, mut pad: Box<dyn Gadget>, reports: watch::Receiver<Vec<u8>>, imported: tokio::sync::oneshot::Sender<()>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, from)) = listener.accept().await else { return };
            if !from.ip().is_loopback() {
                continue;
            }
            let _ = stream.set_nodelay(true);
            match tokio::time::timeout(IMPORT_WAIT, usbip::handshake(&mut stream, BUSID, pad.as_ref())).await {
                Ok(Ok(true)) => {
                    drop(listener);
                    let _ = imported.send(());
                    if let Err(e) = usbip::serve(stream, pad.as_mut(), reports).await {
                        tracing::warn!("the controller's USB/IP connection failed: {e:#}");
                    }
                    return;
                }
                // A device list, or something else knocking: keep waiting for the driver.
                Ok(Ok(false)) => {}
                Ok(Err(e)) => tracing::info!("not the USB/IP driver: {e:#}"),
                Err(_) => {}
            }
        }
    })
}

/// A listener on one of [PORTS], else any port, on loopback only.
async fn bind() -> Result<TcpListener> {
    for port in PORTS {
        if let Ok(l) = TcpListener::bind((HOST, port)).await {
            return Ok(l);
        }
    }
    TcpListener::bind((HOST, 0)).await.context("no port for the controller")
}

impl VirtualPad for Pad {
    fn kind(&self) -> Kind {
        self.asked
    }

    fn apply(&mut self, s: State) -> Result<()> {
        self.states.send_replace(s);
        Ok(())
    }

    async fn rumble(&mut self) -> Result<(u16, u16, u16)> {
        self.rumbles.recv().await.ok_or_else(|| anyhow!("Windows unplugged the controller"))
    }
}

impl Drop for Pad {
    fn drop(&mut self) {
        self.fed.abort();
        let (hub, port, served) = (self.hub, self.port, self.served.clone());
        // Unplugged before its connection closes (a dropped connection is reattached); off the
        // async threads, as the driver waits for Windows to let go of the device.
        let _ = std::thread::Builder::new().name("vyoma-pad-unplug".into()).spawn(move || {
            if let Ok(vhci) = Vhci::open() {
                vhci.detach(hub);
                vhci.stop_attempts(port);
            }
            served.abort();
        });
    }
}

/// The gyro's mouse: relative moves of the pointer.
pub struct Mouse(Pointer);

impl GyroMouse for Mouse {
    fn moved(&mut self, dx: i32, dy: i32) -> Result<()> {
        self.0.handle(&[Act::Move(dx as f64, dy as f64)])
    }
}

struct UsbIp;

impl Pads for UsbIp {
    type Pad = Pad;
    type Mouse = Mouse;

    async fn pad(&mut self, kind: Kind) -> Result<Pad> {
        Pad::create(kind).await
    }

    fn mouse(&mut self) -> Result<Mouse> {
        Ok(Mouse(Pointer::default()))
    }
}

/// Runs the pad for one phone while it sends states.
pub async fn run(input: mpsc::Receiver<GamepadInput>, sender: Sender, name: String) {
    drive(UsbIp, input, sender, name).await
}

/// The Game controller check for This PC.
pub fn check() -> Value {
    if installed() {
        json!({ "name": CHECK, "state": "ok", "detail": "Games see the phone as an Xbox controller", "action": "" })
    } else {
        json!({
            "name": CHECK,
            "state": "warn",
            "detail": "The phone as a game controller needs usbip-win2, a free signed driver. Get driver downloads it from GitHub and opens its installer",
            "action": "Get driver",
        })
    }
}

/// This PC's own processor family, as Windows sees it (an x64 Vyoma runs on ARM64 Windows too).
fn arm64() -> bool {
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, IsWow64Process2};
    const IMAGE_FILE_MACHINE_ARM64: u16 = 0xAA64;
    let (mut process, mut native) = (0u16, 0u16);
    // SAFETY: two out-params for this process.
    unsafe { IsWow64Process2(GetCurrentProcess(), &mut process, &mut native) != 0 && native == IMAGE_FILE_MACHINE_ARM64 }
}

/// Get driver: the pinned installer into Vyoma's folder, checked, opened; then watched for.
pub async fn fetch_driver(on_installed: impl FnOnce() + Send + 'static) -> Result<()> {
    if installed() {
        on_installed();
        return Ok(());
    }
    let (arch, sum) = if arm64() { DRIVER_ARM64 } else { DRIVER_X64 };
    let url = format!("https://github.com/vadimgrn/usbip-win2/releases/download/v.{DRIVER_VERSION}/USBip-{DRIVER_VERSION}-{arch}.exe");
    let dir = vyoma_core::identity::dir();
    std::fs::create_dir_all(&dir)?;
    let setup = dir.join(format!("USBip-{DRIVER_VERSION}-{arch}.exe"));
    let fetched = tokio::time::timeout(FETCH_WAIT, crate::usb::quiet("curl.exe").args(["-fsSL", "--proto", "=https", "-o"]).arg(&setup).arg(&url).status())
        .await
        .context("the download took too long")?
        .context("Windows has no curl")?;
    if !fetched.success() {
        let _ = std::fs::remove_file(&setup);
        bail!("Couldn't download the driver. Check the internet connection");
    }
    let bytes = tokio::fs::read(&setup).await?;
    if !sum_matches(&bytes, sum) {
        let _ = std::fs::remove_file(&setup);
        bail!("The download wasn't the driver Vyoma expects, so it wasn't opened");
    }
    tracing::info!("opening the usbip-win2 {DRIVER_VERSION} installer");
    win::open(&setup.to_string_lossy());
    tokio::spawn(async move {
        let until = tokio::time::Instant::now() + INSTALL_WAIT;
        while tokio::time::Instant::now() < until {
            tokio::time::sleep(Duration::from_secs(3)).await;
            if tokio::task::spawn_blocking(installed).await.unwrap_or(false) {
                tracing::info!("the controller driver is in place");
                // The installer may still be closing; it's only needed once.
                tokio::time::sleep(Duration::from_secs(30)).await;
                let _ = std::fs::remove_file(&setup);
                on_installed();
                return;
            }
        }
    });
    Ok(())
}

fn sum_matches(bytes: &[u8], hex: &str) -> bool {
    let got: String = ring::digest::digest(&ring::digest::SHA256, bytes).as_ref().iter().map(|b| format!("{b:02x}")).collect();
    got == hex
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_have_the_drivers_layouts() {
        // sizeof(plugin_hardware) on Windows for each release's layout.
        assert_eq!(LAYOUTS.map(|l| l.1), [1120, 1116, 1116, 1100]);
        let r = plugin_request(LAYOUTS[0], 1767);
        assert_eq!(&r[..4], 1120u32.to_le_bytes());
        assert_eq!(&r[12..15], b"1-1");
        assert_eq!(&r[12 + 32..12 + 36], b"1767");
        assert_eq!(&r[12 + 64..12 + 73], b"127.0.0.1");
        assert!(r[12 + LOCATION_LEN..].iter().all(|&b| b == 0), "no serial, zero-copy receiving");
        let old = plugin_request(LAYOUTS[3], 1768);
        assert_eq!((&old[8..11], &old[8 + 32..8 + 36]), (&b"1-1"[..], &b"1768"[..]));
        assert_eq!(PLUGIN_HARDWARE, 0x0022_e000);
        assert_eq!(PLUGIN_HARDWARE_ONCE, 0x0022_e018);
    }

    #[test]
    fn checks_the_download() {
        assert!(sum_matches(b"abc", "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"));
        assert!(!sum_matches(b"abd", "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"));
    }

    /// The driver's side of a pad, played here: it lists, imports, reads reports, rumbles.
    #[tokio::test]
    async fn serves_the_pad_to_the_driver() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = bind().await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (reports, rx) = watch::channel(xbox360::report(&State::default()));
        let (rumble_tx, mut rumbles) = mpsc::unbounded_channel();
        let (imported_tx, imported) = tokio::sync::oneshot::channel();
        serve_pad(listener, Box::new(Xbox360::new(rumble_tx)), rx, imported_tx);
        let op = |code: u16| [&0x0111u16.to_be_bytes()[..], &code.to_be_bytes(), &[0; 4]].concat();
        // A device list on its own connection, as usbip.exe asks.
        let mut s = tokio::net::TcpStream::connect((HOST, port)).await.unwrap();
        s.write_all(&op(0x8005)).await.unwrap();
        let mut list = vec![0u8; 8 + 4 + 312];
        s.read_exact(&mut list).await.unwrap();
        assert_eq!(&list[12 + 256..12 + 259], b"1-1");
        // The import.
        let mut s = tokio::net::TcpStream::connect((HOST, port)).await.unwrap();
        let mut busid = [0u8; 32];
        busid[..3].copy_from_slice(b"1-1");
        s.write_all(&[op(0x8003), busid.to_vec()].concat()).await.unwrap();
        let mut reply = vec![0u8; 8 + 312];
        s.read_exact(&mut reply).await.unwrap();
        assert_eq!(&reply[2..8], [0, 3, 0, 0, 0, 0], "imported");
        assert_eq!(&reply[8 + 300..8 + 304], [0x04, 0x5e, 0x02, 0x8e], "an Xbox 360 pad");
        imported.await.unwrap();
        let urb = |seq: u32, dir_in: bool, len: u32, data: &[u8]| {
            let mut h = vec![0u8; 48];
            h[..4].copy_from_slice(&1u32.to_be_bytes());
            h[4..8].copy_from_slice(&seq.to_be_bytes());
            h[12..16].copy_from_slice(&(dir_in as u32).to_be_bytes());
            h[16..20].copy_from_slice(&1u32.to_be_bytes());
            h[24..28].copy_from_slice(&len.to_be_bytes());
            [h, data.to_vec()].concat()
        };
        // Reports: resting, then A held.
        for (seq, state) in [(2, State::default()), (4, State { buttons: 1, ..Default::default() })] {
            s.write_all(&urb(seq, true, 32, &[])).await.unwrap();
            if seq == 4 {
                reports.send(xbox360::report(&state)).unwrap();
            }
            let mut ret = vec![0u8; 48 + 20];
            s.read_exact(&mut ret).await.unwrap();
            assert_eq!(&ret[4..8], seq.to_be_bytes());
            assert_eq!(ret[48..], xbox360::report(&state));
        }
        // A game's rumble.
        s.write_all(&urb(6, false, 8, &[0x00, 0x08, 0x00, 255, 0, 0, 0, 0])).await.unwrap();
        let mut ret = vec![0u8; 48];
        s.read_exact(&mut ret).await.unwrap();
        assert_eq!(rumbles.recv().await, Some((65535, 0, 0)));
    }

    /// A DualSense streams its reports, each counted, the phone's newest state in each; the
    /// pad's address and the report descriptor are there for SDL and Windows.
    #[tokio::test]
    async fn serves_a_streaming_dualsense() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = bind().await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (states, state_rx) = watch::channel(State { kind: Kind::DualSense, ..Default::default() });
        let (reports, rx) = watch::channel(dualsense::Reports::default().report(&State::default()));
        tokio::spawn(feed(Kind::DualSense, state_rx, reports));
        let (rumble_tx, _rumbles) = mpsc::unbounded_channel();
        let (imported_tx, imported) = tokio::sync::oneshot::channel();
        serve_pad(listener, Box::new(DualSense::new(rumble_tx, rx.clone())), rx, imported_tx);
        let mut s = tokio::net::TcpStream::connect((HOST, port)).await.unwrap();
        let mut busid = [0u8; 32];
        busid[..3].copy_from_slice(b"1-1");
        s.write_all(&[&0x0111u16.to_be_bytes()[..], &0x8003u16.to_be_bytes(), &[0; 4], &busid].concat()).await.unwrap();
        let mut reply = vec![0u8; 8 + 312];
        s.read_exact(&mut reply).await.unwrap();
        assert_eq!(&reply[8 + 300..8 + 304], [0x05, 0x4c, 0x0c, 0xe6], "a DualSense");
        imported.await.unwrap();
        let urb = |seq: u32, ep: u32, dir_in: bool, len: u32, setup: [u8; 8]| {
            let mut h = vec![0u8; 48];
            h[..4].copy_from_slice(&1u32.to_be_bytes());
            h[4..8].copy_from_slice(&seq.to_be_bytes());
            h[12..16].copy_from_slice(&(dir_in as u32).to_be_bytes());
            h[16..20].copy_from_slice(&ep.to_be_bytes());
            h[24..28].copy_from_slice(&len.to_be_bytes());
            h[40..48].copy_from_slice(&setup);
            h
        };
        let answer = async |s: &mut tokio::net::TcpStream| {
            let mut h = vec![0u8; 48];
            s.read_exact(&mut h).await.unwrap();
            let mut data = vec![0u8; u32::from_be_bytes(h[24..28].try_into().unwrap()) as usize];
            s.read_exact(&mut data).await.unwrap();
            data
        };
        // The report descriptor, as Windows' HID driver asks for it.
        s.write_all(&urb(2, 0, true, 512, [0x81, 6, 0, 0x22, 0, 0, 0, 2])).await.unwrap();
        assert_eq!(answer(&mut s).await, dualsense::REPORT_DESCRIPTOR);
        // Reports keep coming with nothing pressed, then with cross held.
        s.write_all(&urb(4, 4, true, 64, [0; 8])).await.unwrap();
        let a = answer(&mut s).await;
        s.write_all(&urb(6, 4, true, 64, [0; 8])).await.unwrap();
        let b = answer(&mut s).await;
        assert_ne!(a[7], b[7], "counted");
        states.send_replace(State { kind: Kind::DualSense, buttons: 1, ..Default::default() });
        tokio::time::sleep(dualsense::REPORT_EVERY * 3).await;
        s.write_all(&urb(8, 4, true, 64, [0; 8])).await.unwrap();
        assert_eq!(answer(&mut s).await[8] & 0xf0, 0x20, "cross");
    }

    /// Wine (and CI) have no USB/IP driver: the phone is told what to do, in words it shows.
    #[tokio::test]
    async fn without_the_driver_the_phone_hears_why() {
        if installed() {
            println!("usbip-win2 is installed here: skipped");
            return;
        }
        let e = Pad::create(Kind::Xbox).await.err().expect("no pad without the driver");
        assert_eq!(format!("{e:#}"), NO_DRIVER);
        assert!(NO_DRIVER.len() <= 140 && NO_DRIVER.starts_with(char::is_uppercase));
        let c = check();
        assert_eq!((c["name"].as_str(), c["action"].as_str()), (Some(CHECK), Some("Get driver")));
    }
}
