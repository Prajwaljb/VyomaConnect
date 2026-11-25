//! Bluetooth on Windows, as the Linux app's BlueZ profiles: RFCOMM services a bonded phone opens
//! by their UUIDs (Windows publishes each one's SDP record), through Winsock's own Bluetooth sockets
//! (Microsoft's stack, no driver):
//! - the link: piped to this agent's TCP port, so the phone's usual TLS link runs end to end when
//!   it has no Wi-Fi to the PC (the phone keeps this PC's radio address from `vyoma.bluetooth`);
//! - the controller's frames and the touchpad's (core `gamepad` and `pointer`), steadier than Wi-Fi.
//!
//! A Bluetooth socket isn't one tokio can wait on, so each connection is piped by two threads to
//! a loopback TCP stream, which the async side reads as it would the cable. A PC without a radio,
//! or with it off, tries again every [RETRY].
//! Built and tested on CI and under Wine only (Wine has no Bluetooth sockets); not yet run on a
//! real Windows PC.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tracing::{debug, info, warn};
use windows_sys::Win32::Devices::Bluetooth::{
    AF_BTH, BLUETOOTH_FIND_RADIO_PARAMS, BLUETOOTH_RADIO_INFO, BTHPROTO_RFCOMM, BluetoothFindFirstRadio, BluetoothFindRadioClose,
    BluetoothGetRadioInfo, NS_BTH, SO_BTH_AUTHENTICATE, SOCKADDR_BTH, SOL_RFCOMM,
};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::Networking::WinSock::{
    CSADDR_INFO, INVALID_SOCKET, RNRSERVICE_REGISTER, SD_BOTH, SOCK_STREAM, SOCKADDR, SOCKET, SOCKET_ADDRESS, WSADATA, WSAQUERYSETW,
    WSASetServiceW, WSAStartup, accept, bind, closesocket, getsockname, listen, recv, send, setsockopt, shutdown, socket,
};
use windows_sys::core::GUID;

use vyoma_core::pointer::Frame;

use crate::input::{Act, Button, Pointer};

/// Must match BluetoothTunnel.SERVICE on the phone.
const LINK: u128 = 0x6f1d3e52_8a47_4c1b_9d06_2be7a5c4f801;
/// Must match BluetoothPadChannel.SERVICE on the phone.
const PAD: u128 = 0x6f1d3e52_8a47_4c1b_9d06_2be7a5c4f803;
/// Must match BluetoothPointerChannel.SERVICE on the phone (core `pointer::SERVICE`).
const POINTER: u128 = 0x6f1d3e52_8a47_4c1b_9d06_2be7a5c4f804;
/// Any free RFCOMM channel: the phone finds it from the SDP record.
const BT_PORT_ANY: u32 = u32::MAX;
/// No radio, or it went away: looked for again this often.
const RETRY: Duration = Duration::from_secs(30);

/// Whether the services are up (a radio, on, and Windows took the records).
static READY: AtomicBool = AtomicBool::new(false);

