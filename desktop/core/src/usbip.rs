//! A USB device made in software and served over USB/IP (the protocol Linux's usbip and Windows'
//! usbip-win2 speak), so the computer's own USB stack takes it for a real one plugged in. The client
//! (a driver on this PC) connects, imports the device by its bus id, then sends URBs: control
//! transfers on endpoint 0, which the standard requests answer here and the rest go to the
//! [Gadget]; data from the host on OUT endpoints (a pad's rumble), handed to the gadget; and IN
//! transfers on the report endpoint, each held until there's a new report (as a real device NAKs),
//! then answered with it. Other IN endpoints never answer. No OS calls: each platform attaches it.
//!
//! The wire format (all big-endian), from Linux's Documentation/usb/usbip_protocol.rst: an 8-byte
//! op header (version 0x0111, code, status), OP_REQ_IMPORT with a 32-byte bus id, OP_REP_IMPORT with
//! the device's summary; then 48-byte URB headers (CMD_SUBMIT with its setup packet and any OUT
//! data, CMD_UNLINK) and their replies (RET_SUBMIT with any IN data, RET_UNLINK).

use std::collections::VecDeque;

use anyhow::{Context, Result, bail};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, watch};

const VERSION: u16 = 0x0111;
const OP_REQ_DEVLIST: u16 = 0x8005;
const OP_REP_DEVLIST: u16 = 0x0005;
const OP_REQ_IMPORT: u16 = 0x8003;
const OP_REP_IMPORT: u16 = 0x0003;
/// op status: no device by that bus id.
const ST_NODEV: u32 = 4;

const CMD_SUBMIT: u32 = 1;
const CMD_UNLINK: u32 = 2;
const RET_SUBMIT: u32 = 3;
const RET_UNLINK: u32 = 4;
const DIR_IN: u32 = 1;
/// Linux's errnos, as the protocol carries them.
const ECONNRESET: i32 = -104;
const EPIPE: i32 = -32;

const BUSID_LEN: usize = 32;
const HEADER_LEN: usize = 48;
/// Larger OUT transfers than this end the connection (a pad's are 8 bytes).
const OUT_MAX: u32 = 64 * 1024;
/// Full speed (12 Mb/s), as USB 2.0 pads are.
pub const FULL_SPEED: u32 = 2;

/// A control transfer's setup packet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Setup {
    pub request_type: u8,
    pub request: u8,
    pub value: u16,
    pub index: u16,
    pub length: u16,
}

impl Setup {
    pub fn parse(b: &[u8; 8]) -> Setup {
        Setup {
            request_type: b[0],
            request: b[1],
            value: u16::from_le_bytes([b[2], b[3]]),
            index: u16::from_le_bytes([b[4], b[5]]),
            length: u16::from_le_bytes([b[6], b[7]]),
        }
    }

    fn device_to_host(&self) -> bool {
        self.request_type & 0x80 != 0
    }
}

/// What the device is: its descriptors and what it does with what the host sends.
pub trait Gadget: Send + Sync {
    /// The 18-byte device descriptor.
    fn device(&self) -> &[u8];
    /// The configuration descriptor with its interfaces and endpoints.
    fn configuration(&self) -> &[u8];
    /// String descriptor [index] (1 up), as text.
    fn string(&self, index: u8) -> Option<&str>;
    /// A request the standard ones don't cover (vendor, class, an interface's descriptors): its
    /// answer, empty when there's nothing to say.
    fn control(&mut self, setup: &Setup, data: &[u8]) -> Vec<u8>;
    /// Data from the host on OUT endpoint [ep].
    fn out(&mut self, ep: u8, data: &[u8]);
    /// The IN endpoint (number, without the direction bit) the reports go out on.
    fn report_endpoint(&self) -> u8;
}

/// The device's summary in OP_REP_IMPORT / OP_REP_DEVLIST (312 bytes).
fn summary(g: &dyn Gadget, busid: &str) -> Vec<u8> {
    let d = g.device();
    let c = g.configuration();
    let (busnum, devnum) = busid.split_once('-').map_or((1, 1), |(b, d)| (b.parse().unwrap_or(1), d.parse().unwrap_or(1)));
    let mut out = vec![0u8; 256]; // path: none
    let mut id = [0u8; BUSID_LEN];
    id[..busid.len().min(BUSID_LEN - 1)].copy_from_slice(&busid.as_bytes()[..busid.len().min(BUSID_LEN - 1)]);
    out.extend_from_slice(&id);
    out.extend_from_slice(&(busnum as u32).to_be_bytes());
    out.extend_from_slice(&(devnum as u32).to_be_bytes());
    out.extend_from_slice(&FULL_SPEED.to_be_bytes());
    for at in [8, 10, 12] {
        out.extend_from_slice(&u16::from_le_bytes([d[at], d[at + 1]]).to_be_bytes());
    }
    // Class, subclass, protocol; configuration 1; configurations; interfaces.
    out.extend_from_slice(&[d[4], d[5], d[6], 1, d[17], c[4]]);
    out
}

