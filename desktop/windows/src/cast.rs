//! This PC's screen live on the phone (`vyoma.screen.request`), worked by touch
//! (`vyoma.screen.input`/`touch`), as the Linux app and the Mac send it.
//!
//! The main display is captured with desktop duplication (DXGI), which hands over a picture only
//! when something changed, so a still screen costs nothing; where that isn't there (Wine, some
//! remote sessions, an old graphics driver) a GDI copy of the display at the frame rate instead.
//! A display turned to portrait comes from duplication sideways and is turned upright ([Turn]).
//! The pointer is drawn on top (neither shows it). Each picture is scaled to at most 1920 wide,
//! turned into NV12 (by the graphics card's video processor where it has one, [crate::gpu], else
//! on the CPU, [to_nv12]) and encoded to H.264 by the graphics card's encoder where it has one that
//! takes them, else Microsoft's software one ([Encoder]; real time, no B-frames, a keyframe to
//! start), then framed as the Mac's [frame]: 4-byte lengths, the parameter
//! sets before each keyframe, a timestamp unit before each picture for the phone's latency figures.
//! If the link falls behind, new pictures are skipped before encoding instead of piling up.
//!
//! The lock screen and administrators' prompts are a desktop of their own that can't be captured:
//! the phone keeps the last picture until they're gone. Whoever is at the PC is told with a
//! notification when the phone starts watching.
//! Built and tested on CI and under Wine only; not yet run on a real Windows PC.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use vyoma_core::identity::Identity;
use vyoma_core::link::{Sender, packet};
use vyoma_core::payload;
use windows::Win32::Foundation::{HMODULE, POINT, RECT};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_MODE_ROTATION, DXGI_MODE_ROTATION_ROTATE90, DXGI_MODE_ROTATION_ROTATE180, DXGI_MODE_ROTATION_ROTATE270, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource,
};
use windows::Win32::Graphics::Gdi::{
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BitBlt, CAPTUREBLT, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GdiFlush, GetDC,
    GetMonitorInfoW, HBITMAP, HDC, HGDIOBJ, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint, ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::Media::MediaFoundation::{
    CODECAPI_AVEncMPVDefaultBPictureCount, CODECAPI_AVEncMPVGOPSize, CODECAPI_AVLowLatencyMode, ICodecAPI, IMFActivate, IMFMediaEventGenerator, IMFMediaType,
    IMFSample, IMFShutdown, IMFTransform, METransformHaveOutput, METransformNeedInput, MF_E_NO_EVENTS_AVAILABLE, MF_E_TRANSFORM_NEED_MORE_INPUT,
    MF_E_TRANSFORM_STREAM_CHANGE, MF_EVENT_FLAG_NO_WAIT, MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE,
    MF_MT_MPEG2_PROFILE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE, MF_TRANSFORM_ASYNC, MF_TRANSFORM_ASYNC_UNLOCK, MF_VERSION, MFCreateMediaType,
    MFCreateMemoryBuffer, MFCreateSample, MFMediaType_Video, MFSTARTUP_LITE, MFStartup, MFT_CATEGORY_VIDEO_ENCODER, MFT_ENUM_FLAG, MFT_ENUM_FLAG_HARDWARE,
    MFT_ENUM_FLAG_LOCALMFT, MFT_ENUM_FLAG_SORTANDFILTER, MFT_ENUM_FLAG_SYNCMFT, MFT_FRIENDLY_NAME_Attribute, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
    MFT_MESSAGE_NOTIFY_END_OF_STREAM, MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_OUTPUT_DATA_BUFFER, MFT_OUTPUT_STREAM_PROVIDES_SAMPLES, MFT_REGISTER_TYPE_INFO,
    MFTEnumEx, MFVideoFormat_H264, MFVideoFormat_NV12, MFVideoInterlace_Progressive, eAVEncH264VProfile_Main,
};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::WindowsAndMessaging::{CURSOR_SHOWING, CURSORINFO, DI_NORMAL, DrawIconEx, GetCursorInfo, GetIconInfo, ICONINFO};
use windows::core::Interface;

use crate::input::{self, Act, Area, Button};

pub const SCREEN: &str = "vyoma.screen";

/// The phone's screen is about 1080 px tall in landscape: more than 1920 wide is wasted bits.
const MAX_WIDTH: i32 = 1920;
const BITRATE: u32 = 8_000_000;
/// 30 frames a second unless the phone asks for more (up to 60), never fewer.
const MIN_FPS: u32 = 30;
const MAX_FPS: u32 = 60;
/// Encoded video waiting for the link beyond this, and new pictures are skipped until it drains.
const MAX_PENDING: usize = 256 * 1024;
/// Small windows from the encoder to the socket, so a slow link holds frames back at the start.
const RELAY_BYTES: usize = 16 * 1024;
const STAMP_MAGIC: &[u8; 4] = b"VYTS";
/// A keyframe to start, then this many pictures apart (the link is TCP: nothing is lost).
const KEYFRAME_FRAMES: u32 = 1024;
/// Waiting while the screen can't be captured (the lock screen), between tries.
const RETRY: Duration = Duration::from_millis(500);

/// The rate a phone asked for, within what this PC's encoder keeps up with.
pub fn fps_for(asked: Option<u64>) -> u32 {
    asked.map_or(MIN_FPS, |f| (f.min(MAX_FPS as u64) as u32).max(MIN_FPS))
}

/// Scales to fit [MAX_WIDTH] keeping the shape; even sizes, as H.264 wants.
pub fn fit((w, h): (i32, i32)) -> (i32, i32) {
    let (w, h) = if w > MAX_WIDTH { (MAX_WIDTH, (h as i64 * MAX_WIDTH as i64 / w as i64) as i32) } else { (w, h) };
    (w & !1, h & !1)
}

/// One encoded picture as the phone reads it (the Mac's framing): on a keyframe its parameter
/// sets first, then the picture's NAL units with a timestamp unit (an SEI-typed NAL: our marker
/// and the send time in µs since the epoch, which the phone takes out before decoding) before its
/// first slice. [avcc] is already 4-byte length prefixed.
pub fn frame(params: &[&[u8]], avcc: &[u8], now_us: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(avcc.len() + 64);
    for p in params {
        out.extend_from_slice(&(p.len() as u32).to_be_bytes());
        out.extend_from_slice(p);
    }
    let mut at = 0;
    let mut stamped = false;
    while let Some(len) = avcc.get(at..at + 4).map(|l| u32::from_be_bytes([l[0], l[1], l[2], l[3]]) as usize) {
        let Some(nal) = avcc.get(at + 4..at + 4 + len) else { break };
        if !stamped && nal.first().is_some_and(|h| matches!(h & 0x1f, 1 | 5)) {
            out.extend_from_slice(&13u32.to_be_bytes());
            out.push(0x06);
            out.extend_from_slice(STAMP_MAGIC);
            out.extend_from_slice(&now_us.to_be_bytes());
            stamped = true;
        }
        out.extend_from_slice(&avcc[at..at + 4 + len]);
        at += 4 + len;
    }
    out
}

/// The NAL units of an Annex-B stream (start codes 00 00 01 or 00 00 00 01), as Windows' encoder writes it.
pub fn nal_units(annexb: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= annexb.len() {
        if annexb[i] == 0 && annexb[i + 1] == 0 && annexb[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut out = Vec::new();
    for (n, &s) in starts.iter().enumerate() {
        let mut end = starts.get(n + 1).map_or(annexb.len(), |&next| next - 3);
        // The zero before a 4-byte start code belongs to it, not to this unit.
        while end > s && annexb[end - 1] == 0 && starts.get(n + 1).is_some() {
            end -= 1;
        }
        if end > s {
            out.push(&annexb[s..end]);
        }
    }
    out
}

/// Keeps the newest parameter sets and frames each encoded picture: a keyframe that came without
/// them gets them first.
#[derive(Default)]
pub struct Framer {
    sps: Vec<u8>,
    pps: Vec<u8>,
}

impl Framer {
    pub fn frame(&mut self, annexb: &[u8], now_us: u64) -> Vec<u8> {
        let units = nal_units(annexb);
        let kind = |u: &[u8]| u.first().map_or(0, |h| h & 0x1f);
        for u in &units {
            match kind(u) {
                7 => self.sps = u.to_vec(),
                8 => self.pps = u.to_vec(),
                _ => {}
            }
        }
        let keyframe = units.iter().any(|u| kind(u) == 5);
        let has_params = units.iter().any(|u| kind(u) == 7);
        let mut avcc = Vec::with_capacity(annexb.len() + 16);
        for u in &units {
            avcc.extend_from_slice(&(u.len() as u32).to_be_bytes());
            avcc.extend_from_slice(u);
        }
        let params: Vec<&[u8]> = if keyframe && !has_params && !self.sps.is_empty() { vec![&self.sps, &self.pps] } else { Vec::new() };
        frame(&params, &avcc, now_us)
    }
}

/// BGRA pixels ([stride] bytes a row, [sw]×[sh]) to NV12 at [dw]×[dh] (both even): each output
/// pixel is the average of the 2×2 source pixels at its place (BT.601, video range, as decoders
/// assume when the stream doesn't say).
pub fn to_nv12(src: &[u8], stride: usize, sw: usize, sh: usize, dw: usize, dh: usize, out: &mut Vec<u8>) {
    out.clear();
    out.resize(dw * dh * 3 / 2, 0);
    let (y_plane, uv_plane) = out.split_at_mut(dw * dh);
    let xs: Vec<usize> = (0..dw).map(|x| (x * sw / dw).min(sw.saturating_sub(2))).collect();
    let pixel = |x: usize, y: usize| {
        let i = y * stride + x * 4;
        (src[i + 2] as i32, src[i + 1] as i32, src[i] as i32)
    };
    for y in 0..dh {
        let sy = (y * sh / dh).min(sh.saturating_sub(2));
        let row = &mut y_plane[y * dw..(y + 1) * dw];
        for (x, out) in row.iter_mut().enumerate() {
            let sx = xs[x];
            let (mut r, mut g, mut b) = (0, 0, 0);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let p = pixel(sx + dx, sy + dy);
                (r, g, b) = (r + p.0, g + p.1, b + p.2);
            }
            let (r, g, b) = (r / 4, g / 4, b / 4);
            *out = (((66 * r + 129 * g + 25 * b + 128) >> 8) + 16).clamp(16, 235) as u8;
        }
    }
    for y in 0..dh / 2 {
        let sy = ((y * 2) * sh / dh).min(sh.saturating_sub(2));
        for x in 0..dw / 2 {
            let sx = xs[x * 2];
            let (mut r, mut g, mut b) = (0, 0, 0);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let p = pixel(sx + dx, sy + dy);
                (r, g, b) = (r + p.0, g + p.1, b + p.2);
            }
            let (r, g, b) = (r / 4, g / 4, b / 4);
            let i = y * dw + x * 2;
            uv_plane[i] = (((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128).clamp(16, 240) as u8;
            uv_plane[i + 1] = (((112 * r - 94 * g - 18 * b + 128) >> 8) + 128).clamp(16, 240) as u8;
        }
    }
}

fn now_us() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_micros() as u64).unwrap_or(0)
}

/// The main display's place among all of them.
pub fn main_display() -> Area {
    // SAFETY: the primary monitor's handle and a MONITORINFO with its size set.
    unsafe {
        let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        if GetMonitorInfoW(monitor, &mut info).as_bool() {
            let r = info.rcMonitor;
            return Area { left: r.left, top: r.top, width: r.right - r.left, height: r.bottom - r.top };
        }
        Area { left: 0, top: 0, width: 0, height: 0 }
    }
}

/// A picture of the display in our own memory, which GDI can draw on (the pointer) and we can read.
struct Canvas {
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
    w: i32,
    h: i32,
}

impl Canvas {
    fn new(w: i32, h: i32) -> Result<Canvas> {
        // SAFETY: a top-down 32-bit DIB section selected into a memory DC; freed in Drop.
        unsafe {
            let dc = CreateCompatibleDC(None);
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let bitmap = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0).context("no memory for the picture")?;
            let old = SelectObject(dc, bitmap.into());
            Ok(Canvas { dc, bitmap, old, bits: bits.cast(), w, h })
        }
    }

    fn pixels(&self) -> &[u8] {
        // SAFETY: the DIB section is w*h*4 bytes, alive while self is; GDI is flushed before reading.
        unsafe {
            let _ = GdiFlush();
            std::slice::from_raw_parts(self.bits, self.w as usize * self.h as usize * 4)
        }
    }

    fn pixels_mut(&mut self) -> &mut [u8] {
        // SAFETY: as in pixels.
        unsafe {
            let _ = GdiFlush();
            std::slice::from_raw_parts_mut(self.bits, self.w as usize * self.h as usize * 4)
        }
    }

    /// The pointer where it is on [display], if it's showing.
    fn draw_pointer(&self, display: Area) {
        draw_pointer(self.dc, display);
    }
}

