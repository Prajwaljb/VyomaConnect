//! The presenter's pointer (stock `kdeconnect.presenter`), what every PC side shares: the packet
//! read as an order, the dot's place on a screen, and the dot itself (Apple's system red with a white
//! rim in a soft glow), so it looks the same on Linux, the Mac and Windows. Each side shows it in its
//! own overlay window.
//!
//! `{dx, dy}` moves it (the screen is -1..1 across, dy scaled by the aspect, as kdeconnectd does),
//! `{stop: true}` hides it, and so does a second without word (the phone keeps sending while it's held).

use std::time::Duration;

use serde_json::Value;

pub const PRESENTER: &str = "kdeconnect.presenter";
/// The dot's square, in logical pixels: a 9 px core in a soft glow.
pub const SIZE: i32 = 48;
/// Gone this long without a packet: the phone stopped pointing.
pub const QUIET: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Order {
    Move(f64, f64),
    Stop,
}

impl Order {
    /// A packet's [body] as an order; None for one that says nothing usable.
    pub fn of(body: &Value) -> Option<Order> {
        if body["stop"].as_bool() == Some(true) {
            return Some(Order::Stop);
        }
        let (dx, dy) = (number(&body["dx"])?, number(&body["dy"])?);
        Some(Order::Move(dx.clamp(-2.0, 2.0), dy.clamp(-2.0, 2.0)))
    }
}

/// Peers send numbers either way (see the protocol notes).
fn number(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str()?.parse().ok()).filter(|f: &f64| f.is_finite())
}

/// The dot's position, -1..1 each way from the middle of the screen, y growing downward.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    pub x: f64,
    pub y: f64,
}

impl Spot {
    pub const CENTRE: Spot = Spot { x: 0.0, y: 0.0 };

    pub fn moved(self, dx: f64, dy: f64, (w, h): (i32, i32)) -> Spot {
        let aspect = if h > 0 { w as f64 / h as f64 } else { 1.0 };
        Spot { x: (self.x + dx).clamp(-1.0, 1.0), y: (self.y + dy * aspect).clamp(-1.0, 1.0) }
    }

    /// The top-left of a [side]-pixel square around the spot on a [w]×[h] screen, kept on it.
    pub fn corner(self, (w, h): (i32, i32), side: i32) -> (i32, i32) {
        let at = |f: f64, span: i32| (((f + 1.0) / 2.0 * span as f64).round() as i32 - side / 2).clamp(0, (span - side).max(0));
        (at(self.x, w), at(self.y, h))
    }
}

/// The dot, [px] square, as premultiplied BGRA (little-endian ARGB32).
pub fn paint_dot(canvas: &mut [u8], px: usize) {
    let c = px as f32 / 2.0;
    let unit = px as f32 / SIZE as f32;
    let (core, rim, glow) = (4.5 * unit, 6.0 * unit, c);
    for (i, p) in canvas.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = ((i % px) as f32 + 0.5 - c, (i / px) as f32 + 0.5 - c);
        let d = (x * x + y * y).sqrt();
        // Edge coverage over one pixel, for smooth circles.
        let inside = |r: f32| (r - d + 0.5).clamp(0.0, 1.0);
        // Apple's system red, the colour a presenter's laser is expected to be.
        let (red, green, blue) = (1.0, 0.231, 0.188);
        let halo = ((1.0 - (d - rim).max(0.0) / (glow - rim)).clamp(0.0, 1.0)).powi(2) * 0.35;
        let (mut a, mut r, mut g, mut b) = (halo, red * halo, green * halo, blue * halo);
        let ring = inside(rim);
        (a, r, g, b) = (ring + a * (1.0 - ring), ring + r * (1.0 - ring), ring + g * (1.0 - ring), ring + b * (1.0 - ring));
        let dot = inside(core);
        (a, r, g, b) = (dot + a * (1.0 - dot), red * dot + r * (1.0 - dot), green * dot + g * (1.0 - dot), blue * dot + b * (1.0 - dot));
        let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        *p = [byte(b), byte(g), byte(r), byte(a)];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn moves_like_kdeconnectd() {
        let screen = (1920, 1080);
        let s = Spot::CENTRE.moved(0.5, 0.1, screen);
        assert_eq!(s.x, 0.5);
        assert!((s.y - 0.1 * 1920.0 / 1080.0).abs() < 1e-9);
        assert_eq!(Spot::CENTRE.moved(5.0, -5.0, screen), Spot { x: 1.0, y: -1.0 });
        assert_eq!(Spot::CENTRE.corner(screen, SIZE), (960 - SIZE / 2, 540 - SIZE / 2));
        assert_eq!(Spot { x: 1.0, y: -1.0 }.corner(screen, SIZE), (1920 - SIZE, 0));
    }

    #[test]
    fn reads_what_phones_send() {
        assert_eq!(Order::of(&json!({ "dx": 0.1, "dy": "-0.25" })), Some(Order::Move(0.1, -0.25)));
        assert_eq!(Order::of(&json!({ "dx": 9, "dy": 0 })), Some(Order::Move(2.0, 0.0)), "a wild jump is held back");
        assert_eq!(Order::of(&json!({ "stop": true })), Some(Order::Stop));
        assert_eq!(Order::of(&json!({ "dx": 0.1 })), None);
    }

    #[test]
    fn the_dot_is_opaque_in_the_middle_and_clear_at_the_corners() {
        let px = SIZE as usize;
        let mut canvas = vec![0u8; px * px * 4];
        paint_dot(&mut canvas, px);
        let at = |x: usize, y: usize| &canvas[(y * px + x) * 4..][..4];
        assert_eq!(at(px / 2, px / 2), [48, 59, 255, 255], "red, opaque");
        assert_eq!(at(0, 0)[3], 0);
    }
}
