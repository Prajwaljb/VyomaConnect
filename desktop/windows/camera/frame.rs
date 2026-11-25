//! The picture the agent hands the camera: one NV12 frame in shared memory under a sequence lock
//! (the writer makes the count odd while it writes and even when done; a reader that sees it
//! change starts again), plus the conversions both sides need: turning the phone's picture
//! upright, fitting it into the size an app asked for, and the picture shown while no phone sends.
//! Plain Rust, no Windows calls, so it's tested everywhere.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

pub const MAGIC: u32 = u32::from_le_bytes(*b"VYCM");
/// The largest picture either way (a portrait 1080p turned upright is 1080 x 1920).
pub const MAX_SIDE: usize = 1920;
pub const HEADER: usize = 64;
/// The whole shared section: the header and the largest NV12 picture.
pub const SECTION_BYTES: usize = HEADER + MAX_SIDE * MAX_SIDE * 3 / 2;
/// A picture older than this (ms) isn't shown: the phone stopped without saying.
pub const STALE_MS: u64 = 2000;

// Header: magic u32 @0, live u32 @4, seq u64 @8, width u32 @16, height u32 @20, stamp u64 @24.
const LIVE: usize = 4;
const SEQ: usize = 8;
const WIDTH: usize = 16;
const HEIGHT: usize = 20;
const STAMP: usize = 24;

/// The section's header and picture, at [base] ([SECTION_BYTES] long, 8-byte aligned).
pub struct Shared {
    base: *mut u8,
}

// SAFETY: every access goes through atomics or the sequence lock.
unsafe impl Send for Shared {}
unsafe impl Sync for Shared {}

impl Shared {
    /// # Safety
    /// [base] must point at [SECTION_BYTES] bytes, 8-byte aligned, alive as long as this is.
    pub unsafe fn new(base: *mut u8) -> Shared {
        Shared { base }
    }

    fn u32_at(&self, at: usize) -> &AtomicU32 {
        // SAFETY: inside the header, aligned (see [new]).
        unsafe { &*(self.base.add(at) as *const AtomicU32) }
    }

    fn u64_at(&self, at: usize) -> &AtomicU64 {
        // SAFETY: as in [u32_at].
        unsafe { &*(self.base.add(at) as *const AtomicU64) }
    }

    /// The writer: one upright [w]×[h] NV12 picture (tightly packed), stamped [now_ms].
    pub fn write(&self, w: usize, h: usize, nv12: &[u8], now_ms: u64) {
        let (w, h) = (w.min(MAX_SIDE) & !1, h.min(MAX_SIDE) & !1);
        let len = (w * h * 3 / 2).min(nv12.len());
        let seq = self.u64_at(SEQ);
        let start = seq.load(Ordering::Relaxed) | 1;
        seq.store(start, Ordering::Relaxed);
        std::sync::atomic::fence(Ordering::Release);
        self.u32_at(0).store(MAGIC, Ordering::Relaxed);
        self.u32_at(WIDTH).store(w as u32, Ordering::Relaxed);
        self.u32_at(HEIGHT).store(h as u32, Ordering::Relaxed);
        self.u64_at(STAMP).store(now_ms, Ordering::Relaxed);
        // SAFETY: the picture area holds the largest picture; [len] is within it and the source.
        unsafe { std::ptr::copy_nonoverlapping(nv12.as_ptr(), self.base.add(HEADER), len) };
        seq.store(start + 1, Ordering::Release);
    }

    /// The writer: whether a phone is sending (the camera shows the placeholder otherwise).
    pub fn set_live(&self, live: bool) {
        self.u32_at(LIVE).store(live as u32, Ordering::Release);
    }

    /// The reader: the newest picture into [out] and its size, unless none is fresh at [now_ms].
    pub fn read(&self, out: &mut Vec<u8>, now_ms: u64) -> Option<(usize, usize)> {
        for _ in 0..4 {
            let seq = self.u64_at(SEQ);
            let before = seq.load(Ordering::Acquire);
            if before & 1 == 1 {
                std::thread::yield_now();
                continue;
            }
            if self.u32_at(0).load(Ordering::Relaxed) != MAGIC || self.u32_at(LIVE).load(Ordering::Acquire) == 0 {
                return None;
            }
            let (w, h) = (self.u32_at(WIDTH).load(Ordering::Relaxed) as usize, self.u32_at(HEIGHT).load(Ordering::Relaxed) as usize);
            let stamp = self.u64_at(STAMP).load(Ordering::Relaxed);
            if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE || now_ms.saturating_sub(stamp) > STALE_MS {
                return None;
            }
            let len = w * h * 3 / 2;
            out.resize(len, 0);
            // SAFETY: within the picture area; a torn copy is thrown away below.
            unsafe { std::ptr::copy_nonoverlapping(self.base.add(HEADER), out.as_mut_ptr(), len) };
            std::sync::atomic::fence(Ordering::Acquire);
            if seq.load(Ordering::Relaxed) == before {
                return Some((w, h));
            }
        }
        None
    }
}

