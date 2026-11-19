//! A USB DualSense in software (a [usbip] gadget): what Windows' HID driver takes for a PS5
//! controller by its ids (054c:0ce6), so games and libraries that know one (SDL, Steam, games with
//! their own PlayStation support) see a DualSense with its own button names. One HID interface (the
//! real pad's report descriptor), reports streamed as the real pad does, the game's rumble back.
//!
//! Input report 0x01 (64 bytes, endpoint 0x84): the sticks 0..255 (128 the middle, y down), the
//! triggers, a counter, the d-pad as a hat (0 up, clockwise, 8 none) with square cross circle
//! triangle above it, then L1 R1 L2 R2 create options L3 R3, then PS; gyro, accelerometer (at rest),
//! a timestamp in 1/3 µs, the touch points (none), battery full on USB. Output report 0x02 (endpoint
//! 0x03 or SET_REPORT): flags, then the small (high) and large (low) motors 0..255. Feature reports
//! a host reads: 0x05 the motion sensors' calibration, 0x09 the pad's address (its serial for SDL
//! and Steam), 0x20 the firmware, 0x81 the answer to a 0x80 command (the serial).

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use tokio::sync::{mpsc, watch};

use crate::gamepad::{Button, State};
use crate::usbip::{Gadget, Setup};

/// The pad's report descriptor, as the real one's (273 bytes).
pub const REPORT_DESCRIPTOR: [u8; 273] = [
    0x05, 0x01, 0x09, 0x05, 0xa1, 0x01, // Generic Desktop, Game Pad, Application
    0x85, 0x01, // report 1: input
    0x09, 0x30, 0x09, 0x31, 0x09, 0x32, 0x09, 0x35, 0x09, 0x33, 0x09, 0x34, // X Y Z Rz Rx Ry
    0x15, 0x00, 0x26, 0xff, 0x00, 0x75, 0x08, 0x95, 0x06, 0x81, 0x02, // 6 × 0..255
    0x06, 0x00, 0xff, 0x09, 0x20, 0x95, 0x01, 0x81, 0x02, // the counter
    0x05, 0x01, 0x09, 0x39, 0x15, 0x00, 0x25, 0x07, 0x35, 0x00, 0x46, 0x3b, 0x01, 0x65, 0x14, 0x75, 0x04, 0x95, 0x01, 0x81, 0x42, 0x65, 0x00, // hat
    0x05, 0x09, 0x19, 0x01, 0x29, 0x0f, 0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x95, 0x0f, 0x81, 0x02, // 15 buttons
    0x06, 0x00, 0xff, 0x09, 0x21, 0x95, 0x0d, 0x81, 0x02, // 13 bits
    0x06, 0x00, 0xff, 0x09, 0x22, 0x15, 0x00, 0x26, 0xff, 0x00, 0x75, 0x08, 0x95, 0x34, 0x81, 0x02, // 52 bytes: motion, touch, status
    0x85, 0x02, 0x09, 0x23, 0x95, 0x2f, 0x91, 0x02, // report 2: output, 47 bytes
    0x85, 0x05, 0x09, 0x33, 0x95, 0x28, 0xb1, 0x02, // features: id, usage, bytes
    0x85, 0x08, 0x09, 0x34, 0x95, 0x2f, 0xb1, 0x02,
    0x85, 0x09, 0x09, 0x24, 0x95, 0x13, 0xb1, 0x02,
    0x85, 0x0a, 0x09, 0x25, 0x95, 0x1a, 0xb1, 0x02,
    0x85, 0x20, 0x09, 0x26, 0x95, 0x3f, 0xb1, 0x02,
    0x85, 0x21, 0x09, 0x27, 0x95, 0x04, 0xb1, 0x02,
    0x85, 0x22, 0x09, 0x40, 0x95, 0x3f, 0xb1, 0x02,
    0x85, 0x80, 0x09, 0x28, 0x95, 0x3f, 0xb1, 0x02,
    0x85, 0x81, 0x09, 0x29, 0x95, 0x3f, 0xb1, 0x02,
    0x85, 0x82, 0x09, 0x2a, 0x95, 0x09, 0xb1, 0x02,
    0x85, 0x83, 0x09, 0x2b, 0x95, 0x3f, 0xb1, 0x02,
    0x85, 0x84, 0x09, 0x2c, 0x95, 0x3f, 0xb1, 0x02,
    0x85, 0x85, 0x09, 0x2d, 0x95, 0x02, 0xb1, 0x02,
    0x85, 0xa0, 0x09, 0x2e, 0x95, 0x01, 0xb1, 0x02,
    0x85, 0xe0, 0x09, 0x2f, 0x95, 0x3f, 0xb1, 0x02,
    0x85, 0xf0, 0x09, 0x30, 0x95, 0x3f, 0xb1, 0x02,
    0x85, 0xf1, 0x09, 0x31, 0x95, 0x3f, 0xb1, 0x02,
    0x85, 0xf2, 0x09, 0x32, 0x95, 0x0f, 0xb1, 0x02,
    0x85, 0xf4, 0x09, 0x35, 0x95, 0x3f, 0xb1, 0x02,
    0x85, 0xf5, 0x09, 0x36, 0x95, 0x03, 0xb1, 0x02,
    0xc0,
];