fn op_header(code: u16, status: u32) -> [u8; 8] {
    let mut h = [0u8; 8];
    h[..2].copy_from_slice(&VERSION.to_be_bytes());
    h[2..4].copy_from_slice(&code.to_be_bytes());
    h[4..].copy_from_slice(&status.to_be_bytes());
    h
}

/// Answers one connection's op request: true when the device was imported on it (its URBs follow).
pub async fn handshake<S: AsyncRead + AsyncWrite + Unpin>(stream: &mut S, busid: &str, gadget: &dyn Gadget) -> Result<bool> {
    let mut op = [0u8; 8];
    stream.read_exact(&mut op).await.context("no op request")?;
    let (version, code) = (u16::from_be_bytes([op[0], op[1]]), u16::from_be_bytes([op[2], op[3]]));
    if version != VERSION {
        bail!("USB/IP version {version:#x}, not {VERSION:#x}");
    }
    match code {
        OP_REQ_DEVLIST => {
            let mut reply = op_header(OP_REP_DEVLIST, 0).to_vec();
            reply.extend_from_slice(&1u32.to_be_bytes());
            reply.extend_from_slice(&summary(gadget, busid));
            // Each interface: class, subclass, protocol, padding.
            let c = gadget.configuration();
            let mut at = 0;
            while at + 1 < c.len() && c[at] > 0 {
                if c[at + 1] == 4 && at + 8 <= c.len() {
                    reply.extend_from_slice(&[c[at + 5], c[at + 6], c[at + 7], 0]);
                }
                at += c[at] as usize;
            }
            stream.write_all(&reply).await?;
            Ok(false)
        }
        OP_REQ_IMPORT => {
            let mut asked = [0u8; BUSID_LEN];
            stream.read_exact(&mut asked).await?;
            let asked = std::str::from_utf8(&asked[..asked.iter().position(|&b| b == 0).unwrap_or(BUSID_LEN)]).unwrap_or("");
            if asked != busid {
                stream.write_all(&op_header(OP_REP_IMPORT, ST_NODEV)).await?;
                bail!("asked for bus id {asked:?}");
            }
            let mut reply = op_header(OP_REP_IMPORT, 0).to_vec();
            reply.extend_from_slice(&summary(gadget, busid));
            stream.write_all(&reply).await?;
            Ok(true)
        }
        _ => bail!("unknown USB/IP op {code:#x}"),
    }
}

/// One URB from the host.
#[derive(Debug)]
enum Urb {
    Submit { seq: u32, ep: u8, dir_in: bool, length: u32, setup: [u8; 8], data: Vec<u8> },
    Unlink { seq: u32, victim: u32 },
}

async fn read_urb<R: AsyncRead + Unpin>(r: &mut R) -> Result<Urb> {
    let mut h = [0u8; HEADER_LEN];
    r.read_exact(&mut h).await?;
    let word = |at: usize| u32::from_be_bytes(h[at..at + 4].try_into().unwrap());
    let seq = word(4);
    match word(0) {
        CMD_SUBMIT => {
            let (dir_in, ep, length, packets) = (word(12) == DIR_IN, word(16) as u8, word(24), word(32) as i32);
            if packets > 0 {
                bail!("isochronous transfers aren't served");
            }
            let mut data = Vec::new();
            if !dir_in && length > 0 {
                if length > OUT_MAX {
                    bail!("an OUT transfer of {length} bytes");
                }
                data = vec![0u8; length as usize];
                r.read_exact(&mut data).await?;
            }
            Ok(Urb::Submit { seq, ep, dir_in, length, setup: h[40..48].try_into().unwrap(), data })
        }
        CMD_UNLINK => Ok(Urb::Unlink { seq, victim: word(20) }),
        other => bail!("unknown URB command {other}"),
    }
}