/// The pointer where it is on [display] into [dc], if it's showing.
fn draw_pointer(dc: HDC, display: Area) {
    // SAFETY: reading the cursor's state and drawing it into the DC; the icon's bitmaps that
    // GetIconInfo makes are deleted.
    unsafe {
        let mut ci = CURSORINFO { cbSize: size_of::<CURSORINFO>() as u32, ..Default::default() };
        if GetCursorInfo(&mut ci).is_err() || ci.flags.0 & CURSOR_SHOWING.0 == 0 {
            return;
        }
        let mut icon = ICONINFO::default();
        let (hx, hy) = if GetIconInfo(ci.hCursor.into(), &mut icon).is_ok() {
            if !icon.hbmMask.is_invalid() {
                let _ = DeleteObject(icon.hbmMask.into());
            }
            if !icon.hbmColor.is_invalid() {
                let _ = DeleteObject(icon.hbmColor.into());
            }
            (icon.xHotspot as i32, icon.yHotspot as i32)
        } else {
            (0, 0)
        };
        let _ = DrawIconEx(dc, ci.ptScreenPos.x - display.left - hx, ci.ptScreenPos.y - display.top - hy, ci.hCursor.into(), 0, 0, 0, None, DI_NORMAL);
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        // SAFETY: made in new, in reverse.
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

/// How desktop duplication hands over a display's picture: in the panel's own orientation, so a
/// display turned to portrait comes sideways (`DXGI_OUTDUPL_DESC.Rotation`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Turn {
    None,
    /// DXGI_MODE_ROTATION_ROTATE90.
    Quarter,
    Half,
    /// DXGI_MODE_ROTATION_ROTATE270.
    ThreeQuarters,
}

impl Turn {
    fn of(rotation: DXGI_MODE_ROTATION) -> Turn {
        match rotation {
            DXGI_MODE_ROTATION_ROTATE90 => Turn::Quarter,
            DXGI_MODE_ROTATION_ROTATE180 => Turn::Half,
            DXGI_MODE_ROTATION_ROTATE270 => Turn::ThreeQuarters,
            _ => Turn::None,
        }
    }

    /// The desktop's size for a duplicated picture of [w]×[h].
    pub fn upright(self, (w, h): (i32, i32)) -> (i32, i32) {
        if matches!(self, Turn::Quarter | Turn::ThreeQuarters) { (h, w) } else { (w, h) }
    }

    /// Where the desktop's pixel ([x], [y]) of a [dw]×[dh] desktop is in the duplicated picture.
    /// The inverse of Microsoft's Desktop Duplication sample (DisplayManager's SetDirtyVert), which
    /// puts the picture's (sx, sy) on the desktop at (dw-1-sy, sx) for 90°, (dw-1-sx, dh-1-sy) for
    /// 180° and (sy, dh-1-sx) for 270°.
    pub fn source(self, x: usize, y: usize, dw: usize, dh: usize) -> (usize, usize) {
        match self {
            Turn::None => (x, y),
            Turn::Quarter => (y, dw - 1 - x),
            Turn::Half => (dw - 1 - x, dh - 1 - y),
            Turn::ThreeQuarters => (dh - 1 - y, x),
        }
    }
}

/// A duplicated picture ([pitch] bytes a row) turned upright into [dst], a [dw]×[dh] desktop of
/// 4-byte pixels.
pub fn turn_upright(src: &[u8], pitch: usize, turn: Turn, dst: &mut [u8], dw: usize, dh: usize) {
    for y in 0..dh {
        let row = &mut dst[y * dw * 4..(y + 1) * dw * 4];
        for (x, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let (sx, sy) = turn.source(x, y, dw, dh);
            let i = sy * pitch + sx * 4;
            px.copy_from_slice(&src[i..i + 4]);
        }
    }
}

/// Desktop duplication of one display.
struct Duplication {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    dup: IDXGIOutputDuplication,
    turn: Turn,
    /// A CPU-readable copy of the last picture, made the size of the first one that comes.
    staging: Option<ID3D11Texture2D>,
    /// The picture's size as it comes (before turning).
    w: i32,
    h: i32,
    /// Whether a picture has come.
    picture: bool,
    /// The card's conversion to the stream's size, once tried on the first picture; None where
    /// it can't, and then the staging copy for the CPU.
    gpu: Option<crate::gpu::Converter>,
    gpu_tried: bool,
    out_size: (u32, u32),
    fps: u32,
}

impl Duplication {
    /// The display at [display] on whichever graphics adapter shows it.
    fn open(display: Area, out_size: (u32, u32), fps: u32) -> Result<Duplication> {
        // SAFETY: DXGI and D3D11 calls with owned COM pointers.
        unsafe {
            let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
            let mut a = 0;
            while let Ok(adapter) = factory.EnumAdapters1(a) {
                a += 1;
                let mut o = 0;
                while let Ok(output) = adapter.EnumOutputs(o) {
                    o += 1;
                    let Ok(desc) = output.GetDesc() else { continue };
                    let r: RECT = desc.DesktopCoordinates;
                    if (r.left, r.top) != (display.left, display.top) {
                        continue;
                    }
                    return Self::on(&adapter, &output.cast::<IDXGIOutput1>()?, out_size, fps);
                }
            }
            bail!("no graphics adapter shows the main display")
        }
    }

    unsafe fn on(adapter: &IDXGIAdapter1, output: &IDXGIOutput1, out_size: (u32, u32), fps: u32) -> Result<Duplication> {
        // SAFETY: the caller's; everything made here is owned.
        unsafe {
            let (mut device, mut context): (Option<ID3D11Device>, Option<ID3D11DeviceContext>) = (None, None);
            D3D11CreateDevice(adapter, D3D_DRIVER_TYPE_UNKNOWN, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut context))?;
            let (device, context) = (device.context("no Direct3D device")?, context.context("no Direct3D context")?);
            let dup = output.DuplicateOutput(&device).context("Windows wouldn't share the screen")?;
            let turn = Turn::of(dup.GetDesc().Rotation);
            Ok(Duplication { device, context, dup, turn, staging: None, w: 0, h: 0, picture: false, gpu: None, gpu_tried: false, out_size, fps })
        }
    }

    /// The desktop's size, once a picture came.
    fn size(&self) -> Option<(i32, i32)> {
        self.picture.then(|| self.turn.upright((self.w, self.h)))
    }

    /// Waits up to [wait] for a change; true if there's a new picture. Err when the duplication is
    /// lost (the lock screen, a new display mode): open it again.
    fn next(&mut self, wait: Duration) -> windows::core::Result<bool> {
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource: Option<IDXGIResource> = None;
        // SAFETY: as in open; the frame is released before returning; the staging texture is
        // described from the frame's own texture, so CopyResource's sizes and formats match.
        unsafe {
            match self.dup.AcquireNextFrame(wait.as_millis() as u32, &mut info, &mut resource) {
                Ok(()) => {}
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(false),
                Err(e) => return Err(e),
            }
            let mut copied = false;
            if (info.LastPresentTime != 0 || !self.picture)
                && let Some(tex) = resource.as_ref().and_then(|r| r.cast::<ID3D11Texture2D>().ok())
            {
                let mut desc = D3D11_TEXTURE2D_DESC::default();
                tex.GetDesc(&mut desc);
                let size = (desc.Width, desc.Height);
                // The card converts upright pictures of the size it was made for; a turned display
                // (rare) and a changed mode go through the CPU.
                if !self.gpu_tried && self.turn == Turn::None {
                    self.gpu_tried = true;
                    match crate::gpu::Converter::new(&self.device, &self.context, size, self.out_size, self.fps) {
                        Ok(c) => {
                            tracing::info!("converting on the graphics card");
                            self.gpu = Some(c);
                        }
                        Err(e) => tracing::info!("converting on the CPU: {e:#}"),
                    }
                }
                if self.gpu.as_ref().is_some_and(|g| g.input_size != size) {
                    self.gpu = None;
                }
                if let Some(gpu) = &self.gpu {
                    gpu.load(&tex);
                    (self.w, self.h, self.picture) = (desc.Width as i32, desc.Height as i32, true);
                    let _ = self.dup.ReleaseFrame();
                    return Ok(true);
                }
                if self.staging.is_none() || (desc.Width as i32, desc.Height as i32) != (self.w, self.h) {
                    let want = D3D11_TEXTURE2D_DESC {
                        MipLevels: 1,
                        ArraySize: 1,
                        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                        Usage: D3D11_USAGE_STAGING,
                        BindFlags: 0,
                        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                        MiscFlags: 0,
                        ..desc
                    };
                    let mut staging = None;
                    self.device.CreateTexture2D(&want, None, Some(&mut staging))?;
                    self.staging = staging;
                    (self.w, self.h) = (desc.Width as i32, desc.Height as i32);
                }
                if let Some(staging) = &self.staging {
                    self.context.CopyResource(staging, &tex);
                    (copied, self.picture) = (true, true);
                }
            }
            let _ = self.dup.ReleaseFrame();
            Ok(copied)
        }
    }

    /// The last picture with the pointer, as NV12 through the card; None when the card doesn't
    /// convert (then [copy_into] and the CPU). A card that fails is left for the CPU from then on.
    fn convert(&mut self, display: Area, out: &mut Vec<u8>) -> Option<Result<()>> {
        let result = self.gpu.as_ref()?.convert(|dc| draw_pointer(dc, display), out);
        if let Err(e) = &result {
            tracing::warn!("the graphics card stopped converting, the CPU goes on: {e:#}");
            // The next picture goes to the staging copy instead.
            (self.gpu, self.picture) = (None, false);
        }
        Some(result)
    }

    /// The last picture into [canvas], upright; Err if it isn't the canvas's size (the display
    /// changed under us).
    fn copy_into(&self, canvas: &mut Canvas) -> Result<()> {
        let staging = self.staging.as_ref().context("no picture yet")?;
        let (dw, dh) = self.turn.upright((self.w, self.h));
        if (dw, dh) != (canvas.w, canvas.h) {
            bail!("the display is {dw}x{dh} now, not {}x{}", canvas.w, canvas.h);
        }
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: the staging texture mapped for reading (RowPitch bytes a row, h rows), copied
        // within both buffers' sizes, unmapped after.
        unsafe {
            self.context.Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            let pitch = mapped.RowPitch as usize;
            let src = std::slice::from_raw_parts(mapped.pData as *const u8, pitch * self.h as usize);
            let dst = canvas.pixels_mut();
            if self.turn == Turn::None {
                let row = self.w as usize * 4;
                for y in 0..self.h as usize {
                    dst[y * row..(y + 1) * row].copy_from_slice(&src[y * pitch..y * pitch + row]);
                }
            } else {
                turn_upright(src, pitch, self.turn, dst, dw as usize, dh as usize);
            }
            self.context.Unmap(staging, 0);
        }
        Ok(())
    }
}