pub fn ready() -> bool {
    READY.load(Ordering::Relaxed)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Service {
    Link,
    Pad,
    Pointer,
}

impl Service {
    fn uuid(self) -> u128 {
        match self {
            Service::Link => LINK,
            Service::Pad => PAD,
            Service::Pointer => POINTER,
        }
    }
}

/// A Winsock Bluetooth socket, closed when dropped.
struct Radio(SOCKET);

// SAFETY: a SOCKET is a handle; Winsock lets one thread receive while another sends.
unsafe impl Send for Radio {}
unsafe impl Sync for Radio {}

impl Radio {
    fn recv(&self, buf: &mut [u8]) -> std::io::Result<usize> {
        // SAFETY: a buffer we own and its length.
        let n = unsafe { recv(self.0, buf.as_mut_ptr(), buf.len().min(i32::MAX as usize) as i32, 0) };
        if n < 0 { Err(std::io::Error::last_os_error()) } else { Ok(n as usize) }
    }

    fn send_all(&self, mut buf: &[u8]) -> std::io::Result<()> {
        while !buf.is_empty() {
            // SAFETY: as in [recv].
            let n = unsafe { send(self.0, buf.as_ptr(), buf.len().min(i32::MAX as usize) as i32, 0) };
            if n <= 0 {
                return Err(std::io::Error::last_os_error());
            }
            buf = &buf[n as usize..];
        }
        Ok(())
    }

    fn end(&self) {
        // SAFETY: the socket is ours; shutting it twice is harmless.
        unsafe { shutdown(self.0, SD_BOTH) };
    }
}

impl Drop for Radio {
    fn drop(&mut self) {
        // SAFETY: the socket is ours and closed once.
        unsafe { closesocket(self.0) };
    }
}

/// Starts the services on threads of their own; [tcp_port] is this agent's TLS port. Controller and
/// touchpad streams are read on [rt].
pub fn start(tcp_port: u16, rt: tokio::runtime::Handle) {
    let spawned = std::thread::Builder::new().name("vyoma-bluetooth".into()).spawn(move || {
        let mut said = false;
        loop {
            match open_all() {
                Ok(servers) => {
                    info!("bluetooth services ready");
                    READY.store(true, Ordering::Relaxed);
                    said = false;
                    // Each server accepts on its own thread; when one fails (the radio went off) all are opened again.
                    let (failed_tx, failed_rx) = std::sync::mpsc::channel();
                    let servers: Vec<Arc<Radio>> = servers.into_iter().map(|(service, server)| {
                        let server = Arc::new(server);
                        let (server2, failed, rt) = (server.clone(), failed_tx.clone(), rt.clone());
                        let _ = std::thread::Builder::new().name("vyoma-bt-accept".into()).spawn(move || {
                            serve(service, &server2, tcp_port, &rt);
                            let _ = failed.send(());
                        });
                        server
                    }).collect();
                    let _ = failed_rx.recv();
                    READY.store(false, Ordering::Relaxed);
                    warn!("bluetooth services stopped; trying again");
                    for s in &servers {
                        s.end();
                    }
                }
                Err(e) => {
                    if !said {
                        info!("no Bluetooth services: {e:#}");
                        said = true;
                    } else {
                        debug!("still no Bluetooth: {e:#}");
                    }
                }
            }
            std::thread::sleep(RETRY);
        }
    });
    if let Err(e) = spawned {
        warn!("no Bluetooth thread: {e}");
    }
}

fn open_all() -> Result<Vec<(Service, Radio)>> {
    startup();
    [Service::Link, Service::Pad, Service::Pointer].into_iter().map(|s| Ok((s, listen_on(s)?))).collect()
}

fn startup() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let mut data: WSADATA = unsafe { std::mem::zeroed() };
        // SAFETY: a WSADATA to fill; std's own sockets call this too, and it counts.
        unsafe { WSAStartup(0x0202, &mut data) };
    });
}

/// A listening RFCOMM socket on any channel, its SDP record published under [service]'s UUID.
fn listen_on(service: Service) -> Result<Radio> {
    // SAFETY: plain Winsock calls on a socket we own; the structs outlive each call.
    unsafe {
        let s = socket(AF_BTH as i32, SOCK_STREAM, BTHPROTO_RFCOMM as i32);
        if s == INVALID_SOCKET {
            bail!("no Bluetooth socket (no radio?): {}", std::io::Error::last_os_error());
        }
        let radio = Radio(s);
        // Bonded phones only: TLS (or the touchpad's key) decides who gets further.
        let on: u32 = 1;
        setsockopt(s, SOL_RFCOMM as i32, SO_BTH_AUTHENTICATE as i32, (&on as *const u32).cast(), 4);
        let mut addr = SOCKADDR_BTH { addressFamily: AF_BTH, btAddr: 0, serviceClassId: GUID::from_u128(0), port: BT_PORT_ANY };
        let len = size_of::<SOCKADDR_BTH>() as i32;
        if bind(s, (&addr as *const SOCKADDR_BTH).cast::<SOCKADDR>(), len) != 0 {
            bail!("couldn't take an RFCOMM channel: {}", std::io::Error::last_os_error());
        }
        if listen(s, 4) != 0 {
            bail!("couldn't listen on Bluetooth: {}", std::io::Error::last_os_error());
        }
        let mut got = len;
        if getsockname(s, (&mut addr as *mut SOCKADDR_BTH).cast::<SOCKADDR>(), &mut got) != 0 {
            bail!("no RFCOMM channel: {}", std::io::Error::last_os_error());
        }
        let mut uuid = GUID::from_u128(service.uuid());
        let mut name: Vec<u16> = "Vyoma".encode_utf16().chain([0]).collect();
        let mut csaddr = CSADDR_INFO {
            LocalAddr: SOCKET_ADDRESS { lpSockaddr: (&mut addr as *mut SOCKADDR_BTH).cast::<SOCKADDR>(), iSockaddrLength: len },
            RemoteAddr: SOCKET_ADDRESS { lpSockaddr: std::ptr::null_mut(), iSockaddrLength: 0 },
            iSocketType: SOCK_STREAM,
            iProtocol: BTHPROTO_RFCOMM as i32,
        };
        let mut query: WSAQUERYSETW = std::mem::zeroed();
        query.dwSize = size_of::<WSAQUERYSETW>() as u32;
        query.lpszServiceInstanceName = name.as_mut_ptr();
        query.lpServiceClassId = &mut uuid;
        query.dwNameSpace = NS_BTH;
        query.dwNumberOfCsAddrs = 1;
        query.lpcsaBuffer = &mut csaddr;
        if WSASetServiceW(&query, RNRSERVICE_REGISTER, 0) != 0 {
            bail!("Windows didn't publish the {service:?} service: {}", std::io::Error::last_os_error());
        }
        debug!("bluetooth {service:?} on RFCOMM channel {}", { addr.port });
        Ok(radio)
    }
}

