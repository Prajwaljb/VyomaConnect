//! The desktop wallpaper's colours, for the phone's PC tile (`vyoma.wallpaper`) and Vyoma's window
//! (the pipe's `Wallpaper`): the picture averaged down to a 4×4 grid, as the other PC sides do.
//! Windows says where the picture is (SystemParametersInfo); its own imaging component (WIC) reads
//! it, whatever it is (JPEG, PNG, BMP, HEIC where that's installed), and shrinks it. A plain
//! colour desktop has no picture: that colour fills the grid.

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Graphics::Gdi::{COLOR_DESKTOP, GetSysColor};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppBGRA, IWICImagingFactory, WICBitmapDitherTypeNone, WICBitmapInterpolationModeFant,
    WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
use windows::Win32::UI::WindowsAndMessaging::{SPI_GETDESKWALLPAPER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW};
use windows::core::HSTRING;

pub const COLS: usize = 4;
pub const ROWS: usize = 4;
/// The picture is shrunk to this wide before averaging.
const SMALL: u32 = 64;

/// The wallpaper's file; None for a plain colour.
pub fn path() -> Option<String> {
    let mut buf = [0u16; 1024];
    // SAFETY: Windows writes at most buf.len() characters, NUL-terminated.
    unsafe { SystemParametersInfoW(SPI_GETDESKWALLPAPER, buf.len() as u32, Some(buf.as_mut_ptr().cast()), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)).ok()? };
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len])).filter(|p| !p.trim().is_empty())
}

/// Changes when the wallpaper does: its file and when it was last written (Windows rewrites the
/// same file for a new picture), or the plain colour.
pub fn signature() -> String {
    match path() {
        Some(p) => format!("{p} {:?}", std::fs::metadata(&p).and_then(|m| m.modified()).ok()),
        // SAFETY: a plain read of a system colour.
        None => format!("colour {}", unsafe { GetSysColor(COLOR_DESKTOP) }),
    }
}

/// The grid of colours, row by row (blocking: run off the async threads).
pub fn colors() -> Result<Vec<[u8; 3]>> {
    let Some(path) = path() else {
        // SAFETY: as in [signature]. COLORREF is 0x00BBGGRR.
        let c = unsafe { GetSysColor(COLOR_DESKTOP) };
        return Ok(vec![[c as u8, (c >> 8) as u8, (c >> 16) as u8]; COLS * ROWS]);
    };
    let (w, h, bgra) = small_picture(&path)?;
    average(&bgra, w, h)
}

/// What the window's `Wallpaper` answers.
pub fn for_window() -> Result<Value> {
    Ok(json!({ "cols": COLS, "rows": ROWS, "colors": colors()? }))
}

/// The picture at [path] shrunk to [SMALL] wide, as BGRA.
fn small_picture(path: &str) -> Result<(usize, usize, Vec<u8>)> {
    // SAFETY: this thread joins the multithreaded apartment; WIC's objects are owned and the
    // pixels copied into our buffer of exactly stride × height bytes.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let decoder = wic.CreateDecoderFromFilename(&HSTRING::from(path), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand).context("Windows can't read the wallpaper")?;
        let frame = decoder.GetFrame(0)?;
        let (mut w, mut h) = (0u32, 0u32);
        frame.GetSize(&mut w, &mut h)?;
        if w == 0 || h == 0 {
            bail!("the wallpaper is empty");
        }
        let converter = wic.CreateFormatConverter()?;
        converter.Initialize(&frame, &GUID_WICPixelFormat32bppBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)?;
        let sw = SMALL.min(w);
        let sh = ((h as u64 * sw as u64 / w as u64) as u32).max(1);
        let scaler = wic.CreateBitmapScaler()?;
        scaler.Initialize(&converter, sw, sh, WICBitmapInterpolationModeFant)?;
        let mut pixels = vec![0u8; (sw * sh * 4) as usize];
        scaler.CopyPixels(std::ptr::null(), sw * 4, &mut pixels)?;
        Ok((sw as usize, sh as usize, pixels))
    }
}

/// BGRA pixels ([w]×[h]) averaged into [COLS]×[ROWS] RGB blocks, row by row.
pub fn average(bgra: &[u8], w: usize, h: usize) -> Result<Vec<[u8; 3]>> {
    if w < COLS || h < ROWS {
        bail!("the wallpaper is too small");
    }
    let mut out = Vec::with_capacity(COLS * ROWS);
    for row in 0..ROWS {
        for col in 0..COLS {
            let (mut sum, mut n) = ([0u64; 3], 0u64);
            for y in row * h / ROWS..(row + 1) * h / ROWS {
                for x in col * w / COLS..(col + 1) * w / COLS {
                    let px = &bgra[(y * w + x) * 4..][..4];
                    sum[0] += px[2] as u64;
                    sum[1] += px[1] as u64;
                    sum[2] += px[0] as u64;
                    n += 1;
                }
            }
            out.push(sum.map(|s| (s / n.max(1)) as u8));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_are_averaged_in_rgb() {
        // 8x8: the left half red, the right half blue (BGRA).
        let mut px = Vec::new();
        for _y in 0..8 {
            for x in 0..8 {
                px.extend_from_slice(if x < 4 { &[0, 0, 255, 255] } else { &[255, 0, 0, 255] });
            }
        }
        let grid = average(&px, 8, 8).unwrap();
        assert_eq!(grid.len(), COLS * ROWS);
        assert_eq!((grid[0], grid[3], grid[12]), ([255, 0, 0], [0, 0, 255], [255, 0, 0]));
        assert!(average(&px, 2, 2).is_err());
    }

    #[test]
    fn reads_this_pcs_wallpaper() {
        // Wine's desktop has no picture: its plain colour, then.
        let grid = colors().unwrap();
        assert_eq!(grid.len(), COLS * ROWS);
        assert!(!signature().is_empty());
    }
}