/// USB 2.0, defined per interface, 64-byte EP0, 054c:0ce6 release 1.00, no serial.
const DEVICE: [u8; 18] = [18, 1, 0x00, 0x02, 0, 0, 0, 64, 0x4c, 0x05, 0xe6, 0x0c, 0x00, 0x01, 1, 2, 0, 1];

/// One HID interface (its HID descriptor naming the report descriptor), IN 0x84 and OUT 0x03 each
/// millisecond, 64 bytes.
const CONFIGURATION: [u8; 41] = [
    9, 2, 41, 0, 1, 1, 0, 0xc0, 0xfa,
    9, 4, 0, 0, 2, 0x03, 0x00, 0x00, 0,
    9, 0x21, 0x11, 0x01, 0x00, 1, 0x22, (REPORT_DESCRIPTOR.len() & 0xff) as u8, (REPORT_DESCRIPTOR.len() >> 8) as u8,
    7, 5, 0x84, 3, 64, 0, 1,
    7, 5, 0x03, 3, 64, 0, 1,
];

const STRINGS: [&str; 3] = ["", "Vyoma", "Vyoma Controller (DualSense)"];

/// The report endpoint.
pub const REPORT_ENDPOINT: u8 = 4;
/// How often the real pad sends a report over USB (250 a second).
pub const REPORT_EVERY: std::time::Duration = std::time::Duration::from_millis(4);
const REPORT_LEN: usize = 64;

/// The face buttons and the rest in the report's bits: byte 8 (above the hat), 9, 10.
const FACE: [(Button, u8); 4] = [(Button::X, 0x10), (Button::A, 0x20), (Button::B, 0x40), (Button::Y, 0x80)];
const SHOULDERS: [(Button, u8); 6] = [(Button::Lb, 0x01), (Button::Rb, 0x02), (Button::Back, 0x10), (Button::Start, 0x20), (Button::Ls, 0x40), (Button::Rs, 0x80)];

/// The d-pad as the hat's direction: 0 up, clockwise to 7 up-left, 8 none.
fn hat(s: &State) -> u8 {
    match s.hat() {
        (0, -1) => 0,
        (1, -1) => 1,
        (1, 0) => 2,
        (1, 1) => 3,
        (0, 1) => 4,
        (-1, 1) => 5,
        (-1, 0) => 6,
        (-1, -1) => 7,
        _ => 8,
    }
}

/// A stick value as the pad has it (0..255, 128 the middle; y down, as the phone's).
fn stick(v: i32) -> u8 {
    ((v.clamp(-32768, 32767) + 32768) >> 8) as u8
}

/// Builds the stream of input reports: each with the next counter and the time.
pub struct Reports {
    counter: u8,
    started: Instant,
}

impl Default for Reports {
    fn default() -> Reports {
        Reports { counter: 0, started: Instant::now() }
    }
}

