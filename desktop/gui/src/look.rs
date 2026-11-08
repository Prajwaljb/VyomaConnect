//! The window's colours from the PC's wallpaper, by the phone app's formulas (designsystem
//! `Theme.kt` and `Glass.kt`), so both show the same accent and the same soft canvas.

use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

/// An RGB colour, 0..1 a channel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgb(pub f32, pub f32, pub f32);

impl Rgb {
    pub fn of(c: [u8; 3]) -> Rgb {
        Rgb(c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0)
    }

    pub fn slint(self) -> slint::Color {
        let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        slint::Color::from_rgb_u8(b(self.0), b(self.1), b(self.2))
    }

    fn lerp(self, o: Rgb, t: f32) -> Rgb {
        Rgb(self.0 + (o.0 - self.0) * t, self.1 + (o.1 - self.1) * t, self.2 + (o.2 - self.2) * t)
    }

    /// HSL saturation and lightness.
    fn sl(self) -> (f32, f32) {
        let (max, min) = (self.0.max(self.1).max(self.2), self.0.min(self.1).min(self.2));
        let l = (max + min) / 2.0;
        let s = if max == min { 0.0 } else { (max - min) / (1.0 - (2.0 * l - 1.0).abs()) };
        (s, l)
    }

    /// The same hue, its saturation scaled by [saturation] and, if given, its [lightness] set.
    pub fn toned(self, saturation: f32, lightness: Option<f32>) -> Rgb {
        let Rgb(r, g, b) = self;
        let (max, min) = (r.max(g).max(b), r.min(g).min(b));
        let l = (max + min) / 2.0;
        if max == min {
            return lightness.map_or(self, |v| Rgb(v, v, v));
        }
        let d = max - min;
        let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
        let h = if max == r {
            (g - b) / d + if g < b { 6.0 } else { 0.0 }
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        } / 6.0;
        let s2 = (s * saturation).min(1.0);
        let l2 = lightness.unwrap_or(l);
        let q = if l2 < 0.5 { l2 * (1.0 + s2) } else { l2 + s2 - l2 * s2 };
        let p = 2.0 * l2 - q;
        let hue = |t0: f32| {
            let t = (t0 + 1.0) % 1.0;
            if t < 1.0 / 6.0 {
                p + (q - p) * 6.0 * t
            } else if t < 0.5 {
                q
            } else if t < 2.0 / 3.0 {
                p + (q - p) * (2.0 / 3.0 - t) * 6.0
            } else {
                p
            }
        };
        Rgb(hue(h + 1.0 / 3.0), hue(h), hue(h - 1.0 / 3.0))
    }
}

/// The PC's wallpaper as a grid of colours, rows top first.
#[derive(Clone, Debug, PartialEq)]
pub struct Wallpaper {
    pub cols: usize,
    pub rows: usize,
    pub colors: Vec<[u8; 3]>,
}

impl Wallpaper {
    /// Its most vivid colour (saturated and mid-light, so neither a grey sky nor a black corner
    /// wins), bold enough for controls, at the theme's lightness. None for a wallpaper of greys.
    pub fn accent(&self, dark: bool) -> Option<Rgb> {
        let mut best = None;
        let mut best_score = 0.1;
        for &c in &self.colors {
            let col = Rgb::of(c);
            let (s, l) = col.sl();
            let score = s * (1.0 - (l - 0.5).abs() * 1.4).max(0.0);
            if score > best_score {
                best_score = score;
                best = Some(col);
            }
        }
        let c = best?;
        let (s, _) = c.sl();
        Some(c.toned((0.72 / s).clamp(1.0, 3.0), Some(if dark { 0.7 } else { 0.47 })))
    }

    /// The canvas: the grid toned towards the theme's base, stretched smoothly (bilinear, cell
    /// centres on the grid's points) to [w]×[h]. The fade to the base lower down is the window's.
    pub fn canvas(&self, dark: bool, w: usize, h: usize) -> Image {
        let base = if dark { Rgb(0.0, 0.0, 0.0) } else { Rgb::of([0xED, 0xEE, 0xF2]) };
        let toned: Vec<Rgb> = self.colors.iter().map(|&c| base.lerp(Rgb::of(c).toned(1.25, None), if dark { 0.42 } else { 0.32 })).collect();
        let at = |x: usize, y: usize| toned[y.min(self.rows - 1) * self.cols + x.min(self.cols - 1)];
        let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(w as u32, h as u32);
        let px = buf.make_mut_slice();
        for y in 0..h {
            let gy = ((y as f32 + 0.5) / h as f32 * self.rows as f32 - 0.5).clamp(0.0, (self.rows - 1) as f32);
            let (y0, ty) = (gy.floor() as usize, gy.fract());
            for x in 0..w {
                let gx = ((x as f32 + 0.5) / w as f32 * self.cols as f32 - 0.5).clamp(0.0, (self.cols - 1) as f32);
                let (x0, tx) = (gx.floor() as usize, gx.fract());
                let top = at(x0, y0).lerp(at(x0 + 1, y0), tx);
                let bottom = at(x0, y0 + 1).lerp(at(x0 + 1, y0 + 1), tx);
                let c = top.lerp(bottom, ty).slint();
                px[y * w + x] = Rgba8Pixel::new(c.red(), c.green(), c.blue(), 255);
            }
        }
        Image::from_rgba8(buf)
    }
}

/// Sets the window's accent and canvas from [wall], or back to the defaults without one.
pub fn apply(theme: &crate::Theme, wall: Option<&Wallpaper>) {
    let valid = wall.filter(|w| w.cols > 0 && w.rows > 0 && w.colors.len() == w.cols * w.rows);
    let (light, dark) = match valid.map(|w| (w.accent(false), w.accent(true))) {
        Some((Some(l), Some(d))) => (l, d),
        _ => (Rgb::of([0x2F, 0x6F, 0xDE]), Rgb::of([0x7D, 0xB0, 0xFF])),
    };
    theme.set_accent_light(light.slint());
    theme.set_accent_dark(dark.slint());
    // Filled controls: a deep shade of the accent under white ink (the phone's Glass.accent).
    theme.set_fill_light(light.toned(1.2, Some(0.5)).slint());
    theme.set_fill_dark(dark.toned(1.2, Some(0.56)).slint());
    theme.set_has_wall(valid.is_some());
    if let Some(w) = valid {
        theme.set_wall_light(w.canvas(false, 96, 72));
        theme.set_wall_dark(w.canvas(true, 96, 72));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_vivid_colour_and_skips_greys() {
        let greys = Wallpaper { cols: 2, rows: 1, colors: vec![[128, 128, 128], [20, 20, 20]] };
        assert_eq!(greys.accent(false), None);
        let wall = Wallpaper { cols: 3, rows: 1, colors: vec![[128, 128, 128], [200, 90, 30], [10, 10, 12]] };
        let a = wall.accent(false).unwrap();
        assert!(a.0 > a.1 && a.1 > a.2, "stays orange: {a:?}");
        assert!((a.sl().1 - 0.47).abs() < 0.01);
        assert!((wall.accent(true).unwrap().sl().1 - 0.7).abs() < 0.01);
        let back = Rgb(0.2, 0.5, 0.8).toned(1.0, None);
        assert!((back.0 - 0.2).abs() < 1e-4 && (back.1 - 0.5).abs() < 1e-4 && (back.2 - 0.8).abs() < 1e-4);
    }
}
