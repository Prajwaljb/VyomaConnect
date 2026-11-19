//! A wired Xbox 360 controller in software (a [usbip] gadget): what Windows' own driver (xusb22)
//! binds to by its ids (045e:028e), so every XInput game, Steam and SDL see a controller. The
//! descriptors are the real pad's (its four vendor interfaces: the pad, the headset port, the
//! plug-in module and the security one); its reports carry the phone's [State]; the game's rumble
//! comes back on the OUT endpoint.
//!
//! Input report (endpoint 0x81, 20 bytes): 0x00, 0x14, buttons (u16 LE: up down left right, start
//! back, stick clicks, LB RB, guide, -, A B X Y), the triggers 0..255, the sticks as i16 LE with up
//! positive. Rumble (endpoint 0x01): 0x00, 0x08, 0x00, the big (low) motor, the small (high) one
//! 0..255, then three zeros. The LED pattern (0x01, 0x03, n) is the player number, unused here.

use tokio::sync::mpsc;

use crate::gamepad::{Button, State};
use crate::usbip::{Gadget, Setup};

/// The real pad's device descriptor (USB 2.0, vendor class, 8-byte EP0), without a serial: each pad
/// is known by its port, so plugging them in over and over leaves no trail of devices behind.
const DEVICE: [u8; 18] = [18, 1, 0x00, 0x02, 0xff, 0xff, 0xff, 8, 0x5e, 0x04, 0x8e, 0x02, 0x14, 0x01, 1, 2, 0, 1];

/// The configuration: 153 bytes, four interfaces, bus powered at 500 mA, as the real pad.
const CONFIGURATION: [u8; 153] = [
    9, 2, 153, 0, 4, 1, 0, 0xa0, 0xfa,
    // 0: the pad (ff/5d/01), its unknown 0x21 descriptor, IN 0x81 every 4 ms, OUT 0x01 every 8.
    9, 4, 0, 0, 2, 0xff, 0x5d, 0x01, 0,
    17, 0x21, 0x00, 0x01, 0x01, 0x25, 0x81, 0x14, 0x00, 0x00, 0x00, 0x00, 0x13, 0x01, 0x08, 0x00, 0x00,
    7, 5, 0x81, 3, 32, 0, 4,
    7, 5, 0x01, 3, 32, 0, 8,
    // 1: the headset port (ff/5d/03).
    9, 4, 1, 0, 4, 0xff, 0x5d, 0x03, 0,
    27, 0x21, 0x00, 0x01, 0x01, 0x01, 0x82, 0x40, 0x01, 0x02, 0x20, 0x16, 0x83, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x16, 0x03, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00,
    7, 5, 0x82, 3, 32, 0, 2,
    7, 5, 0x02, 3, 32, 0, 4,
    7, 5, 0x83, 3, 32, 0, 64,
    7, 5, 0x03, 3, 32, 0, 16,
    // 2: the plug-in module port (ff/5d/02).
    9, 4, 2, 0, 1, 0xff, 0x5d, 0x02, 0,
    9, 0x21, 0x00, 0x01, 0x01, 0x22, 0x84, 0x07, 0x00,
    7, 5, 0x84, 3, 32, 0, 16,
    // 3: security (ff/fd/13), no endpoints.
    9, 4, 3, 0, 0, 0xff, 0xfd, 0x13, 4,
    6, 0x41, 0x00, 0x01, 0x01, 0x03,
];

const STRINGS: [&str; 5] = ["", "Vyoma", "Vyoma Controller", "", "Xbox Security Method 3, Version 1.00, \u{a9} 2005 Microsoft Corporation. All rights reserved."];

/// The capabilities XInputGetCapabilities reads (vendor request 0xc1/0x01, value 0x0100): every
/// button, both triggers and sticks at full range, both motors.
const CAPABILITIES: [u8; 20] = [0x00, 0x14, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f, 0xff, 0x7f, 0xff, 0x7f, 0xff, 0x7f, 0xff, 0xff, 0xff, 0xff, 0x00, 0x00];

/// XInput's button bits, in [Button] order.
const BITS: [(Button, u16); 15] = [
    (Button::Up, 0x0001),
    (Button::Down, 0x0002),
    (Button::Left, 0x0004),
    (Button::Right, 0x0008),
    (Button::Start, 0x0010),
    (Button::Back, 0x0020),
    (Button::Ls, 0x0040),
    (Button::Rs, 0x0080),
    (Button::Lb, 0x0100),
    (Button::Rb, 0x0200),
    (Button::Guide, 0x0400),
    (Button::A, 0x1000),
    (Button::B, 0x2000),
    (Button::X, 0x4000),
    (Button::Y, 0x8000),
];