impl Reports {
    pub fn report(&mut self, s: &State) -> Vec<u8> {
        let mut r = vec![0u8; REPORT_LEN];
        r[0] = 0x01;
        (r[1], r[2], r[3], r[4]) = (stick(s.lx), stick(s.ly), stick(s.rx), stick(s.ry));
        (r[5], r[6]) = (s.lt.clamp(0, 255) as u8, s.rt.clamp(0, 255) as u8);
        self.counter = self.counter.wrapping_add(1);
        r[7] = self.counter;
        r[8] = hat(s) | FACE.iter().filter(|(b, _)| s.pressed(*b as u32)).fold(0, |acc, (_, bit)| acc | bit);
        r[9] = SHOULDERS.iter().filter(|(b, _)| s.pressed(*b as u32)).fold(0, |acc, (_, bit)| acc | bit)
            | if s.lt > 0 { 0x04 } else { 0 }
            | if s.rt > 0 { 0x08 } else { 0 };
        r[10] = s.pressed(Button::Guide as u32) as u8;
        // At rest, face up: 1 g on the accelerometer's z (8192 a g).
        r[26..28].copy_from_slice(&(-8192i16).to_le_bytes());
        let stamp = (self.started.elapsed().as_micros() as u64 * 3) as u32;
        r[28..32].copy_from_slice(&stamp.to_le_bytes());
        // Two touch points, neither touching.
        (r[33], r[37]) = (0x80, 0x80);
        // The adaptive triggers at rest.
        (r[42], r[43]) = (0x09, 0x09);
        r[49..53].copy_from_slice(&stamp.to_le_bytes());
        // Battery full, on USB.
        (r[53], r[54]) = (0x2a, 0x08);
        r
    }
}

/// The game's rumble from an output report: strong (the large motor), weak, until the next. None
/// when the report sets only lights or triggers.
pub fn rumble(out: &[u8]) -> Option<(u16, u16, u16)> {
    // Flags: 0x01 compatible vibration, 0x02 haptics select (how SDL and Windows' games say rumble).
    (out.len() >= 5 && out[0] == 0x02 && out[1] & 0x03 != 0).then(|| (out[4] as u16 * 257, out[3] as u16 * 257, 0))
}

/// Pads made so far: each its own address and serial (SDL and Steam tell pads apart by them).
static MADE: AtomicU32 = AtomicU32::new(0);

/// The pad as a gadget; rumble goes to [rumbles], GET_REPORT answers with the newest [reports].
pub struct DualSense {
    rumbles: mpsc::UnboundedSender<(u16, u16, u16)>,
    reports: watch::Receiver<Vec<u8>>,
    /// A locally administered address: 02:56:59 ("VY"), then the pad's number.
    address: [u8; 6],
    command: u8,
}

impl DualSense {
    pub fn new(rumbles: mpsc::UnboundedSender<(u16, u16, u16)>, reports: watch::Receiver<Vec<u8>>) -> DualSense {
        let n = MADE.fetch_add(1, Ordering::Relaxed).to_be_bytes();
        DualSense { rumbles, reports, address: [0x02, 0x56, 0x59, n[1], n[2], n[3]], command: 0 }
    }

    fn serial(&self) -> String {
        format!("VYOMA{:02X}{:02X}{:02X}", self.address[3], self.address[4], self.address[5])
    }