/// An NV12 picture as a decoder lays it out: planes with a row pitch, and the visible part.
pub struct View<'a> {
    pub y: &'a [u8],
    pub uv: &'a [u8],
    pub pitch: usize,
    pub offset: (usize, usize),
    pub w: usize,
    pub h: usize,
}

/// [v] turned [degrees] clockwise (0, 90, 180, 270), tightly packed; its new size.
pub fn upright(v: &View, degrees: u32) -> (Vec<u8>, usize, usize) {
    let (w, h) = (v.w & !1, v.h & !1);
    let turn = degrees % 360;
    let (ow, oh) = if turn == 90 || turn == 270 { (h, w) } else { (w, h) };
    let mut out = vec![0u8; ow * oh * 3 / 2];
    let (ox, oy) = v.offset;
    // Where output (x, y) comes from in the source.
    let from = |x: usize, y: usize, sw: usize, sh: usize| -> (usize, usize) {
        match turn {
            90 => (y, sh - 1 - x),
            180 => (sw - 1 - x, sh - 1 - y),
            270 => (sw - 1 - y, x),
            _ => (x, y),
        }
    };
    for y in 0..oh {
        for x in 0..ow {
            let (sx, sy) = from(x, y, w, h);
            out[y * ow + x] = v.y[(oy + sy) * v.pitch + ox + sx];
        }
    }
    let (cw, ch, scw, sch) = (ow / 2, oh / 2, w / 2, h / 2);
    let uv_out = &mut out[ow * oh..];
    for y in 0..ch {
        for x in 0..cw {
            let (sx, sy) = from(x, y, scw, sch);
            let at = (oy / 2 + sy) * v.pitch + (ox & !1) + sx * 2;
            uv_out[y * ow + x * 2] = v.uv[at];
            uv_out[y * ow + x * 2 + 1] = v.uv[at + 1];
        }
    }
    (out, ow, oh)
}

/// [src] ([sw]×[sh], packed NV12) fitted into [dw]×[dh] with black bars, shape kept (nearest pixel).
pub fn fit(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize, out: &mut [u8]) {
    let (dw, dh) = (dw & !1, dh & !1);
    black(out, dw, dh);
    if sw < 2 || sh < 2 || dw < 2 || dh < 2 || src.len() < sw * sh * 3 / 2 {
        return;
    }
    // The fitted size, even, and where it sits.
    let scale = (dw as f64 / sw as f64).min(dh as f64 / sh as f64);
    let fw = ((sw as f64 * scale) as usize).clamp(2, dw) & !1;
    let fh = ((sh as f64 * scale) as usize).clamp(2, dh) & !1;
    let (left, top) = (((dw - fw) / 2) & !1, ((dh - fh) / 2) & !1);
    let cols: Vec<usize> = (0..fw).map(|x| (x * sw / fw).min(sw - 1)).collect();
    for y in 0..fh {
        let sy = (y * sh / fh).min(sh - 1);
        let (row, src_row) = (&mut out[(top + y) * dw + left..][..fw], &src[sy * sw..][..sw]);
        for (d, &sx) in row.iter_mut().zip(&cols) {
            *d = src_row[sx];
        }
    }
    let (suv, duv) = (&src[sw * sh..], dw * dh);
    for y in 0..fh / 2 {
        let sy = (y * (sh / 2) / (fh / 2)).min(sh / 2 - 1);
        for x in 0..fw / 2 {
            let sx = (x * (sw / 2) / (fw / 2)).min(sw / 2 - 1);
            let d = duv + (top / 2 + y) * dw + left + x * 2;
            out[d] = suv[sy * sw + sx * 2];
            out[d + 1] = suv[sy * sw + sx * 2 + 1];
        }
    }
}

fn black(out: &mut [u8], w: usize, h: usize) {
    let luma = (w * h).min(out.len());
    out[..luma].fill(16);
    out[luma..].fill(128);
}