/// The pad's report endpoint.
pub const REPORT_ENDPOINT: u8 = 1;

/// The report for [s] (the phone's sticks have y down, XInput's up).
pub fn report(s: &State) -> Vec<u8> {
    let buttons = BITS.iter().filter(|(b, _)| s.pressed(*b as u32)).fold(0u16, |acc, (_, bit)| acc | bit);
    let up = |v: i32| (-v).clamp(-32768, 32767) as i16;
    let mut r = vec![0x00, 0x14];
    r.extend_from_slice(&buttons.to_le_bytes());
    r.push(s.lt.clamp(0, 255) as u8);
    r.push(s.rt.clamp(0, 255) as u8);
    for v in [s.lx.clamp(-32768, 32767) as i16, up(s.ly), s.rx.clamp(-32768, 32767) as i16, up(s.ry)] {
        r.extend_from_slice(&v.to_le_bytes());
    }
    r.extend_from_slice(&[0; 6]);
    r
}

/// The game's rumble as the phone takes it: strong, weak (0..65535), until the next.
pub fn rumble(out: &[u8]) -> Option<(u16, u16, u16)> {
    (out.len() >= 5 && out[0] == 0x00 && out[1] == 0x08).then(|| (out[3] as u16 * 257, out[4] as u16 * 257, 0))
}

/// The pad as a gadget; rumble goes to [rumbles].
pub struct Xbox360 {
    rumbles: mpsc::UnboundedSender<(u16, u16, u16)>,
}

impl Xbox360 {
    pub fn new(rumbles: mpsc::UnboundedSender<(u16, u16, u16)>) -> Xbox360 {
        Xbox360 { rumbles }
    }
}

impl Gadget for Xbox360 {
    fn device(&self) -> &[u8] {
        &DEVICE
    }

    fn configuration(&self) -> &[u8] {
        &CONFIGURATION
    }

    fn string(&self, index: u8) -> Option<&str> {
        STRINGS.get(index as usize).copied().filter(|s| !s.is_empty())
    }

    fn control(&mut self, s: &Setup, _: &[u8]) -> Vec<u8> {
        if (s.request_type, s.request, s.value) == (0xc1, 0x01, 0x0100) { CAPABILITIES.to_vec() } else { Vec::new() }
    }