    /// Feature report [id], as the host reads it.
    fn feature(&self, id: u8) -> Vec<u8> {
        match id {
            // Calibration: the gyro's biases (none), its ranges, speeds, the accelerometer's ranges.
            0x05 => {
                let mut r = vec![0u8; 41];
                r[0] = id;
                let values: [i16; 17] = [0, 0, 0, 8192, -8192, 8192, -8192, 8192, -8192, 500, 500, 8192, -8192, 8192, -8192, 8192, -8192];
                for (i, v) in values.iter().enumerate() {
                    r[1 + i * 2..3 + i * 2].copy_from_slice(&v.to_le_bytes());
                }
                r[35] = 0x0b;
                r
            }
            // The pad's address, lowest byte first, then its pairing details.
            0x09 => {
                let mut r = vec![0u8; 20];
                r[0] = id;
                for i in 0..6 {
                    r[1 + i] = self.address[5 - i];
                }
                (r[7], r[8], r[10], r[12], r[13], r[14], r[15]) = (0x08, 0x25, 0x1e, 0xee, 0x74, 0xd0, 0xbc);
                r
            }
            // Firmware: build date and time, hardware type and info, version 0x0630.
            0x20 => {
                let mut r = vec![0u8; 64];
                r[0] = id;
                r[1..12].copy_from_slice(b"Jul 04 2025");
                r[12..20].copy_from_slice(b"10:10:32");
                (r[20], r[21], r[22]) = (0x03, 0x01, 0x44);
                r[24..28].copy_from_slice(&0x0100_0208u32.to_le_bytes());
                (r[28], r[31], r[32], r[33]) = (0x36, 0x01, 0xc1, 0xc8);
                r[44..46].copy_from_slice(&0x0630u16.to_le_bytes());
                (r[48], r[52], r[54], r[56]) = (0x14, 0x0b, 0x01, 0x06);
                r
            }
            // The answer to the last command: the serial when asked (1), else "done".
            0x81 => {
                let mut r = vec![0u8; 64];
                r[0] = id;
                if self.command == 0x01 {
                    let serial = self.serial();
                    r[3..3 + serial.len()].copy_from_slice(serial.as_bytes());
                } else {
                    r[1] = 0x01;
                }
                r
            }
            _ => vec![id],
        }
    }
}

impl Gadget for DualSense {
    fn device(&self) -> &[u8] {
        &DEVICE
    }

    fn configuration(&self) -> &[u8] {
        &CONFIGURATION
    }

    fn string(&self, index: u8) -> Option<&str> {
        STRINGS.get(index as usize).copied().filter(|s| !s.is_empty())
    }

