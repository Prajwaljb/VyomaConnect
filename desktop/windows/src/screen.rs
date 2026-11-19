//! This PC's screens: a screenshot of all of them for the phone (GDI, PNG), and what they are
//! for About this PC (each display's own pixels, the graphics adapters by name).
//! Built and tested on CI and under Wine only; not yet run on a real Windows PC.

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use vyoma_core::identity::Identity;
use vyoma_core::link::{Sender, packet};
use vyoma_core::payload;
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap, CreateCompatibleDC, DIB_RGB_COLORS, DISPLAY_DEVICE_ATTACHED_TO_DESKTOP,
    DISPLAY_DEVICE_PRIMARY_DEVICE, DISPLAY_DEVICEW, DEVMODEW, DeleteDC, DeleteObject, ENUM_CURRENT_SETTINGS, EnumDisplayDevicesW, EnumDisplaySettingsW,
    GetDC, GetDIBits, ReleaseDC, SRCCOPY, SelectObject,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN};

use crate::win::from_wide;

pub const SCREENSHOT: &str = "vyoma.screenshot";
/// Past this, it isn't a screen (a broken metric); nothing that size is sent.
const MAX_SIDE: i32 = 32_768;

/// Takes a screenshot and offers it to the phone; a failure reaches the phone as its error, so it
/// never waits on a spinner.
pub async fn screenshot(sender: &Sender, me: &Identity, pinned: &[u8], phone: &str) -> Result<()> {
    let image = match tokio::task::spawn_blocking(capture).await.context("the screenshot stopped")? {
        Ok(b) => b,
        Err(e) => {
            sender.send(&packet(SCREENSHOT, json!({ "error": format!("{e:#}") }))).await?;
            return Err(e);
        }
    };
    // Someone at the PC should always know its screen was taken from elsewhere.
    crate::tray::notify("Screenshot taken from your phone", &format!("{phone} took a screenshot of this PC."));
    payload::offer(sender, me, pinned, packet(SCREENSHOT, json!({ "format": "png" })), image.len() as u64, image.as_slice()).await
}

/// Every screen at once, as one PNG.
pub fn capture() -> Result<Vec<u8>> {
    // SAFETY: GetSystemMetrics takes no pointers.
    let (x, y, w, h) = unsafe { (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN), GetSystemMetrics(SM_CXVIRTUALSCREEN), GetSystemMetrics(SM_CYVIRTUALSCREEN)) };
    if !(1..=MAX_SIDE).contains(&w) || !(1..=MAX_SIDE).contains(&h) {
        bail!("Windows didn't say how big the screen is");
    }
    let bgra = grab(x, y, w, h)?;
    encode(&bgra, w as u32, h as u32)
}

/// The screen's pixels, top row first, as GDI gives them (blue, green, red, unused).
fn grab(x: i32, y: i32, w: i32, h: i32) -> Result<Vec<u8>> {
    // SAFETY: the screen's DC, a compatible DC and bitmap made, used and released here in order;
    // GetDIBits fills a buffer of exactly w*h*4 bytes (32 bits, top-down).
    unsafe {
        let screen = GetDC(std::ptr::null_mut());
        if screen.is_null() {
            bail!("no screen to capture (is the PC locked?)");
        }
        let dc = CreateCompatibleDC(screen);
        let bitmap = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(dc, bitmap);
        // CAPTUREBLT: layered windows (menus, tooltips) too.
        let copied = BitBlt(dc, 0, 0, w, h, screen, x, y, SRCCOPY | CAPTUREBLT) != 0;
        SelectObject(dc, old);
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader = BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB, ..std::mem::zeroed() };
        let mut pixels = vec![0u8; w as usize * h as usize * 4];
        let rows = if copied { GetDIBits(dc, bitmap, 0, h as u32, pixels.as_mut_ptr().cast(), &mut info, DIB_RGB_COLORS) } else { 0 };
        DeleteObject(bitmap);
        DeleteDC(dc);
        ReleaseDC(std::ptr::null_mut(), screen);
        if rows != h {
            bail!("Windows didn't hand over the screen (is the PC locked?)");
        }
        Ok(pixels)
    }
}