/// Where the pictures come from.
enum Source {
    Duplication(Duplication),
    /// GDI, every frame.
    Copy,
}

/// Windows' H.264 encoders: the graphics card's (Intel Quick Sync, NVIDIA NVENC, AMD VCE: an
/// asynchronous Media Foundation transform that says through events when it wants a picture and
/// when it has output) when there's one that takes our pictures, else Microsoft's software one,
/// which is on every Windows with media features.
pub(crate) struct Encoder {
    mft: IMFTransform,
    /// The hardware encoder's events; None for the software one, which is called in turn.
    events: Option<IMFMediaEventGenerator>,
    /// Pictures the hardware encoder asked for and hasn't had.
    wanted: u32,
    name: String,
    fps: u32,
    pictures: i64,
    provides_samples: bool,
    out_size: u32,
}

/// How long a busy hardware encoder is waited for before a picture is skipped, and how long for
/// the output of the picture it was just given.
const HARDWARE_INPUT_WAIT: Duration = Duration::from_millis(15);
const HARDWARE_OUTPUT_WAIT: Duration = Duration::from_millis(40);

fn media_type(subtype: &windows::core::GUID, w: u32, h: u32, fps: u32) -> Result<IMFMediaType> {
    // SAFETY: a fresh media type, filled with plain values.
    unsafe {
        let t = MFCreateMediaType()?;
        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        t.SetGUID(&MF_MT_SUBTYPE, subtype)?;
        t.SetUINT64(&MF_MT_FRAME_SIZE, ((w as u64) << 32) | h as u64)?;
        t.SetUINT64(&MF_MT_FRAME_RATE, ((fps as u64) << 32) | 1)?;
        t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1 << 32) | 1)?;
        t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        Ok(t)
    }
}