/// What the camera shows while no phone sends: Vyoma's night-sky blue with a white orb in the
/// middle (the icon's PC), so a video call shows something that says where the picture will come from.
pub fn placeholder(w: usize, h: usize, out: &mut [u8]) {
    let (w, h) = (w & !1, h & !1);
    // BT.601 video range: #1d2b4a sky, white orb.
    let (sky_y, sky_u, sky_v) = (43u8, 150u8, 121u8);
    let (cx, cy, r) = (w as f64 / 2.0, h as f64 / 2.0, h.min(w) as f64 / 7.0);
    for y in 0..h {
        for x in 0..w {
            let d = ((x as f64 + 0.5 - cx).powi(2) + (y as f64 + 0.5 - cy).powi(2)).sqrt();
            // A soft rim: a pixel's worth of blend.
            let a = (r + 0.5 - d).clamp(0.0, 1.0);
            out[y * w + x] = (sky_y as f64 + (235.0 - sky_y as f64) * a) as u8;
        }
    }
    let uv = &mut out[w * h..w * h * 3 / 2];
    for y in 0..h / 2 {
        for x in 0..w / 2 {
            let d = ((x as f64 * 2.0 + 1.0 - cx).powi(2) + (y as f64 * 2.0 + 1.0 - cy).powi(2)).sqrt();
            let a = (r + 1.0 - d).clamp(0.0, 1.0);
            uv[y * w + x * 2] = (sky_u as f64 + (128.0 - sky_u as f64) * a) as u8;
            uv[y * w + x * 2 + 1] = (sky_v as f64 + (128.0 - sky_v as f64) * a) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 4×2 picture whose luma counts 0..8 and whose two chroma pairs are (1,2), (3,4).
    fn tiny() -> Vec<u8> {
        vec![0, 1, 2, 3, 4, 5, 6, 7, 1, 2, 3, 4]
    }

    fn view(p: &[u8], w: usize, h: usize) -> View<'_> {
        View { y: &p[..w * h], uv: &p[w * h..], pitch: w, offset: (0, 0), w, h }
    }

    #[test]
    fn turns_pictures_upright() {
        let p = tiny();
        let (same, w, h) = upright(&view(&p, 4, 2), 0);
        assert_eq!((same, w, h), (p.clone(), 4, 2));
        let (r, w, h) = upright(&view(&p, 4, 2), 90);
        assert_eq!((w, h), (2, 4));
        // Clockwise: the bottom-left comes to the top-left.
        assert_eq!(&r[..8], &[4, 0, 5, 1, 6, 2, 7, 3]);
        assert_eq!(&r[8..], &[1, 2, 3, 4], "chroma turns with it");
        let (r, ..) = upright(&view(&p, 4, 2), 180);
        assert_eq!(&r[..8], &[7, 6, 5, 4, 3, 2, 1, 0]);
        assert_eq!(&r[8..], &[3, 4, 1, 2]);
        let (r, w, h) = upright(&view(&p, 4, 2), 270);
        assert_eq!((w, h), (2, 4));
        assert_eq!(&r[..8], &[3, 7, 2, 6, 1, 5, 0, 4]);
        assert_eq!(&r[8..], &[3, 4, 1, 2]);
    }

    #[test]
    fn reads_a_decoders_padded_layout() {
        // 4×2 visible inside 6×4 with a 6-byte pitch, from (2, 2).
        let mut y = vec![99u8; 6 * 4];
        let mut uv = vec![99u8; 6 * 2];
        for r in 0..2 {
            for c in 0..4 {
                y[(2 + r) * 6 + 2 + c] = (r * 4 + c) as u8;
            }
        }
        uv[6 + 2..6 + 6].copy_from_slice(&[1, 2, 3, 4]);
        let v = View { y: &y, uv: &uv, pitch: 6, offset: (2, 2), w: 4, h: 2 };
        assert_eq!(upright(&v, 0).0, tiny());
    }

    #[test]
    fn fits_with_bars_and_keeps_the_shape() {
        // A 4×4 white square into 8×4: bars left and right.
        let mut src = vec![235u8; 16];
        src.extend([128u8; 8]);
        let mut out = vec![0u8; 8 * 4 * 3 / 2];
        fit(&src, 4, 4, 8, 4, &mut out);
        for row in 0..4 {
            assert_eq!(&out[row * 8..row * 8 + 8], &[16, 16, 235, 235, 235, 235, 16, 16]);
        }
        assert!(out[32..].iter().all(|&c| c == 128));
        // Bigger into smaller: still fills the middle.
        let big = vec![200u8; 1280 * 720 * 3 / 2];
        let mut small = vec![0u8; 640 * 360 * 3 / 2];
        fit(&big, 1280, 720, 640, 360, &mut small);
        assert!(small[..640 * 360].iter().all(|&l| l == 200));
    }

    #[test]
    fn the_placeholder_has_its_orb() {
        let (w, h) = (64, 36);
        let mut out = vec![0u8; w * h * 3 / 2];
        placeholder(w, h, &mut out);
        assert_eq!(out[(h / 2) * w + w / 2], 235, "white in the middle");
        assert_eq!(out[0], 43, "sky in the corner");
    }

    #[test]
    fn readers_see_whole_pictures_only() {
        #[repr(C, align(8))]
        struct Section([u8; HEADER + 4 * 2 * 3 / 2]);
        let mut mem = Box::new(Section([0; HEADER + 12]));
        // SAFETY: an aligned buffer as big as the pictures written here (the test never writes bigger).
        let shared = unsafe { Shared::new(mem.0.as_mut_ptr()) };
        let mut out = Vec::new();
        assert_eq!(shared.read(&mut out, 0), None, "nothing written yet");
        shared.write(4, 2, &tiny(), 1000);
        assert_eq!(shared.read(&mut out, 1500), None, "not live");
        shared.set_live(true);
        assert_eq!(shared.read(&mut out, 1500), Some((4, 2)));
        assert_eq!(out, tiny());
        assert_eq!(shared.read(&mut out, 1000 + STALE_MS + 1), None, "stale");
        // A write in progress (odd count) isn't read.
        shared.u64_at(SEQ).fetch_add(1, Ordering::Relaxed);
        assert_eq!(shared.read(&mut out, 1500), None);
    }
}