/// Accepts [service]'s connections until the server fails.
fn serve(service: Service, server: &Radio, tcp_port: u16, rt: &tokio::runtime::Handle) {
    loop {
        // SAFETY: accepting on our listening socket; the peer's address isn't wanted.
        let s = unsafe { accept(server.0, std::ptr::null_mut(), std::ptr::null_mut()) };
        if s == INVALID_SOCKET {
            return debug!("bluetooth {service:?}: {}", std::io::Error::last_os_error());
        }
        let radio = Arc::new(Radio(s));
        let done = match service {
            Service::Link => link(radio, tcp_port),
            Service::Pad => frames(radio, rt, |stream| Box::pin(vyoma_core::gamepad::read_frames(stream, "Bluetooth", vyoma_core::gamepad::ECHO_EVERY))),
            Service::Pointer => frames(radio, rt, |stream| Box::pin(vyoma_core::pointer::read_frames(stream))),
        };
        if let Err(e) = done {
            warn!("bluetooth {service:?}: {e:#}");
        }
    }
}

/// The phone's link over the radio, to this agent's own TCP port.
fn link(radio: Arc<Radio>, tcp_port: u16) -> Result<()> {
    info!("bluetooth link opened");
    let tcp = TcpStream::connect(("127.0.0.1", tcp_port)).context("Vyoma isn't listening")?;
    pipe(radio, tcp);
    Ok(())
}

type Reader = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;

/// A stream of frames, read on the runtime through a loopback pair.
fn frames(radio: Arc<Radio>, rt: &tokio::runtime::Handle, read: fn(tokio::net::TcpStream) -> Reader) -> Result<()> {
    let local = TcpListener::bind("127.0.0.1:0")?;
    let near = TcpStream::connect(local.local_addr()?)?;
    let (far, _) = local.accept()?;
    near.set_nodelay(true)?;
    far.set_nodelay(true)?;
    far.set_nonblocking(true)?;
    pipe(radio, near);
    let _guard = rt.enter();
    let stream = tokio::net::TcpStream::from_std(far)?;
    rt.spawn(read(stream));
    Ok(())
}

/// Copies both ways on two threads; either side ending ends both.
fn pipe(radio: Arc<Radio>, tcp: TcpStream) {
    let _ = tcp.set_nodelay(true);
    let Ok(mut tcp_in) = tcp.try_clone() else { return radio.end() };
    let mut tcp_out = tcp;
    let up = radio.clone();
    let _ = std::thread::Builder::new().name("vyoma-bt-in".into()).spawn(move || {
        let mut buf = [0u8; 16 * 1024];
        while let Ok(n) = up.recv(&mut buf) {
            if n == 0 || tcp_out.write_all(&buf[..n]).is_err() {
                break;
            }
        }
        up.end();
        let _ = tcp_out.shutdown(Shutdown::Both);
    });
    let _ = std::thread::Builder::new().name("vyoma-bt-out".into()).spawn(move || {
        let mut buf = [0u8; 16 * 1024];
        while let Ok(n) = tcp_in.read(&mut buf) {
            if n == 0 || radio.send_all(&buf[..n]).is_err() {
                break;
            }
        }
        radio.end();
        let _ = tcp_in.shutdown(Shutdown::Both);
    });
}

/// One touchpad over Bluetooth while its page is open on the phone: hands over the key, then
/// performs the frames with SendInput until [Frame::Close].
pub async fn touchpad(sender: vyoma_core::link::Sender, name: String, tx: tokio::sync::mpsc::Sender<Frame>, mut rx: tokio::sync::mpsc::Receiver<Frame>) {
    use serde_json::json;
    use vyoma_core::link::packet;
    if !ready() {
        let error = "This PC's Bluetooth is off or missing";
        info!("{name}: no touchpad over Bluetooth: {error}");
        let _ = sender.send(&packet("vyoma.pointer.ready", json!({ "error": error }))).await;
        return;
    }
    let Some((_open, key)) = vyoma_core::pointer::open(tx) else { return };
    if sender.send(&packet("vyoma.pointer.ready", json!({ "key": key }))).await.is_err() {
        return;
    }
    let mut pointer = Pointer::default();
    let mut held = [false; vyoma_core::pointer::BUTTONS];
    while let Some(frame) = rx.recv().await {
        let acts = match frame {
            Frame::KeepAwake => continue,
            Frame::Move(dx, dy) => vec![Act::Move(dx, dy)],
            Frame::Scroll(dx, dy) => vec![Act::Scroll(dx, dy)],
            Frame::Button(b, down) => {
                held[b] = down;
                vec![Act::Button(BUTTONS[b], down)]
            }
            // Nothing stays pressed after the radio drops or the page closes.
            Frame::StreamEnded | Frame::Close => {
                let up: Vec<Act> = held.iter_mut().enumerate().filter_map(|(b, h)| std::mem::take(h).then(|| Act::Button(BUTTONS[b], false))).collect();
                let _ = pointer.handle(&up);
                if frame == Frame::Close {
                    return;
                }
                continue;
            }
        };
        if let Err(e) = pointer.handle(&acts) {
            debug!("touchpad: {e:#}");
        }
    }
}