/// The encoders of one kind that turn NV12 into H.264, best first.
fn encoders(flags: MFT_ENUM_FLAG) -> Result<Vec<IMFActivate>> {
    let input = MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: MFVideoFormat_NV12 };
    let output = MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: MFVideoFormat_H264 };
    let mut list: *mut Option<IMFActivate> = std::ptr::null_mut();
    let mut count = 0u32;
    // SAFETY: the list Media Foundation allocates is read once per entry (taking ownership of each
    // pointer) and freed.
    unsafe {
        MFTEnumEx(MFT_CATEGORY_VIDEO_ENCODER, flags | MFT_ENUM_FLAG_SORTANDFILTER, Some(&input), Some(&output), &mut list, &mut count)?;
        if list.is_null() {
            return Ok(Vec::new());
        }
        let found: Vec<Option<IMFActivate>> = (0..count as usize).map(|i| std::ptr::read(list.add(i))).collect();
        CoTaskMemFree(Some(list.cast()));
        Ok(found.into_iter().flatten().collect())
    }
}

fn friendly_name(activate: &IMFActivate) -> String {
    let (mut name, mut len) = (windows::core::PWSTR::null(), 0u32);
    // SAFETY: the string Media Foundation allocates is copied and freed.
    unsafe {
        if activate.GetAllocatedString(&MFT_FRIENDLY_NAME_Attribute, &mut name, &mut len).is_err() || name.is_null() {
            return "an H.264 encoder".into();
        }
        let out = name.to_string().unwrap_or_default();
        CoTaskMemFree(Some(name.0.cast()));
        out
    }
}