fn encode(bgra: &[u8], w: u32, h: u32) -> Result<Vec<u8>> {
    let rgb: Vec<u8> = bgra.as_chunks::<4>().0.iter().flat_map(|p| [p[2], p[1], p[0]]).collect();
    let mut out = Vec::new();
    let mut png = png::Encoder::new(&mut out, w, h);
    png.set_color(png::ColorType::Rgb);
    png.set_depth(png::BitDepth::Eight);
    // Screens are flat colour and text: fast filtering keeps it quick and still small.
    png.set_compression(png::Compression::Fast);
    png.write_header()?.write_image_data(&rgb)?;
    Ok(out)
}

/// The graphics adapters showing the desktop, by name, and each display's own pixels, the main
/// one first. Windows doesn't say which display is built in, so `internal` is never claimed.
pub fn displays() -> (Vec<String>, Vec<Value>) {
    let mut gpus: Vec<String> = Vec::new();
    let mut screens: Vec<(bool, Value)> = Vec::new();
    for i in 0.. {
        // SAFETY: a zeroed DISPLAY_DEVICEW with its size set, filled by Windows.
        let mut d: DISPLAY_DEVICEW = unsafe { std::mem::zeroed() };
        d.cb = size_of::<DISPLAY_DEVICEW>() as u32;
        // SAFETY: as above; false past the last adapter.
        if unsafe { EnumDisplayDevicesW(std::ptr::null(), i, &mut d, 0) } == 0 {
            break;
        }
        if d.StateFlags & DISPLAY_DEVICE_ATTACHED_TO_DESKTOP == 0 {
            continue;
        }
        // SAFETY: fixed arrays Windows fills with NUL-terminated strings.
        let (name, device) = unsafe { (from_wide(d.DeviceString.as_ptr()), d.DeviceName) };
        if !name.is_empty() && !gpus.contains(&name) {
            gpus.push(name);
        }
        // SAFETY: a zeroed DEVMODEW with its size set, filled for this display.
        let mut mode: DEVMODEW = unsafe { std::mem::zeroed() };
        mode.dmSize = size_of::<DEVMODEW>() as u16;
        // SAFETY: as above.
        if unsafe { EnumDisplaySettingsW(device.as_ptr(), ENUM_CURRENT_SETTINGS, &mut mode) } != 0 && mode.dmPelsWidth > 0 {
            let primary = d.StateFlags & DISPLAY_DEVICE_PRIMARY_DEVICE != 0;
            screens.push((primary, json!({ "w": mode.dmPelsWidth, "h": mode.dmPelsHeight, "internal": false })));
        }
    }
    screens.sort_by_key(|(primary, _)| !*primary);
    (gpus, screens.into_iter().map(|(_, s)| s).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixels_become_a_png() {
        // Two pixels: blue and red, as GDI orders them.
        let png = encode(&[255, 0, 0, 0, 0, 0, 255, 0], 2, 1).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let decoder = png::Decoder::new(std::io::Cursor::new(png));
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size()];
        reader.next_frame(&mut buf).unwrap();
        assert_eq!(&buf[..6], &[0, 0, 255, 255, 0, 0], "red and green swapped back");
    }

    /// Needs a screen (Windows, or Wine with a display); skipped on a headless runner that has none.
    #[test]
    fn captures_the_screen() {
        match capture() {
            Ok(png) => assert_eq!(&png[..4], b"\x89PNG"),
            Err(e) => eprintln!("no screen here: {e:#}"),
        }
        let (_, screens) = displays();
        assert!(screens.iter().all(|s| s["w"].as_u64().unwrap() > 0));
    }
}