    fn out(&mut self, ep: u8, data: &[u8]) {
        if ep == 1
            && let Some(r) = rumble(data)
        {
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
    use tokio::sync::watch;

    /// Walks a configuration descriptor: (type, bytes) of each descriptor in it.
    fn walk(c: &[u8]) -> Vec<(u8, &[u8])> {
        let mut out = Vec::new();
        let mut at = 0;
        while at < c.len() {
            let len = c[at] as usize;
            assert!(len >= 2 && at + len <= c.len(), "a descriptor runs past the end at {at}");
            out.push((c[at + 1], &c[at..at + len]));
            at += len;
        }
        out
    }

    #[test]
    fn the_descriptors_hold_together() {
        assert_eq!(u16::from_le_bytes([CONFIGURATION[2], CONFIGURATION[3]]) as usize, CONFIGURATION.len(), "wTotalLength");
        let all = walk(&CONFIGURATION);
        let interfaces: Vec<_> = all.iter().filter(|(t, _)| *t == 4).collect();
        assert_eq!(interfaces.len(), CONFIGURATION[4] as usize);
        // Each interface's endpoints follow it, as many as it says.
        for (i, (_, d)) in interfaces.iter().enumerate() {
            assert_eq!(d[2] as usize, i, "numbered in order");
            let at = all.iter().position(|(_, x)| std::ptr::eq(*x, *d)).unwrap();
            let endpoints = all[at + 1..].iter().take_while(|(t, _)| *t != 4).filter(|(t, _)| *t == 5).count();
            assert_eq!(endpoints, d[4] as usize, "interface {i}'s endpoints");
        }
        assert_eq!(&interfaces[0].1[5..8], [0xff, 0x5d, 0x01], "the pad's class, as xusb22 matches it");
        assert_eq!((DEVICE[0], DEVICE[8..12].to_vec(), DEVICE[16]), (18, vec![0x5e, 0x04, 0x8e, 0x02], 0), "045e:028e, no serial");
    }

    #[test]
    fn reports_the_phone_as_xinput_has_it() {
        let s = State { buttons: 1 << Button::A as u32 | 1 << Button::Up as u32 | 1 << Button::Guide as u32, lx: -32768, ly: -32768, rx: 1000, ry: 32767, lt: 255, rt: 9, ..Default::default() };
        let r = report(&s);
        assert_eq!(r.len(), 20);
        assert_eq!(&r[..6], [0x00, 0x14, 0x01, 0x14, 255, 9]);
        let axis = |at: usize| i16::from_le_bytes([r[at], r[at + 1]]);
        assert_eq!((axis(6), axis(8), axis(10), axis(12)), (-32768, 32767, 1000, -32767), "y turned up");
        assert_eq!(report(&State::default()), [0x00, 0x14, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        for (b, bit) in BITS {
            let r = report(&State { buttons: 1 << b as u32, ..Default::default() });
            assert_eq!(u16::from_le_bytes([r[2], r[3]]), bit);
        }
    }

    #[test]
    fn reads_the_games_rumble() {
        assert_eq!(rumble(&[0x00, 0x08, 0x00, 255, 128, 0, 0, 0]), Some((65535, 32896, 0)));
        assert_eq!(rumble(&[0x01, 0x03, 0x06]), None, "the LED");
    }

    /// As Windows meets a new pad: descriptors, configuration, capabilities, the LED, a report, a rumble.
    #[tokio::test]
    async fn a_host_drives_the_pad() {
        let (mut host, mut dev) = tokio::io::duplex(1 << 16);
        let (states, rx) = watch::channel(report(&State::default()));
        let (tx, mut rumbles) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut pad = Xbox360::new(tx);
            assert!(handshake(&mut dev, "1-1", &pad).await.unwrap());
            serve(dev, &mut pad, rx).await.unwrap();
        });
        let summary = import(&mut host, "1-1").await.unwrap();
        assert_eq!(&summary[300..304], [0x04, 0x5e, 0x02, 0x8e]);
        submit(&mut host, 2, 0, true, 18, get_descriptor(1, 0, 18), &[]).await.unwrap();
        assert_eq!(reply_in(&mut host).await.unwrap().3, DEVICE);
        submit(&mut host, 4, 0, true, 255, get_descriptor(2, 0, 255), &[]).await.unwrap();
        assert_eq!(reply_in(&mut host).await.unwrap().3, CONFIGURATION);
        submit(&mut host, 6, 0, true, 255, get_descriptor(3, 2, 255), &[]).await.unwrap();
        assert_eq!(String::from_utf16(&reply_in(&mut host).await.unwrap().3[2..].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>()).unwrap(), "Vyoma Controller");
        submit(&mut host, 8, 0, false, 0, [0, 9, 1, 0, 0, 0, 0, 0], &[]).await.unwrap();
        assert_eq!(reply(&mut host).await.unwrap().2, 0);
        submit(&mut host, 10, 0, true, 20, [0xc1, 1, 0x00, 0x01, 0, 0, 20, 0], &[]).await.unwrap();
        assert_eq!(reply_in(&mut host).await.unwrap().3, CAPABILITIES);
        submit(&mut host, 12, 1, false, 3, [0; 8], &[0x01, 0x03, 0x06]).await.unwrap();
        assert_eq!(reply(&mut host).await.unwrap().2, 0);

        submit(&mut host, 14, 1, true, 32, [0; 8], &[]).await.unwrap();
        assert_eq!(reply_in(&mut host).await.unwrap().3, report(&State::default()), "resting at first");
        submit(&mut host, 16, 1, true, 32, [0; 8], &[]).await.unwrap();
        let pressed = State { buttons: 1, ..Default::default() };
        states.send(report(&pressed)).unwrap();
        assert_eq!(reply_in(&mut host).await.unwrap().3, report(&pressed));

        submit(&mut host, 18, 1, false, 8, [0; 8], &[0x00, 0x08, 0x00, 200, 40, 0, 0, 0]).await.unwrap();
        assert_eq!(reply(&mut host).await.unwrap().2, 0);
        assert_eq!(rumbles.recv().await, Some((200 * 257, 40 * 257, 0)));
    }
}