impl Encoder {
    /// The graphics card's encoder if one takes our pictures, else the software one.
    /// `VYOMA_SOFTWARE_ENCODER=1` skips the graphics card's.
    fn new(w: u32, h: u32, fps: u32) -> Result<Encoder> {
        // SAFETY: Media Foundation started once per process (it counts).
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_LITE).context("no Media Foundation (a Windows N edition without the Media Feature Pack?)")? };
        if std::env::var_os("VYOMA_SOFTWARE_ENCODER").is_none() {
            for activate in encoders(MFT_ENUM_FLAG_HARDWARE).unwrap_or_default() {
                let name = friendly_name(&activate);
                match Self::with(&activate, name.clone(), w, h, fps) {
                    Ok(e) => {
                        tracing::info!("encoding the screen with {name}");
                        return Ok(e);
                    }
                    Err(e) => {
                        tracing::info!("{name} won't encode this: {e:#}");
                        // SAFETY: the object it made (if any) is let go.
                        let _ = unsafe { activate.ShutdownObject() };
                    }
                }
            }
        }
        Self::software(w, h, fps)
    }

    pub(crate) fn software(w: u32, h: u32, fps: u32) -> Result<Encoder> {
        let activate = encoders(MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_LOCALMFT)?.into_iter().next().context("no H.264 encoder on this PC")?;
        let name = friendly_name(&activate);
        let e = Self::with(&activate, name.clone(), w, h, fps)?;
        tracing::info!("encoding the screen with {name} (software)");
        Ok(e)
    }

    fn with(activate: &IMFActivate, name: String, w: u32, h: u32, fps: u32) -> Result<Encoder> {
        // SAFETY: the encoder made from its activation object; an asynchronous one is unlocked
        // before anything else is asked of it, as Media Foundation requires; types are set output
        // first, as encoders want.
        unsafe {
            let mft: IMFTransform = activate.ActivateObject()?;
            let mut events = None;
            if let Ok(attrs) = mft.GetAttributes()
                && attrs.GetUINT32(&MF_TRANSFORM_ASYNC).unwrap_or(0) != 0
            {
                attrs.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1).context("couldn't unlock it")?;
                events = Some(mft.cast::<IMFMediaEventGenerator>()?);
            }
            if let Ok(codec) = mft.cast::<ICodecAPI>() {
                let _ = codec.SetValue(&CODECAPI_AVLowLatencyMode, &VARIANT::from(true));
                let _ = codec.SetValue(&CODECAPI_AVEncMPVGOPSize, &VARIANT::from(KEYFRAME_FRAMES));
                let _ = codec.SetValue(&CODECAPI_AVEncMPVDefaultBPictureCount, &VARIANT::from(0u32));
            }
            let out = media_type(&MFVideoFormat_H264, w, h, fps)?;
            out.SetUINT32(&MF_MT_AVG_BITRATE, BITRATE)?;
            out.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_Main.0 as u32)?;
            mft.SetOutputType(0, &out, 0).context("the encoder won't make this size")?;
            mft.SetInputType(0, &media_type(&MFVideoFormat_NV12, w, h, fps)?, 0).context("the encoder won't take NV12")?;
            let info = mft.GetOutputStreamInfo(0)?;
            mft.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
            mft.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
            Ok(Encoder {
                mft,
                events,
                wanted: 0,
                name,
                fps,
                pictures: 0,
                provides_samples: info.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32 != 0,
                out_size: info.cbSize.max(w * h),
            })
        }
    }

    fn hardware(&self) -> bool {
        self.events.is_some()
    }

    fn sample(&mut self, nv12: &[u8]) -> Result<IMFSample> {
        let tick = 10_000_000 / self.fps as i64;
        // SAFETY: a memory buffer of exactly nv12's length, locked while it's copied in.
        unsafe {
            let buffer = MFCreateMemoryBuffer(nv12.len() as u32)?;
            let mut data = std::ptr::null_mut();
            buffer.Lock(&mut data, None, None)?;
            std::ptr::copy_nonoverlapping(nv12.as_ptr(), data, nv12.len());
            buffer.Unlock()?;
            buffer.SetCurrentLength(nv12.len() as u32)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buffer)?;
            sample.SetSampleTime(self.pictures * tick)?;
            sample.SetSampleDuration(tick)?;
            self.pictures += 1;
            Ok(sample)
        }
    }

    /// One NV12 picture in; what came out (Annex-B), possibly nothing yet. A busy hardware encoder
    /// skips the picture instead of holding the capture up.
    pub(crate) fn encode(&mut self, nv12: &[u8]) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        if !self.hardware() {
            let sample = self.sample(nv12)?;
            // SAFETY: a complete sample for the stream the types were set on.
            unsafe { self.mft.ProcessInput(0, &sample, 0)? };
            while let Some(bytes) = self.pull()? {
                out.extend_from_slice(&bytes);
            }
            return Ok(out);
        }
        self.wait_events(&mut out, HARDWARE_INPUT_WAIT, |e| e.wanted > 0)?;
        if self.wanted == 0 {
            return Ok(out);
        }
        self.wanted -= 1;
        let sample = self.sample(nv12)?;
        // SAFETY: as above; the encoder asked for it (METransformNeedInput).
        unsafe { self.mft.ProcessInput(0, &sample, 0)? };
        self.wait_events(&mut out, HARDWARE_OUTPUT_WAIT, |_| false)?;
        Ok(out)
    }

    /// Takes the hardware encoder's events until [done] or [wait] passes (output arriving ends the
    /// wait too): input wanted is counted, output is pulled into [out].
    fn wait_events(&mut self, out: &mut Vec<u8>, wait: Duration, done: impl Fn(&Encoder) -> bool) -> Result<()> {
        let until = Instant::now() + wait;
        loop {
            let got_output = self.take_events(out)?;
            if done(self) || got_output || Instant::now() >= until {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// The hardware encoder's waiting events, without blocking; true if output came.
    fn take_events(&mut self, out: &mut Vec<u8>) -> Result<bool> {
        let Some(events) = self.events.clone() else { return Ok(false) };
        let mut got = false;
        loop {
            // SAFETY: a non-blocking read of the encoder's own event queue.
            let event = match unsafe { events.GetEvent(MF_EVENT_FLAG_NO_WAIT) } {
                Ok(e) => e,
                Err(e) if e.code() == MF_E_NO_EVENTS_AVAILABLE => return Ok(got),
                Err(e) => return Err(e.into()),
            };
            // SAFETY: reading the event's type and status.
            let (kind, status) = unsafe { (event.GetType()?, event.GetStatus()?) };
            status.ok().context("the encoder failed")?;
            if kind == METransformNeedInput.0 as u32 {
                self.wanted += 1;
            } else if kind == METransformHaveOutput.0 as u32
                && let Some(bytes) = self.pull()?
            {
                out.extend_from_slice(&bytes);
                got = true;
            }
        }
    }

    /// One output, None when the encoder wants more input.
    fn pull(&mut self) -> Result<Option<Vec<u8>>> {
        // SAFETY: the output buffer's sample and events are taken out of their ManuallyDrop
        // exactly once, whatever ProcessOutput returned.
        unsafe {
            let ours: Option<IMFSample> = if self.provides_samples {
                None
            } else {
                let s = MFCreateSample()?;
                s.AddBuffer(&MFCreateMemoryBuffer(self.out_size)?)?;
                Some(s)
            };
            let mut buffers = [MFT_OUTPUT_DATA_BUFFER { dwStreamID: 0, pSample: std::mem::ManuallyDrop::new(ours), dwStatus: 0, pEvents: std::mem::ManuallyDrop::new(None) }];
            let mut status = 0u32;
            let result = self.mft.ProcessOutput(0, &mut buffers, &mut status);
            let sample = std::mem::ManuallyDrop::take(&mut buffers[0].pSample);
            drop(std::mem::ManuallyDrop::take(&mut buffers[0].pEvents));
            match result {
                Ok(()) => {}
                Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(None),
                Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                    // The encoder settled its own output type: take it and go on.
                    let t = self.mft.GetOutputAvailableType(0, 0)?;
                    self.mft.SetOutputType(0, &t, 0)?;
                    return Ok(Some(Vec::new()));
                }
                Err(e) => return Err(e.into()),
            }
            let Some(sample) = sample else { return Ok(Some(Vec::new())) };
            let buffer = sample.ConvertToContiguousBuffer()?;
            let (mut data, mut len) = (std::ptr::null_mut(), 0u32);
            buffer.Lock(&mut data, None, Some(&mut len))?;
            let bytes = std::slice::from_raw_parts(data, len as usize).to_vec();
            buffer.Unlock()?;
            Ok(Some(bytes))
        }
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        // SAFETY: ending the stream and, for an asynchronous encoder, shutting its event queue
        // down, as Media Foundation asks before it's let go.
        unsafe {
            let _ = self.mft.ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0);
            if let Ok(shutdown) = self.mft.cast::<IMFShutdown>() {
                let _ = shutdown.Shutdown();
            }
        }
    }
}