fn ret_submit(seq: u32, status: i32, actual: u32, data: &[u8]) -> Vec<u8> {
    let mut m = vec![0u8; HEADER_LEN];
    m[..4].copy_from_slice(&RET_SUBMIT.to_be_bytes());
    m[4..8].copy_from_slice(&seq.to_be_bytes());
    m[20..24].copy_from_slice(&status.to_be_bytes());
    m[24..28].copy_from_slice(&actual.to_be_bytes());
    m.extend_from_slice(data);
    m
}

fn ret_unlink(seq: u32, status: i32) -> Vec<u8> {
    let mut m = vec![0u8; HEADER_LEN];
    m[..4].copy_from_slice(&RET_UNLINK.to_be_bytes());
    m[4..8].copy_from_slice(&seq.to_be_bytes());
    m[20..24].copy_from_slice(&status.to_be_bytes());
    m
}

fn string_descriptor(text: &str) -> Vec<u8> {
    let units: Vec<u16> = text.encode_utf16().take(126).collect();
    let mut d = vec![(2 + units.len() * 2) as u8, 3];
    for u in units {
        d.extend_from_slice(&u.to_le_bytes());
    }
    d
}

/// The answer to a control transfer: Some(data) (empty for none), None to stall.
fn control(g: &mut dyn Gadget, s: &Setup, data: &[u8]) -> Option<Vec<u8>> {
    let answer = match (s.request_type, s.request) {
        // GET_DESCRIPTOR from the device: device, configuration, strings.
        (0x80, 6) => match ((s.value >> 8) as u8, s.value as u8) {
            (1, _) => g.device().to_vec(),
            (2, _) => g.configuration().to_vec(),
            (3, 0) => vec![4, 3, 0x09, 0x04], // English (US)
            (3, i) => string_descriptor(g.string(i)?),
            // Device qualifier, other speed, BOS: a full-speed USB 2.0 device has none.
            _ => return None,
        },
        // GET_STATUS (device, interface, endpoint): bus powered, nothing halted.
        (0x80..=0x82, 0) => vec![0, 0],
        // GET_CONFIGURATION, GET_INTERFACE: the one there is.
        (0x80, 8) => vec![1],
        (0x81, 10) => vec![0],
        // SET_ADDRESS, SET_CONFIGURATION, SET_INTERFACE, CLEAR_FEATURE, SET_FEATURE (a port reset too).
        (0x00, 5 | 9) | (0x01, 11) | (0x00..=0x02, 1 | 3) | (0x23, 3) => Vec::new(),
        _ => g.control(s, data),
    };
    Some(answer)
}

/// Serves the imported device's URBs until the host lets go; [reports] are what the report
/// endpoint sends, the newest first to each IN transfer.
pub async fn serve<S: AsyncRead + AsyncWrite + Send + 'static>(stream: S, gadget: &mut dyn Gadget, mut reports: watch::Receiver<Vec<u8>>) -> Result<()> {
    let (mut rd, mut wr) = tokio::io::split(stream);
    let (tx, mut urbs) = mpsc::channel::<Result<Urb>>(64);
    let reader = tokio::spawn(async move {
        loop {
            let urb = read_urb(&mut rd).await;
            let end = urb.is_err();
            if tx.send(urb).await.is_err() || end {
                return;
            }
        }
    });
    let _stop_reading = AbortOnDrop(reader);
    let report_ep = gadget.report_endpoint();
    // IN transfers waiting for a report, and ones on endpoints that never answer.
    let mut waiting: VecDeque<(u32, u32)> = VecDeque::new();
    let mut parked: Vec<u32> = Vec::new();
    // A report the host hasn't had yet (the first, too).
    let mut fresh = true;
    let mut reports_open = true;
    loop {
        let reply = tokio::select! {
            urb = urbs.recv() => {
                let Some(urb) = urb else { return Ok(()) };
                match urb {
                    Err(e) if is_eof(&e) => return Ok(()),
                    Err(e) => return Err(e),
                    Ok(Urb::Unlink { seq, victim }) => {
                        let before = waiting.len() + parked.len();
                        waiting.retain(|(s, _)| *s != victim);
                        parked.retain(|s| *s != victim);
                        // Still waiting: it's undone; else its answer has already gone.
                        ret_unlink(seq, if waiting.len() + parked.len() < before { ECONNRESET } else { 0 })
                    }
                    Ok(Urb::Submit { seq, ep: 0, setup, data, length, .. }) => {
                        let s = Setup::parse(&setup);
                        match control(gadget, &s, &data) {
                            Some(mut answer) if s.device_to_host() => {
                                answer.truncate(s.length.min(length.min(u16::MAX as u32) as u16) as usize);
                                ret_submit(seq, 0, answer.len() as u32, &answer)
                            }
                            Some(_) => ret_submit(seq, 0, data.len() as u32, &[]),
                            None => ret_submit(seq, EPIPE, 0, &[]),
                        }
                    }
                    Ok(Urb::Submit { seq, ep, dir_in: false, data, .. }) => {
                        gadget.out(ep, &data);
                        ret_submit(seq, 0, data.len() as u32, &[])
                    }
                    Ok(Urb::Submit { seq, ep, length, .. }) if ep == report_ep => {
                        if fresh {
                            fresh = false;
                            let mut r = reports.borrow_and_update().clone();
                            r.truncate(length as usize);
                            ret_submit(seq, 0, r.len() as u32, &r)
                        } else {
                            waiting.push_back((seq, length));
                            continue;
                        }
                    }
                    Ok(Urb::Submit { seq, .. }) => {
                        parked.push(seq);
                        continue;
                    }
                }
            }
            changed = reports.changed(), if reports_open => {
                if changed.is_err() {
                    // The pad's gone: nothing more to report (the host is about to be told).
                    reports_open = false;
                    continue;
                }
                match waiting.pop_front() {
                    Some((seq, length)) => {
                        let mut r = reports.borrow_and_update().clone();
                        r.truncate(length as usize);
                        ret_submit(seq, 0, r.len() as u32, &r)
                    }
                    None => {
                        fresh = true;
                        continue;
                    }
                }
            }
        };
        wr.write_all(&reply).await?;
    }
}