    fn control(&mut self, s: &Setup, data: &[u8]) -> Vec<u8> {
        let (kind, id) = ((s.value >> 8) as u8, s.value as u8);
        match (s.request_type, s.request) {
            // GET_DESCRIPTOR from the interface: its HID descriptor, its report descriptor.
            (0x81, 6) if kind == 0x21 => CONFIGURATION[18..27].to_vec(),
            (0x81, 6) if kind == 0x22 => REPORT_DESCRIPTOR.to_vec(),
            // GET_REPORT: the newest input report, or a feature report.
            (0xa1, 0x01) if kind == 1 => self.reports.borrow().clone(),
            (0xa1, 0x01) if kind == 3 => self.feature(id),
            // GET_IDLE, GET_PROTOCOL (the report protocol).
            (0xa1, 0x02) => vec![0],
            (0xa1, 0x03) => vec![1],
            // SET_REPORT: an output report (rumble), or a command to answer later.
            (0x21, 0x09) => {
                if kind == 2 {
                    self.out(3, data);
                } else if kind == 3 && id == 0x80 && data.len() >= 2 {
                    self.command = data[1];
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn out(&mut self, _ep: u8, data: &[u8]) {
        if let Some(r) = rumble(data) {
            let _ = self.rumbles.send(r);
        }
    }

    fn report_endpoint(&self) -> u8 {
        REPORT_ENDPOINT
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usbip::host::*;
    use crate::usbip::{handshake, serve};

    /// Each report's size in bits by kind (0x80 input, 0x90 output, 0xb0 feature), from the
    /// descriptor's own items, as Windows' HID parser reads them.
    fn report_bits(d: &[u8]) -> std::collections::HashMap<(u8, u8), u32> {
        let (mut size, mut count, mut id) = (0u32, 0u32, 0u8);
        let mut bits = std::collections::HashMap::new();
        let mut at = 0;
        while at < d.len() {
            let prefix = d[at];
            let len = [0, 1, 2, 4][(prefix & 3) as usize];
            let value = d[at + 1..at + 1 + len].iter().rev().fold(0u32, |acc, b| acc << 8 | *b as u32);
            match prefix & 0xfc {
                0x74 => size = value,
                0x94 => count = value,
                0x84 => id = value as u8,
                main @ (0x80 | 0x90 | 0xb0) => *bits.entry((main, id)).or_default() += size * count,
                _ => {}
            }
            at += 1 + len;
        }
        bits
    }

    #[test]
    fn the_descriptor_says_what_the_reports_are() {
        let bits = report_bits(&REPORT_DESCRIPTOR);
        assert_eq!(bits[&(0x80, 0x01)], 63 * 8, "input 1: 64 bytes with its id");
        assert_eq!(bits[&(0x90, 0x02)], 47 * 8, "output 2");
        assert_eq!(bits[&(0xb0, 0x05)], 40 * 8, "calibration");
        assert_eq!(bits[&(0xb0, 0x09)], 19 * 8, "pairing");
        assert_eq!(bits[&(0xb0, 0x20)], 63 * 8, "firmware");
        assert_eq!(REPORT_DESCRIPTOR[REPORT_DESCRIPTOR.len() - 1], 0xc0, "the collection closes");
        assert_eq!(u16::from_le_bytes([CONFIGURATION[2], CONFIGURATION[3]]) as usize, CONFIGURATION.len());
        assert_eq!(&CONFIGURATION[18..20], [9, 0x21], "the HID descriptor in its place");
        let dummy = DualSense::new(mpsc::unbounded_channel().0, watch::channel(Vec::new()).1);
        assert_eq!(dummy.feature(0x05).len(), 41);
        assert_eq!(dummy.feature(0x09).len(), 20);
        assert_eq!(dummy.feature(0x20).len(), 64);
    }

    #[test]
    fn reports_the_phone_as_a_dualsense_has_it() {
        let mut reports = Reports::default();
        let s = State { buttons: 1 << Button::A as u32 | 1 << Button::Y as u32 | 1 << Button::Up as u32 | 1 << Button::Right as u32 | 1 << Button::Lb as u32 | 1 << Button::Start as u32 | 1 << Button::Guide as u32, lx: -32768, ly: 32767, rt: 200, ..Default::default() };
        let a = reports.report(&s);
        assert_eq!(a.len(), 64);
        assert_eq!(&a[..7], [0x01, 0, 255, 128, 128, 0, 200], "sticks with y down, the triggers");
        assert_eq!(a[8], 0x01 | 0x20 | 0x80, "up-right, cross, triangle");
        assert_eq!(a[9], 0x01 | 0x20 | 0x08, "L1, options, R2 pressed");
        assert_eq!(a[10], 0x01, "PS");
        assert_eq!((a[33], a[37], a[54]), (0x80, 0x80, 0x08), "no touches, on USB");
        let b = reports.report(&State::default());
        assert_eq!(b[7], a[7].wrapping_add(1), "counted");
        assert_eq!((b[1], b[8], b[9]), (128, 8, 0), "at rest: sticks centred, the hat at none");
        for (dx, dy, want) in [(0, -1, 0), (1, 0, 2), (0, 1, 4), (-1, 1, 5), (-1, 0, 6), (-1, -1, 7)] {
            let pressed = |b: Button, on: bool| if on { 1u32 << b as u32 } else { 0 };
            let s = State { buttons: pressed(Button::Up, dy < 0) | pressed(Button::Down, dy > 0) | pressed(Button::Left, dx < 0) | pressed(Button::Right, dx > 0), ..Default::default() };
            assert_eq!(hat(&s), want, "({dx}, {dy})");
        }
    }

    #[test]
    fn reads_the_games_rumble() {
        let mut out = vec![0u8; 48];
        out[0] = 0x02;
        out[1] = 0x03;
        (out[3], out[4]) = (10, 255);
        assert_eq!(rumble(&out), Some((65535, 2570, 0)), "large motor strong, small weak");
        out[1] = 0x04;
        assert_eq!(rumble(&out), None, "the right trigger's effect alone");
    }

    /// As Windows and SDL meet the pad: descriptors, the report descriptor, feature reports, a
    /// command, reports streaming, rumble by SET_REPORT and on the OUT endpoint.
    #[tokio::test]
    async fn a_host_drives_the_pad() {
        let (mut host, mut dev) = tokio::io::duplex(1 << 16);
        let mut builder = Reports::default();
        let (reports, rx) = watch::channel(builder.report(&State::default()));
        let (tx, mut rumbles) = mpsc::unbounded_channel();
        let gadget_rx = rx.clone();
        tokio::spawn(async move {
            let mut pad = DualSense::new(tx, gadget_rx);
            assert!(handshake(&mut dev, "1-1", &pad).await.unwrap());
            serve(dev, &mut pad, rx).await.unwrap();
        });
        let summary = import(&mut host, "1-1").await.unwrap();
        assert_eq!(&summary[300..304], [0x05, 0x4c, 0x0c, 0xe6]);
        submit(&mut host, 2, 0, true, 255, get_descriptor(2, 0, 255), &[]).await.unwrap();
        assert_eq!(reply_in(&mut host).await.unwrap().3, CONFIGURATION);
        submit(&mut host, 4, 0, true, 512, [0x81, 6, 0, 0x22, 0, 0, 0x11, 0x01], &[]).await.unwrap();
        assert_eq!(reply_in(&mut host).await.unwrap().3, REPORT_DESCRIPTOR);
        // SDL reads the pad's address (its serial) and firmware.
        submit(&mut host, 6, 0, true, 20, [0xa1, 1, 0x09, 3, 0, 0, 20, 0], &[]).await.unwrap();
        let pairing = reply_in(&mut host).await.unwrap().3;
        assert_eq!((pairing[0], pairing[4..7].to_vec()), (0x09, vec![0x59, 0x56, 0x02]), "02:56:59:… lowest byte first");
        submit(&mut host, 8, 0, true, 64, [0xa1, 1, 0x20, 3, 0, 0, 64, 0], &[]).await.unwrap();
        assert_eq!(u16::from_le_bytes(reply_in(&mut host).await.unwrap().3[44..46].try_into().unwrap()), 0x0630);
        // Steam's serial: a command, then its answer.
        submit(&mut host, 10, 0, false, 3, [0x21, 9, 0x80, 3, 0, 0, 3, 0], &[0x80, 0x01, 0x13]).await.unwrap();
        assert_eq!(reply(&mut host).await.unwrap().2, 0);
        submit(&mut host, 12, 0, true, 64, [0xa1, 1, 0x81, 3, 0, 0, 64, 0], &[]).await.unwrap();
        assert!(reply_in(&mut host).await.unwrap().3[3..].starts_with(b"VYOMA"));

        // Reports stream: each IN gets the next.
        submit(&mut host, 14, 4, true, 64, [0; 8], &[]).await.unwrap();
        let first = reply_in(&mut host).await.unwrap().3;
        submit(&mut host, 16, 4, true, 64, [0; 8], &[]).await.unwrap();
        reports.send(builder.report(&State { buttons: 1, ..Default::default() })).unwrap();
        let next = reply_in(&mut host).await.unwrap().3;
        assert_eq!((next[7], next[8] & 0xf0), (first[7].wrapping_add(1), 0x20), "counted, cross held");

        // Rumble on the OUT endpoint, then by SET_REPORT.
        let mut out = vec![0u8; 48];
        (out[0], out[1], out[3], out[4]) = (0x02, 0x03, 0, 128);
        submit(&mut host, 18, 3, false, 48, [0; 8], &out).await.unwrap();
        assert_eq!(reply(&mut host).await.unwrap().2, 0);
        assert_eq!(rumbles.recv().await, Some((128 * 257, 0, 0)));
        out[3] = 64;
        submit(&mut host, 20, 0, false, 48, [0x21, 9, 0x02, 2, 0, 0, 48, 0], &out).await.unwrap();
        assert_eq!(reply(&mut host).await.unwrap().2, 0);
        assert_eq!(rumbles.recv().await, Some((128 * 257, 64 * 257, 0)));
    }
}