/// A running cast: the capture thread stops when this is dropped.
pub struct Cast {
    pub size: (i32, i32),
    pub display: Area,
    stop: Arc<AtomicBool>,
    pending: Arc<AtomicUsize>,
}

impl Drop for Cast {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Cast {
    /// Starts capturing and encoding the main display; the encoded pieces come on the receiver.
    pub fn start(fps: u32) -> Result<(Cast, mpsc::UnboundedReceiver<Vec<u8>>)> {
        let display = main_display();
        if display.width < 2 || display.height < 2 {
            bail!("Windows didn't say how big the screen is");
        }
        let size = fit((display.width, display.height));
        let (stop, pending) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicUsize::new(0)));
        let (tx, rx) = mpsc::unbounded_channel();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();
        let (s, p) = (stop.clone(), pending.clone());
        std::thread::Builder::new().name("vyoma-cast".into()).spawn(move || {
            // SAFETY: this thread joins the multithreaded apartment for its COM objects.
            let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            capture(display, size, fps, s, p, tx, ready_tx);
        })?;
        ready_rx.recv_timeout(Duration::from_secs(10)).context("the screen didn't start")??;
        Ok((Cast { size, display, stop, pending }, rx))
    }
}

/// The capture thread: pictures in, framed H.264 out, until [stop].
fn capture(display: Area, (w, h): (i32, i32), fps: u32, stop: Arc<AtomicBool>, pending: Arc<AtomicUsize>, out: mpsc::UnboundedSender<Vec<u8>>, ready: std::sync::mpsc::Sender<Result<()>>) {
    let setup = || -> Result<(Canvas, Encoder)> { Ok((Canvas::new(display.width, display.height)?, Encoder::new(w as u32, h as u32, fps)?)) };
    let (mut canvas, mut encoder) = match setup() {
        Ok(x) => x,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let out_size = (w as u32, h as u32);
    let mut source = match Duplication::open(display, out_size, fps) {
        Ok(d) => Source::Duplication(d),
        Err(e) => {
            tracing::info!("no desktop duplication, copying the screen instead: {e:#}");
            Source::Copy
        }
    };
    let _ = ready.send(Ok(()));
    let every = Duration::from_secs(1) / fps;
    let mut framer = Framer::default();
    let mut nv12 = Vec::new();
    let mut last_pointer = POINT::default();
    let mut next = Instant::now();
    while !stop.load(Ordering::Relaxed) && !out.is_closed() {
        let changed = match &mut source {
            Source::Duplication(d) => match d.next(every) {
                Ok(false) => false,
                Ok(true) if d.gpu.is_some() => true,
                Ok(true) => match d.copy_into(&mut canvas) {
                    Ok(()) => true,
                    Err(e) => {
                        // Not the size the stream was set up for: GDI copies at that size instead.
                        tracing::info!("copying the screen instead of duplicating it: {e:#}");
                        source = Source::Copy;
                        false
                    }
                },
                Err(e) => {
                    // The lock screen or a new display mode: wait, then duplicate again.
                    tracing::debug!("duplication lost: {e}");
                    std::thread::sleep(RETRY);
                    if let Ok(d) = Duplication::open(display, out_size, fps) {
                        source = Source::Duplication(d);
                    }
                    false
                }
            },
            Source::Copy => {
                let now = Instant::now();
                if now < next {
                    std::thread::sleep(next - now);
                }
                next = Instant::now() + every;
                gdi_copy(&canvas, display)
            }
        };
        // Desktop duplication says nothing of the pointer moving over a still screen: looked at here.
        let mut pointer = POINT::default();
        // SAFETY: GetCursorPos writes one POINT.
        let moved = unsafe { windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pointer) }.is_ok() && (pointer.x, pointer.y) != (last_pointer.x, last_pointer.y);
        if !changed && !moved {
            continue;
        }
        if moved {
            last_pointer = pointer;
            if let Source::Duplication(d) = &source
                && (d.size().is_none() || (d.gpu.is_none() && d.copy_into(&mut canvas).is_err()))
            {
                continue;
            }
        }
        if pending.load(Ordering::Relaxed) > MAX_PENDING {
            continue;
        }
        let on_card = match &mut source {
            Source::Duplication(d) => d.convert(display, &mut nv12),
            Source::Copy => None,
        };
        match on_card {
            Some(Ok(())) => {}
            // The CPU takes over from the next picture.
            Some(Err(_)) => continue,
            None => {
                canvas.draw_pointer(display);
                to_nv12(canvas.pixels(), display.width as usize * 4, display.width as usize, display.height as usize, w as usize, h as usize, &mut nv12);
            }
        }
        match encoder.encode(&nv12) {
            Ok(annexb) if !annexb.is_empty() => {
                let piece = framer.frame(&annexb, now_us());
                pending.fetch_add(piece.len(), Ordering::Relaxed);
                if out.send(piece).is_err() {
                    break;
                }
            }
            Ok(_) => {}
            Err(e) if encoder.hardware() => {
                // Some graphics cards' encoders take the format, then refuse pictures in memory:
                // the software encoder takes over, starting with a keyframe of its own.
                tracing::warn!("{} stopped, going on with the software encoder: {e:#}", encoder.name);
                match Encoder::software(w as u32, h as u32, fps) {
                    Ok(e) => {
                        encoder = e;
                        framer = Framer::default();
                    }
                    Err(e) => {
                        tracing::warn!("no software encoder either: {e:#}");
                        break;
                    }
                }
            }
            Err(e) => {
                tracing::warn!("the encoder stopped: {e:#}");
                break;
            }
        }
    }
}