fn is_eof(e: &anyhow::Error) -> bool {
    e.downcast_ref::<std::io::Error>().is_some_and(|e| matches!(e.kind(), std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted))
}

struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// The client's side, for tests: a host that imports the device and drives it.
#[cfg(test)]
pub(crate) mod host {
    use super::*;

    pub async fn import<S: AsyncRead + AsyncWrite + Unpin>(s: &mut S, busid: &str) -> Result<Vec<u8>> {
        let mut req = op_header(OP_REQ_IMPORT, 0).to_vec();
        let mut id = [0u8; BUSID_LEN];
        id[..busid.len()].copy_from_slice(busid.as_bytes());
        req.extend_from_slice(&id);
        s.write_all(&req).await?;
        let mut op = [0u8; 8];
        s.read_exact(&mut op).await?;
        if u32::from_be_bytes(op[4..].try_into().unwrap()) != 0 {
            bail!("refused");
        }
        let mut dev = vec![0u8; 312];
        s.read_exact(&mut dev).await?;
        Ok(dev)
    }

    pub async fn submit<S: AsyncWrite + Unpin>(s: &mut S, seq: u32, ep: u32, dir_in: bool, length: u32, setup: [u8; 8], data: &[u8]) -> Result<()> {
        let mut h = vec![0u8; HEADER_LEN];
        h[..4].copy_from_slice(&CMD_SUBMIT.to_be_bytes());
        h[4..8].copy_from_slice(&seq.to_be_bytes());
        h[12..16].copy_from_slice(&(dir_in as u32).to_be_bytes());
        h[16..20].copy_from_slice(&ep.to_be_bytes());
        h[24..28].copy_from_slice(&length.to_be_bytes());
        h[32..36].copy_from_slice(&(-1i32).to_be_bytes());
        h[40..48].copy_from_slice(&setup);
        h.extend_from_slice(data);
        s.write_all(&h).await?;
        Ok(())
    }

    pub async fn unlink<S: AsyncWrite + Unpin>(s: &mut S, seq: u32, victim: u32) -> Result<()> {
        let mut h = vec![0u8; HEADER_LEN];
        h[..4].copy_from_slice(&CMD_UNLINK.to_be_bytes());
        h[4..8].copy_from_slice(&seq.to_be_bytes());
        h[20..24].copy_from_slice(&victim.to_be_bytes());
        s.write_all(&h).await?;
        Ok(())
    }

    /// A reply: (command, seq, status, data).
    pub async fn reply<S: AsyncRead + Unpin>(s: &mut S) -> Result<(u32, u32, i32, Vec<u8>)> {
        let mut h = [0u8; HEADER_LEN];
        s.read_exact(&mut h).await?;
        let word = |at: usize| u32::from_be_bytes(h[at..at + 4].try_into().unwrap());
        let (cmd, seq, status, actual) = (word(0), word(4), word(20) as i32, word(24));
        let mut data = Vec::new();
        if cmd == RET_SUBMIT && actual > 0 && actual <= OUT_MAX {
            // OUT replies carry no data: the tests only read IN ones' (they say how long).
            data = vec![0u8; actual as usize];
        }
        Ok((cmd, seq, status, data))
    }