const BUTTONS: [Button; vyoma_core::pointer::BUTTONS] = [Button::Left, Button::Right, Button::Middle];

/// This PC's radio address ("AA:BB:CC:DD:EE:FF"), for the phone to keep: with Wi-Fi gone it can't
/// look for the PC. None without a radio.
pub fn address() -> Option<String> {
    let params = BLUETOOTH_FIND_RADIO_PARAMS { dwSize: size_of::<BLUETOOTH_FIND_RADIO_PARAMS>() as u32 };
    let mut radio: HANDLE = std::ptr::null_mut();
    // SAFETY: the find handle and radio handle are closed below; the info struct is ours.
    unsafe {
        let find = BluetoothFindFirstRadio(&params, &mut radio);
        if find.is_null() {
            return None;
        }
        BluetoothFindRadioClose(find);
        let mut info = BLUETOOTH_RADIO_INFO { dwSize: size_of::<BLUETOOTH_RADIO_INFO>() as u32, ..Default::default() };
        let got = BluetoothGetRadioInfo(radio, &mut info);
        CloseHandle(radio);
        (got == 0).then(|| format_address(info.address.Anonymous.rgBytes))
    }
}

/// Windows keeps the address least significant byte first.
fn format_address(bytes: [u8; 6]) -> String {
    bytes.iter().rev().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_phones_uuids() {
        let text = |u: u128| {
            let h = format!("{u:032x}");
            format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
        };
        assert_eq!(text(LINK), "6f1d3e52-8a47-4c1b-9d06-2be7a5c4f801");
        assert_eq!(text(PAD), "6f1d3e52-8a47-4c1b-9d06-2be7a5c4f803");
        assert_eq!(text(POINTER), vyoma_core::pointer::SERVICE);
    }

    #[test]
    fn addresses_read_most_significant_first() {
        assert_eq!(format_address([0x66, 0x55, 0x44, 0x33, 0x22, 0x11]), "11:22:33:44:55:66");
    }

    /// A radio connection's bytes reach the touchpad through the pipe and the loopback pair: a TCP
    /// socket stands in for the phone's RFCOMM one (Radio only receives and sends on it).
    #[test]
    fn frames_cross_the_pipe_to_the_touchpad() {
        use base64::Engine;
        use ring::hmac;
        use std::os::windows::io::IntoRawSocket;
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let (_open, key) = vyoma_core::pointer::open(tx).unwrap();
        let key = hmac::Key::new(hmac::HMAC_SHA256, &base64::engine::general_purpose::STANDARD.decode(key).unwrap());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut phone = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (pc, _) = listener.accept().unwrap();
        let radio = Arc::new(Radio(pc.into_raw_socket() as SOCKET));
        frames(radio, rt.handle(), |stream| Box::pin(vyoma_core::pointer::read_frames(stream))).unwrap();
        for (seq, kind, dx) in [(1u32, 1u8, 5.0f32), (2, 3, 0.0)] {
            let mut f = Vec::from(*b"VYP1");
            f.extend(seq.to_be_bytes());
            f.push(kind);
            f.extend(dx.to_be_bytes());
            f.extend(0f32.to_be_bytes());
            let tag = hmac::sign(&key, &f);
            f.extend(&tag.as_ref()[..16]);
            phone.write_all(&f).unwrap();
        }
        let got = rt.block_on(async {
            let mut got = Vec::new();
            for _ in 0..2 {
                got.push(tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.unwrap().unwrap());
            }
            drop(phone);
            got.push(tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.unwrap().unwrap());
            got
        });
        assert_eq!(got, [Frame::Move(5.0, 0.0), Frame::Button(0, true), Frame::StreamEnded], "the radio closing lets go");
    }

    /// Wine has no Bluetooth sockets: the services say so and wait, never panic.
    #[test]
    fn no_radio_is_an_error_not_a_crash() {
        if let Err(e) = open_all() {
            assert!(!format!("{e:#}").is_empty());
        }
        let _ = address();
    }
}