/// The display through GDI into [canvas]; false if Windows wouldn't hand it over (the lock screen).
fn gdi_copy(canvas: &Canvas, display: Area) -> bool {
    // SAFETY: the screen's DC, released after one BitBlt into our DC.
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return false;
        }
        let ok = BitBlt(canvas.dc, 0, 0, display.width, display.height, Some(screen), display.left, display.top, SRCCOPY | CAPTUREBLT).is_ok();
        ReleaseDC(None, screen);
        ok
    }
}

/// Casts until the phone hangs up or the stream fails. Failures before it starts reach the phone
/// as an error, so it never waits on a spinner.
pub async fn stream(sender: &Sender, me: &Identity, pinned: &[u8], name: &str, fps: u32, inputs: mpsc::UnboundedReceiver<Value>) -> Result<()> {
    let started = tokio::task::spawn_blocking(move || Cast::start(fps)).await.context("the cast didn't start")?;
    let (cast, mut video) = match started {
        Ok(c) => c,
        Err(e) => {
            sender.send(&packet(SCREEN, json!({ "error": format!("{e:#}") }))).await?;
            return Err(e);
        }
    };
    let (w, h) = cast.size;
    tracing::info!("{name}: casting the screen at {w}x{h}, {fps} a second");
    crate::tray::notify("Your phone is showing this screen", &format!("{name} can see and work this PC's screen until it closes it."));
    let (relay, source) = tokio::io::duplex(RELAY_BYTES);
    let pending = cast.pending.clone();
    let write = async move {
        use tokio::io::AsyncWriteExt;
        let mut relay = relay;
        while let Some(piece) = video.recv().await {
            let n = piece.len();
            if relay.write_all(&piece).await.is_err() {
                break;
            }
            pending.fetch_sub(n, Ordering::Relaxed);
        }
    };
    // Both end with this cast however it ends (the phone's stop aborts this task).
    let _writer = AbortOnDrop(tokio::spawn(write));
    let _touching = AbortOnDrop(tokio::spawn(follow(cast.display, inputs)));
    let announce = packet(SCREEN, json!({ "codec": "h264", "framing": "avc4", "width": w, "height": h, "control": true }));
    let result = payload::offer_stream(sender, me, pinned, announce, source).await;
    drop(cast);
    if let Err(e) = &result {
        tracing::info!("{name}: cast ended ({e:#})");
    }
    Ok(())
}

struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Does the phone's touches on [display]; a drag cut off mid-way is let go.
async fn follow(display: Area, mut inputs: mpsc::UnboundedReceiver<Value>) {
    let mut pointer = input::Pointer::default();
    pointer.cast_on(display);
    let mut held = false;
    while let Some(mut i) = inputs.recv().await {
        // Behind? Only the newest of a run of moves matters.
        while i["kind"] == "move" {
            match inputs.try_recv() {
                Ok(next) if next["kind"] == "move" => i = next,
                Ok(next) => {
                    let _ = pointer.handle(&input::touch_acts(&i));
                    i = next;
                    break;
                }
                Err(_) => break,
            }
        }
        match i["kind"].as_str() {
            Some("down") => held = true,
            Some("up") => held = false,
            _ => {}
        }
        let _ = pointer.handle(&input::touch_acts(&i));
    }
    if held {
        let _ = pointer.handle(&[Act::Button(Button::Left, false)]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_rates() {
        assert_eq!(fit((3840, 2160)), (1920, 1080));
        assert_eq!(fit((1366, 768)), (1366, 768));
        assert_eq!(fit((2560, 1601)), (1920, 1200));
        assert_eq!((fps_for(None), fps_for(Some(120)), fps_for(Some(10))), (30, 60, 30));
    }

    #[test]
    fn the_encoders_output_becomes_the_phones_framing() {
        // SPS, PPS, IDR slice, with both start code lengths.
        let annexb = [0, 0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x68, 3, 0, 0, 0, 1, 0x65, 9, 9, 0];
        let units = nal_units(&annexb);
        assert_eq!(units, [&[0x67, 1, 2][..], &[0x68, 3][..], &[0x65, 9, 9, 0][..]], "a trailing zero inside the last unit stays");
        let mut f = Framer::default();
        let out = f.frame(&annexb, 7);
        // SPS, PPS, then the stamp, then the slice.
        assert_eq!(&out[..7], &[0, 0, 0, 3, 0x67, 1, 2]);
        assert_eq!(&out[7..13], &[0, 0, 0, 2, 0x68, 3]);
        assert_eq!(&out[13..22], &[0, 0, 0, 13, 0x06, b'V', b'Y', b'T', b'S']);
        assert_eq!(u64::from_be_bytes(out[22..30].try_into().unwrap()), 7);
        // A later keyframe without parameter sets gets the kept ones.
        let again = f.frame(&[0, 0, 1, 0x65, 5], 8);
        assert_eq!(&again[..7], &[0, 0, 0, 3, 0x67, 1, 2]);
        // An ordinary picture: just the stamp and the slice.
        let p = f.frame(&[0, 0, 1, 0x41, 5], 9);
        assert_eq!(p.len(), 4 + 13 + 4 + 2);
    }

    #[test]
    fn sideways_pictures_are_turned_upright() {
        // A 3x2 picture whose pixels are their own numbers, as Microsoft's sample places them.
        let (pw, ph) = (3usize, 2usize);
        let pitch = pw * 4 + 4; // rows padded, as mapped textures are
        let mut src = vec![0u8; pitch * ph];
        for y in 0..ph {
            for x in 0..pw {
                src[y * pitch + x * 4] = (y * pw + x) as u8;
            }
        }
        let placed = |turn: Turn, sx: usize, sy: usize, dw: usize, dh: usize| match turn {
            Turn::None => (sx, sy),
            Turn::Quarter => (dw - 1 - sy, sx),
            Turn::Half => (dw - 1 - sx, dh - 1 - sy),
            Turn::ThreeQuarters => (sy, dh - 1 - sx),
        };
        for turn in [Turn::None, Turn::Quarter, Turn::Half, Turn::ThreeQuarters] {
            let (dw, dh) = turn.upright((pw as i32, ph as i32));
            let (dw, dh) = (dw as usize, dh as usize);
            let mut dst = vec![0xffu8; dw * dh * 4];
            turn_upright(&src, pitch, turn, &mut dst, dw, dh);
            for sy in 0..ph {
                for sx in 0..pw {
                    let (dx, dy) = placed(turn, sx, sy, dw, dh);
                    assert_eq!(dst[(dy * dw + dx) * 4], (sy * pw + sx) as u8, "{turn:?} at {sx},{sy}");
                }
            }
        }
        assert_eq!(Turn::Quarter.upright((1920, 1080)), (1080, 1920));
    }

    #[test]
    fn colours_become_nv12() {
        // 4x2 of white then 4x2 of black (BGRA), scaled to 2x2.
        let mut src = Vec::new();
        for y in 0..4 {
            for _ in 0..4 {
                let v = if y < 2 { 255 } else { 0 };
                src.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let mut out = Vec::new();
        to_nv12(&src, 16, 4, 4, 2, 2, &mut out);
        assert_eq!(out.len(), 6);
        assert_eq!((out[0], out[1]), (235, 235), "white is 235 in video range");
        assert_eq!((out[2], out[3]), (16, 16), "black is 16");
        assert!((out[4] as i32 - 128).abs() <= 1 && (out[5] as i32 - 128).abs() <= 1, "grey has no colour: {:?}", &out[4..]);
        // Pure red (BGRA 0,0,255).
        let red = [0u8, 0, 255, 255].repeat(4);
        to_nv12(&red, 8, 2, 2, 2, 2, &mut out);
        assert_eq!((out[0], out[4], out[5]), (82, 90, 240));
    }

    /// Whatever this PC has: under Wine there's no desktop duplication and maybe no encoder;
    /// it starts or says why, never hangs.
    #[test]
    fn starts_or_says_why() {
        match Cast::start(30) {
            Ok((cast, mut rx)) => {
                let first = rx.blocking_recv();
                eprintln!("cast {:?}, first piece {} bytes", cast.size, first.as_ref().map_or(0, |p| p.len()));
                // VYOMA_CAST_DUMP=<file>: two seconds of the stream as sent, to check with a decoder.
                if let Some(path) = std::env::var_os("VYOMA_CAST_DUMP") {
                    let mut all = first.unwrap_or_default();
                    let until = Instant::now() + Duration::from_secs(2);
                    while Instant::now() < until {
                        match rx.try_recv() {
                            Ok(p) => all.extend_from_slice(&p),
                            Err(_) => std::thread::sleep(Duration::from_millis(20)),
                        }
                    }
                    std::fs::write(path, all).unwrap();
                }
            }
            Err(e) => eprintln!("no cast here: {e:#}"),
        }
    }
}