    pub async fn reply_in<S: AsyncRead + Unpin>(s: &mut S) -> Result<(u32, u32, i32, Vec<u8>)> {
        let (cmd, seq, status, mut data) = reply(s).await?;
        s.read_exact(&mut data).await?;
        Ok((cmd, seq, status, data))
    }

    pub fn get_descriptor(kind: u8, index: u8, length: u16) -> [u8; 8] {
        let l = length.to_le_bytes();
        [0x80, 6, index, kind, 0, 0, l[0], l[1]]
    }
}

#[cfg(test)]
mod tests {
    use super::host::*;
    use super::*;

    /// A tiny gadget: one interface with an IN and an OUT endpoint.
    struct Tiny {
        outs: Vec<(u8, Vec<u8>)>,
    }

    const TINY_DEVICE: [u8; 18] = [18, 1, 0x00, 0x02, 0xff, 0xff, 0xff, 8, 0x34, 0x12, 0x78, 0x56, 0x00, 0x01, 1, 2, 0, 1];
    const TINY_CONFIG: [u8; 32] = [
        9, 2, 32, 0, 1, 1, 0, 0x80, 50, // configuration
        9, 4, 0, 0, 2, 0xff, 0x5d, 0x01, 0, // interface
        7, 5, 0x81, 3, 32, 0, 4, // IN 1
        7, 5, 0x01, 3, 32, 0, 8, // OUT 1
    ];

    impl Gadget for Tiny {
        fn device(&self) -> &[u8] {
            &TINY_DEVICE
        }
        fn configuration(&self) -> &[u8] {
            &TINY_CONFIG
        }
        fn string(&self, index: u8) -> Option<&str> {
            [None, Some("Maker"), Some("Thing")].get(index as usize).copied().flatten()
        }
        fn control(&mut self, s: &Setup, _: &[u8]) -> Vec<u8> {
            if s.request_type == 0xc1 { vec![7, 7, 7] } else { Vec::new() }
        }
        fn out(&mut self, ep: u8, data: &[u8]) {
            self.outs.push((ep, data.to_vec()));
        }
        fn report_endpoint(&self) -> u8 {
            1
        }
    }

    #[tokio::test]
    async fn refuses_another_bus_id() {
        let (mut host, mut dev) = tokio::io::duplex(4096);
        let served = tokio::spawn(async move { handshake(&mut dev, "1-1", &Tiny { outs: vec![] }).await });
        assert!(import(&mut host, "2-7").await.is_err());
        assert!(served.await.unwrap().is_err());
    }

    #[tokio::test]
    async fn lists_the_device() {
        let (mut host, mut dev) = tokio::io::duplex(4096);
        tokio::spawn(async move { handshake(&mut dev, "1-1", &Tiny { outs: vec![] }).await });
        host.write_all(&op_header(OP_REQ_DEVLIST, 0)).await.unwrap();
        let mut got = vec![0u8; 8 + 4 + 312 + 4];
        host.read_exact(&mut got).await.unwrap();
        assert_eq!(&got[12 + 256..12 + 259], b"1-1");
        assert_eq!(&got[got.len() - 4..], [0xff, 0x5d, 0x01, 0], "its one interface");
    }

    #[tokio::test]
    async fn imports_and_serves_the_device() {
        let (mut host, mut dev) = tokio::io::duplex(1 << 16);
        let (reports, rx) = watch::channel(vec![1, 2, 3]);
        let served = tokio::spawn(async move {
            let mut tiny = Tiny { outs: vec![] };
            assert!(handshake(&mut dev, "1-1", &tiny).await.unwrap());
            serve(dev, &mut tiny, rx).await.unwrap();
            tiny
        });
        let summary = import(&mut host, "1-1").await.unwrap();
        assert_eq!(&summary[256..259], b"1-1");
        let word = |at: usize| u32::from_be_bytes(summary[at..at + 4].try_into().unwrap());
        assert_eq!((word(288), word(292), word(296)), (1, 1, FULL_SPEED), "bus 1, device 1, full speed");
        assert_eq!(&summary[300..306], [0x12, 0x34, 0x56, 0x78, 0x01, 0x00], "vendor, product, release");
        assert_eq!(&summary[306..312], [0xff, 0xff, 0xff, 1, 1, 1]);

        // Descriptors, cut to what the host asked for.
        submit(&mut host, 2, 0, true, 64, get_descriptor(1, 0, 64), &[]).await.unwrap();
        assert_eq!(reply_in(&mut host).await.unwrap(), (RET_SUBMIT, 2, 0, TINY_DEVICE.to_vec()));
        submit(&mut host, 4, 0, true, 9, get_descriptor(2, 0, 9), &[]).await.unwrap();
        assert_eq!(reply_in(&mut host).await.unwrap().3, TINY_CONFIG[..9]);
        submit(&mut host, 6, 0, true, 255, get_descriptor(3, 2, 255), &[]).await.unwrap();
        assert_eq!(reply_in(&mut host).await.unwrap().3, [12, 3, b'T', 0, b'h', 0, b'i', 0, b'n', 0, b'g', 0]);
        submit(&mut host, 8, 0, true, 255, get_descriptor(3, 9, 255), &[]).await.unwrap();
        assert_eq!(reply_in(&mut host).await.unwrap().2, EPIPE, "no such string");
        submit(&mut host, 10, 0, true, 10, get_descriptor(6, 0, 10), &[]).await.unwrap();
        assert_eq!(reply_in(&mut host).await.unwrap().2, EPIPE, "no device qualifier at full speed");
        // SET_CONFIGURATION, then a vendor request the gadget answers.
        submit(&mut host, 12, 0, false, 0, [0, 9, 1, 0, 0, 0, 0, 0], &[]).await.unwrap();
        assert_eq!(reply(&mut host).await.unwrap(), (RET_SUBMIT, 12, 0, vec![]));
        submit(&mut host, 14, 0, true, 2, [0xc1, 1, 0, 1, 0, 0, 2, 0], &[]).await.unwrap();
        assert_eq!(reply_in(&mut host).await.unwrap().3, [7, 7], "cut to its length");

        // The first IN gets the report at once; the next waits for a new one.
        submit(&mut host, 16, 1, true, 32, [0; 8], &[]).await.unwrap();
        assert_eq!(reply_in(&mut host).await.unwrap(), (RET_SUBMIT, 16, 0, vec![1, 2, 3]));
        submit(&mut host, 18, 1, true, 32, [0; 8], &[]).await.unwrap();
        // An OUT goes past it to the gadget.
        submit(&mut host, 20, 1, false, 3, [0; 8], &[0, 8, 9]).await.unwrap();
        let (cmd, seq, status, _) = reply(&mut host).await.unwrap();
        assert_eq!((cmd, seq, status), (RET_SUBMIT, 20, 0));
        reports.send(vec![4, 5]).unwrap();
        assert_eq!(reply_in(&mut host).await.unwrap(), (RET_SUBMIT, 18, 0, vec![4, 5]));
        // Reports while none waits: the next IN has the newest at once.
        reports.send(vec![6]).unwrap();
        reports.send(vec![7]).unwrap();
        tokio::task::yield_now().await;
        submit(&mut host, 22, 1, true, 32, [0; 8], &[]).await.unwrap();
        assert_eq!(reply_in(&mut host).await.unwrap().3, [7]);

        // Unlinking a waiting IN undoes it; one on a silent endpoint too; an answered one is past undoing.
        submit(&mut host, 24, 1, true, 32, [0; 8], &[]).await.unwrap();
        submit(&mut host, 26, 2, true, 32, [0; 8], &[]).await.unwrap();
        unlink(&mut host, 28, 24).await.unwrap();
        assert_eq!(reply(&mut host).await.unwrap(), (RET_UNLINK, 28, ECONNRESET, vec![]));
        unlink(&mut host, 30, 26).await.unwrap();
        assert_eq!(reply(&mut host).await.unwrap(), (RET_UNLINK, 30, ECONNRESET, vec![]));
        unlink(&mut host, 32, 22).await.unwrap();
        assert_eq!(reply(&mut host).await.unwrap(), (RET_UNLINK, 32, 0, vec![]));
        // The undone IN never answers: the next report goes to a new one.
        reports.send(vec![8]).unwrap();
        submit(&mut host, 34, 1, true, 32, [0; 8], &[]).await.unwrap();
        assert_eq!(reply_in(&mut host).await.unwrap(), (RET_SUBMIT, 34, 0, vec![8]));

        drop(host);
        let tiny = served.await.unwrap();
        assert_eq!(tiny.outs, [(1, vec![0, 8, 9])]);
    }
}
