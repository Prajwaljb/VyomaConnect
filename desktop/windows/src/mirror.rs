//! The phone's screen in a window on this PC, worked with the mouse and keyboard (core's
//! [vyoma_core::mirror]: the phone sends live H.264, the window's clicks and keys go back as
//! `vyoma.mirror.input`). The phone's sound plays here too (`vyoma.mirror.audio`, PCM, through
//! WASAPI); the phone copying text puts it on this PC's clipboard, and its volume keys turn this
//! PC's speaker up and down while its sound plays here.
//!
//! The window is this same program run as a child (`--mirror-window <name>`), as on Linux and the
//! Mac, so a decoder problem can't take the agent down. It reads the stream on stdin, decodes it
//! with Windows' own H.264 decoder (Media Foundation, low latency), draws it with GDI (aspect kept,
//! black bars), and writes what the person does as JSON lines on stdout. Keys as on Windows:
//! Ctrl+C/X/A/V copy, cut, select all and paste (this PC's clipboard, typed into the phone),
//! Ctrl+arrows and Ctrl+Backspace by word, Escape, a right click or the mouse's back button is
//! Back, a middle click Home, Ctrl+W closes.
//! Built and tested on CI and under Wine only; not yet run on a real Windows PC.

use std::net::IpAddr;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use vyoma_core::identity::Identity;
use vyoma_core::link::{Sender, packet};
use vyoma_core::mirror::{INPUT, PASTE_MAX, STATE};
use vyoma_core::payload;
use windows::Win32::Media::MediaFoundation::{
    CODECAPI_AVLowLatencyMode, ICodecAPI, IMF2DBuffer, IMFActivate, IMFMediaType, IMFSample, IMFTransform, MF_E_NOTACCEPTING, MF_E_TRANSFORM_NEED_MORE_INPUT,
    MF_E_TRANSFORM_STREAM_CHANGE, MF_LOW_LATENCY, MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE, MF_MT_MINIMUM_DISPLAY_APERTURE, MF_MT_SUBTYPE,
    MF_VERSION, MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample, MFMediaType_Video, MFSTARTUP_LITE, MFStartup, MFT_CATEGORY_VIDEO_DECODER,
    MFT_ENUM_FLAG_LOCALMFT, MFT_ENUM_FLAG_SORTANDFILTER, MFT_ENUM_FLAG_SYNCMFT, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, MFT_MESSAGE_NOTIFY_START_OF_STREAM,
    MFT_OUTPUT_DATA_BUFFER, MFT_OUTPUT_STREAM_PROVIDES_SAMPLES, MFT_REGISTER_TYPE_INFO, MFTEnumEx, MFVideoFormat_H264, MFVideoFormat_NV12,
};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Variant::VARIANT;
use windows::core::Interface;

pub const WINDOW_ARG: &str = "--mirror-window";

async fn state(sender: &Sender, body: Value) {
    let _ = sender.send(&packet(STATE, body)).await;
}

/// Shows the phone's screen until the phone stops or the window is closed.
pub async fn show(sender: &Sender, me: &Identity, pinned: &[u8], phone: IpAddr, p: &Value, name: &str) -> Result<()> {
    let mut input = payload::fetch_stream(me, pinned, phone, p).await?;
    let child = tokio::process::Command::new(std::env::current_exe()?)
        .args([WINDOW_ARG, name])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            state(sender, json!({ "open": false, "error": format!("Couldn't open the window: {e}") })).await;
            return Err(e.into());
        }
    };
    let mut stdin = child.stdin.take().context("no stdin")?;
    let mut lines = BufReader::new(child.stdout.take().context("no stdout")?).lines();
    // Sound as PCM: WASAPI plays it with nothing to decode.
    state(sender, json!({ "open": true, "sound": [] })).await;
    tracing::info!("{name}: mirroring the phone's screen");
    let feed = async {
        let _ = tokio::io::copy(&mut input, &mut stdin).await;
        // The phone stopped: the window sees the end of the stream and closes.
        drop(stdin);
    };
    let inputs = async {
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(body) = serde_json::from_str::<Value>(&line) else { continue };
            let body = if body["kind"] == "paste" {
                match tokio::task::spawn_blocking(crate::win::clipboard).await.ok().flatten() {
                    Some(text) => json!({ "kind": "text", "text": text.chars().take(PASTE_MAX).collect::<String>() }),
                    None => continue,
                }
            } else {
                body
            };
            let _ = sender.send(&packet(INPUT, body)).await;
        }
    };
    tokio::select! {
        _ = feed => {
            let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
        }
        // The window closed (its stdout ends with it).
        _ = inputs => {}
    }
    state(sender, json!({ "open": false })).await;
    tracing::info!("{name}: mirror closed");
    Ok(())
}

// ---- What the window's keys and mouse mean on the phone

const VK_BACK: u16 = 0x08;
const VK_TAB: u16 = 0x09;
const VK_RETURN: u16 = 0x0D;
const VK_ESCAPE: u16 = 0x1B;
const VK_END: u16 = 0x23;
const VK_HOME: u16 = 0x24;
const VK_LEFT: u16 = 0x25;
const VK_UP: u16 = 0x26;
const VK_RIGHT: u16 = 0x27;
const VK_DOWN: u16 = 0x28;
const VK_DELETE: u16 = 0x2E;

/// A key press that isn't typing ([vk] a Windows virtual key); typing comes as characters
/// ([char_input]). AltGr is Ctrl+Alt on Windows: those type characters, so they're no shortcut.
pub fn key_input(vk: u16, ctrl: bool, shift: bool, alt: bool) -> Option<Value> {
    let key = |k: &str, word: bool| Some(json!({ "kind": "key", "key": k, "ctrl": word, "shift": shift }));
    let shortcut = ctrl && !alt;
    match vk {
        VK_ESCAPE => Some(json!({ "kind": "back" })),
        VK_TAB if !ctrl && !alt => Some(json!({ "kind": "text", "text": "\t" })),
        VK_BACK => key("backspace", shortcut),
        VK_DELETE => key("delete", shortcut),
        VK_RETURN => key("enter", false),
        VK_LEFT => key("left", shortcut),
        VK_RIGHT => key("right", shortcut),
        VK_UP => key("up", false),
        VK_DOWN => key("down", false),
        VK_HOME => key("home", false),
        VK_END => key("end", false),
        0x41 if shortcut => Some(json!({ "kind": "key", "key": "selectall" })),
        0x43 if shortcut => Some(json!({ "kind": "key", "key": "copy" })),
        0x58 if shortcut => Some(json!({ "kind": "key", "key": "cut" })),
        0x56 if shortcut => Some(json!({ "kind": "paste" })),
        _ => None,
    }
}

/// A typed character; control characters (Enter, Backspace, Ctrl+letters) are keys, not text.
pub fn char_input(c: char) -> Option<Value> {
    (!c.is_control()).then(|| json!({ "kind": "text", "text": c.to_string() }))
}

/// Where a [pw]×[ph] picture goes in a [cw]×[ch] client area: as large as fits, centred.
pub fn letterbox((cw, ch): (i32, i32), (pw, ph): (i32, i32)) -> (i32, i32, i32, i32) {
    if pw <= 0 || ph <= 0 || cw <= 0 || ch <= 0 {
        return (0, 0, cw.max(0), ch.max(0));
    }
    let (w, h) = if cw as i64 * ph as i64 > ch as i64 * pw as i64 { ((ch as i64 * pw as i64 / ph as i64) as i32, ch) } else { (cw, (cw as i64 * ph as i64 / pw as i64) as i32) };
    ((cw - w) / 2, (ch - h) / 2, w, h)
}

/// A point in the client area as 0..1 across and down the picture at [rect].
pub fn at((x, y): (i32, i32), (rx, ry, rw, rh): (i32, i32, i32, i32)) -> (f64, f64) {
    (((x - rx) as f64 / rw.max(1) as f64).clamp(0.0, 1.0), ((y - ry) as f64 / rh.max(1) as f64).clamp(0.0, 1.0))
}

/// The window's picture size for a [pw]×[ph] picture on a [aw]×[ah] work area: its pixels, at
/// most 85% of the area, shape kept.
pub fn fit_picture((pw, ph): (i32, i32), (aw, ah): (i32, i32)) -> (i32, i32) {
    let (pw, ph) = (pw.max(1) as f64, ph.max(1) as f64);
    let shrink = (aw as f64 * 0.85 / pw).min(ah as f64 * 0.85 / ph).min(1.0);
    ((pw * shrink).round() as i32, (ph * shrink).round() as i32)
}

/// NV12 ([pitch] bytes a row; the chroma plane at [uv]) cropped to [w]×[h] from ([ox], [oy]),
/// into top-down BGRA (BT.601, video range, as Android's encoders write it).
#[allow(clippy::too_many_arguments)]
pub fn nv12_to_bgra(y_plane: &[u8], uv: &[u8], pitch: usize, (ox, oy): (usize, usize), w: usize, h: usize, out: &mut Vec<u8>) {
    out.clear();
    out.resize(w * h * 4, 255);
    for row in 0..h {
        let (sy, cy) = (oy + row, (oy + row) / 2);
        let dst = &mut out[row * w * 4..(row + 1) * w * 4];
        for (col, px) in dst.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let sx = ox + col;
            let c = y_plane[sy * pitch + sx] as i32 - 16;
            let ci = cy * pitch + (sx & !1);
            let (d, e) = (uv[ci] as i32 - 128, uv[ci + 1] as i32 - 128);
            px[0] = ((298 * c + 516 * d + 128) >> 8).clamp(0, 255) as u8;
            px[1] = ((298 * c - 100 * d - 208 * e + 128) >> 8).clamp(0, 255) as u8;
            px[2] = ((298 * c + 409 * e + 128) >> 8).clamp(0, 255) as u8;
        }
    }
}

// ---- Windows' H.264 decoder

pub use vyoma_camera::frame::View;

/// The decoder's pictures as they come out: their place in the buffer and their visible size.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Layout {
    /// The allocated size (rows of luma: often padded, 1080 → 1088).
    alloc_w: usize,
    alloc_h: usize,
    stride: usize,
    /// What's shown.
    offset: (usize, usize),
    w: usize,
    h: usize,
}

pub struct Decoder {
    mft: IMFTransform,
    layout: Option<Layout>,
    provides_samples: bool,
    out_size: u32,
    pictures: i64,
}

fn decoders() -> Result<Vec<IMFActivate>> {
    let input = MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: MFVideoFormat_H264 };
    let output = MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: MFVideoFormat_NV12 };
    let mut list: *mut Option<IMFActivate> = std::ptr::null_mut();
    let mut count = 0u32;
    // SAFETY: the list Media Foundation allocates is read once per entry and freed.
    unsafe {
        MFTEnumEx(MFT_CATEGORY_VIDEO_DECODER, MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_LOCALMFT | MFT_ENUM_FLAG_SORTANDFILTER, Some(&input), Some(&output), &mut list, &mut count)?;
        if list.is_null() {
            return Ok(Vec::new());
        }
        let found: Vec<Option<IMFActivate>> = (0..count as usize).map(|i| std::ptr::read(list.add(i))).collect();
        CoTaskMemFree(Some(list.cast()));
        Ok(found.into_iter().flatten().collect())
    }
}

impl Decoder {
    pub fn new() -> Result<Decoder> {
        // SAFETY: Media Foundation started (it counts); the decoder made from its activation
        // object, told to hand pictures over as soon as it has them, and given its input type.
        unsafe {
            MFStartup(MF_VERSION, MFSTARTUP_LITE).context("no Media Foundation (a Windows N edition without the Media Feature Pack?)")?;
            let activate = decoders()?.into_iter().next().context("no H.264 decoder on this PC")?;
            let mft: IMFTransform = activate.ActivateObject()?;
            if let Ok(attrs) = mft.GetAttributes() {
                let _ = attrs.SetUINT32(&MF_LOW_LATENCY, 1);
            }
            if let Ok(codec) = mft.cast::<ICodecAPI>() {
                let _ = codec.SetValue(&CODECAPI_AVLowLatencyMode, &VARIANT::from(true));
            }
            let t = MFCreateMediaType()?;
            t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            t.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
            mft.SetInputType(0, &t, 0).context("the decoder won't take H.264")?;
            let mut d = Decoder { mft, layout: None, provides_samples: false, out_size: 0, pictures: 0 };
            d.pick_output()?;
            d.mft.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
            d.mft.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
            Ok(d)
        }
    }

    /// Chooses NV12 among what the decoder offers (again after the stream's size changes), and
    /// reads how its pictures are laid out.
    fn pick_output(&mut self) -> Result<()> {
        // SAFETY: the decoder's own types, read and set back.
        unsafe {
            let mut i = 0;
            let chosen = loop {
                let Ok(t) = self.mft.GetOutputAvailableType(0, i) else { bail!("the decoder offers no NV12") };
                if t.GetGUID(&MF_MT_SUBTYPE).is_ok_and(|g| g == MFVideoFormat_NV12) {
                    break t;
                }
                i += 1;
            };
            self.mft.SetOutputType(0, &chosen, 0)?;
            self.layout = layout(&chosen);
            let info = self.mft.GetOutputStreamInfo(0)?;
            self.provides_samples = info.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32 != 0;
            self.out_size = info.cbSize;
        }
        Ok(())
    }

    /// One access unit (Annex-B, start codes included) in; each picture that came out, as BGRA,
    /// to [shown] with its size.
    pub fn decode(&mut self, unit: &[u8], mut shown: impl FnMut(usize, usize, &[u8])) -> Result<()> {
        let mut bgra = Vec::new();
        self.decode_nv12(unit, |v| {
            nv12_to_bgra(v.y, v.uv, v.pitch, v.offset, v.w, v.h, &mut bgra);
            if bgra.len() == v.w * v.h * 4 {
                shown(v.w, v.h, &bgra);
            }
        })
    }

    /// One access unit in; each picture that came out as the decoder lays it out (NV12).
    pub fn decode_nv12(&mut self, unit: &[u8], mut seen: impl FnMut(&View)) -> Result<()> {
        let sample = self.sample(unit)?;
        // SAFETY: a complete sample for the stream the types were set on.
        match unsafe { self.mft.ProcessInput(0, &sample, 0) } {
            Ok(()) => {}
            // Full: what it has comes out first, then it takes this one.
            Err(e) if e.code() == MF_E_NOTACCEPTING => {
                while self.pull(&mut seen)? {}
                // SAFETY: as above.
                unsafe { self.mft.ProcessInput(0, &sample, 0)? };
            }
            Err(e) => return Err(e.into()),
        }
        while self.pull(&mut seen)? {}
        Ok(())
    }

    fn sample(&mut self, unit: &[u8]) -> Result<IMFSample> {
        // SAFETY: a memory buffer of exactly the unit's length, locked while it's copied in.
        unsafe {
            let buffer = MFCreateMemoryBuffer(unit.len() as u32)?;
            let mut data = std::ptr::null_mut();
            buffer.Lock(&mut data, None, None)?;
            std::ptr::copy_nonoverlapping(unit.as_ptr(), data, unit.len());
            buffer.Unlock()?;
            buffer.SetCurrentLength(unit.len() as u32)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buffer)?;
            sample.SetSampleTime(self.pictures * 166_666)?;
            self.pictures += 1;
            Ok(sample)
        }
    }

    /// One picture out, if there is one; false when the decoder wants more input.
    fn pull(&mut self, seen: &mut impl FnMut(&View)) -> Result<bool> {
        // SAFETY: the output buffer's sample and events are taken out of their ManuallyDrop
        // exactly once; the picture's buffer is locked while it's converted.
        unsafe {
            let ours: Option<IMFSample> = if self.provides_samples {
                None
            } else {
                let s = MFCreateSample()?;
                s.AddBuffer(&MFCreateMemoryBuffer(self.out_size.max(1))?)?;
                Some(s)
            };
            let mut buffers = [MFT_OUTPUT_DATA_BUFFER { dwStreamID: 0, pSample: std::mem::ManuallyDrop::new(ours), dwStatus: 0, pEvents: std::mem::ManuallyDrop::new(None) }];
            let mut status = 0u32;
            let result = self.mft.ProcessOutput(0, &mut buffers, &mut status);
            let sample = std::mem::ManuallyDrop::take(&mut buffers[0].pSample);
            drop(std::mem::ManuallyDrop::take(&mut buffers[0].pEvents));
            match result {
                Ok(()) => {}
                Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(false),
                Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                    // The stream said its size (or changed it: the phone turned): choose again.
                    self.pick_output()?;
                    return Ok(true);
                }
                Err(e) => return Err(e.into()),
            }
            let (Some(sample), Some(l)) = (sample, self.layout) else { return Ok(true) };
            let buffer = sample.GetBufferByIndex(0)?;
            if let Ok(two_d) = buffer.cast::<IMF2DBuffer>() {
                let (mut scan0, mut pitch) = (std::ptr::null_mut(), 0i32);
                two_d.Lock2D(&mut scan0, &mut pitch)?;
                let pitch = pitch.unsigned_abs() as usize;
                let y = std::slice::from_raw_parts(scan0, pitch * l.alloc_h);
                let uv = std::slice::from_raw_parts(scan0.add(pitch * l.alloc_h), pitch * l.alloc_h.div_ceil(2));
                seen(&View { y, uv, pitch, offset: l.offset, w: l.w, h: l.h });
                two_d.Unlock2D()?;
            } else {
                let (mut data, mut len) = (std::ptr::null_mut(), 0u32);
                buffer.Lock(&mut data, None, Some(&mut len))?;
                let all = std::slice::from_raw_parts(data, len as usize);
                let luma = l.stride * l.alloc_h;
                if all.len() >= luma + l.stride * l.alloc_h.div_ceil(2) {
                    seen(&View { y: &all[..luma], uv: &all[luma..], pitch: l.stride, offset: l.offset, w: l.w, h: l.h });
                }
                buffer.Unlock()?;
            }
            Ok(true)
        }
    }
}

/// How the decoder lays its NV12 pictures out: the allocated size, the row stride, and the part
/// that's shown (the minimum display aperture, when it says one).
fn layout(t: &IMFMediaType) -> Option<Layout> {
    // SAFETY: plain reads of the type's attributes.
    unsafe {
        let size = t.GetUINT64(&MF_MT_FRAME_SIZE).ok()?;
        let (alloc_w, alloc_h) = ((size >> 32) as usize, (size & 0xffff_ffff) as usize);
        if alloc_w == 0 || alloc_h == 0 {
            return None;
        }
        let stride = t.GetUINT32(&MF_MT_DEFAULT_STRIDE).map(|s| (s as i32).unsigned_abs() as usize).unwrap_or(alloc_w).max(alloc_w);
        let mut area = [0u8; 16];
        let mut l = Layout { alloc_w, alloc_h, stride, offset: (0, 0), w: alloc_w, h: alloc_h };
        // MFVideoArea: two MFOffsets ({fract: u16, value: i16}) and a SIZE (two i32s).
        if t.GetBlob(&MF_MT_MINIMUM_DISPLAY_APERTURE, &mut area, None).is_ok() {
            let ox = i16::from_le_bytes([area[2], area[3]]).max(0) as usize;
            let oy = i16::from_le_bytes([area[6], area[7]]).max(0) as usize;
            let w = i32::from_le_bytes([area[8], area[9], area[10], area[11]]).max(0) as usize;
            let h = i32::from_le_bytes([area[12], area[13], area[14], area[15]]).max(0) as usize;
            if w > 0 && h > 0 && ox + w <= alloc_w && oy + h <= alloc_h {
                (l.offset, l.w, l.h) = ((ox, oy), w, h);
            }
        }
        // Even sizes: NV12's chroma is shared by 2×2 pixels.
        Some(Layout { w: l.w & !1, h: l.h & !1, ..l })
    }
}

/// Groups NAL units into what the decoder takes at once: the parameter sets and other units that
/// come before a picture's slice go in with it.
#[derive(Default)]
pub struct Units {
    pending: Vec<u8>,
}

impl Units {
    /// One NAL unit (no start code); an access unit (Annex-B) when it ends one.
    pub fn push(&mut self, nal: &[u8]) -> Option<Vec<u8>> {
        self.pending.extend_from_slice(&[0, 0, 0, 1]);
        self.pending.extend_from_slice(nal);
        let slice = nal.first().is_some_and(|h| matches!(h & 0x1f, 1 | 5));
        slice.then(|| std::mem::take(&mut self.pending))
    }
}

/// The window process (`--mirror-window <title>`): H.264 on stdin, a window, inputs on stdout.
/// Exits when the window closes or the stream ends.
pub fn window(args: &[String]) -> Result<()> {
    let title = args.first().cloned().unwrap_or_else(|| "Phone".into());
    view::run(&title, std::io::stdin())
}

/// The phone's volume keys while its sound plays here: this PC's speaker, a step a press.
pub fn volume(up: bool) {
    crate::media::step_volume(up);
}

mod view {
    use std::io::{Read, Write};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

    use anyhow::Result;
    use serde_json::{Value, json};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLACK_BRUSH, BeginPaint, DIB_RGB_COLORS, EndPaint, ExcludeClipRect, FillRect, GetMonitorInfoW, GetStockObject, HALFTONE,
        InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow, PAINTSTRUCT, SRCCOPY, ScreenToClient, SelectClipRgn, SetBrushOrgEx, SetStretchBltMode,
        StretchDIBits,
    };
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetCapture, GetKeyState, ReleaseCapture, SetCapture, VK_CONTROL, VK_MENU, VK_SHIFT};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AdjustWindowRectEx, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW, IDC_ARROW, LoadCursorW, LoadIconW,
        MINMAXINFO, MSG, PostMessageW, PostQuitMessage, RegisterClassExW, SW_SHOW, SWP_NOACTIVATE, SWP_NOZORDER, SetForegroundWindow, SetWindowPos, ShowWindow,
        TranslateMessage, WM_APP, WM_CHAR, WM_CLOSE, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_GETMINMAXINFO, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP,
        WM_MBUTTONDOWN, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_PAINT, WM_RBUTTONDOWN, WM_SYSKEYDOWN, WM_XBUTTONDOWN, WNDCLASSEXW, WS_OVERLAPPEDWINDOW,
    };

    use super::{Decoder, Units, at, char_input, fit_picture, key_input, letterbox};
    use crate::win::wide;

    /// A new picture is in [FRAME] (wparam 1 when its size changed).
    const WM_FRAME: u32 = WM_APP + 1;
    const CLASS: &str = "VyomaMirror";
    /// One wheel notch.
    const NOTCH: f64 = 120.0;

    #[derive(Default)]
    struct Frame {
        w: usize,
        h: usize,
        bgra: Vec<u8>,
    }

    static FRAME: Mutex<Frame> = Mutex::new(Frame { w: 0, h: 0, bgra: Vec::new() });
    static WINDOW: AtomicPtr<core::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());
    /// Pictures shown so far (for the tests).
    pub static SHOWN: AtomicUsize = AtomicUsize::new(0);
    /// A UTF-16 high surrogate waiting for its pair (characters outside the BMP: emoji).
    static HIGH: Mutex<Option<u16>> = Mutex::new(None);

    /// Writes one input line for the agent.
    fn emit(v: Value) {
        let mut out = std::io::stdout().lock();
        if writeln!(out, "{v}").and_then(|()| out.flush()).is_err() {
            // The agent's gone: nothing more to do.
            std::process::exit(0);
        }
    }

    fn picture_size() -> (i32, i32) {
        let f = FRAME.lock().unwrap();
        (f.w as i32, f.h as i32)
    }

    fn client(hwnd: HWND) -> (i32, i32) {
        let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        // SAFETY: our window's client rectangle.
        unsafe { GetClientRect(hwnd, &mut r) };
        (r.right - r.left, r.bottom - r.top)
    }

    /// Where the picture is in the window now.
    fn picture_rect(hwnd: HWND) -> (i32, i32, i32, i32) {
        letterbox(client(hwnd), picture_size())
    }

    fn point(lparam: LPARAM) -> (i32, i32) {
        ((lparam & 0xffff) as i16 as i32, ((lparam >> 16) & 0xffff) as i16 as i32)
    }

    fn down(vk: i32) -> bool {
        // SAFETY: a plain read of the keyboard's state.
        unsafe { GetKeyState(vk) < 0 }
    }

    /// The window sized to the picture: its pixels, at most 85% of the screen it's on, centred.
    fn fit(hwnd: HWND) {
        let (pw, ph) = picture_size();
        if pw == 0 || ph == 0 {
            return;
        }
        // SAFETY: our window, the monitor it's on, and plain rectangles.
        unsafe {
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info);
            let area = info.rcWork;
            let (aw, ah) = (area.right - area.left, area.bottom - area.top);
            let (w, h) = fit_picture((pw, ph), (aw, ah));
            let mut r = RECT { left: 0, top: 0, right: w, bottom: h };
            AdjustWindowRectEx(&mut r, WS_OVERLAPPEDWINDOW, 0, 0);
            let (ww, wh) = (r.right - r.left, r.bottom - r.top);
            SetWindowPos(hwnd, std::ptr::null_mut(), area.left + (aw - ww) / 2, area.top + (ah - wh) / 2, ww, wh, SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }

    fn paint(hwnd: HWND) {
        // SAFETY: painting our own window between BeginPaint and EndPaint; the picture's pixels
        // stay locked while GDI copies them.
        unsafe {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            let (cw, ch) = client(hwnd);
            let f = FRAME.lock().unwrap();
            let (x, y, w, h) = letterbox((cw, ch), (f.w as i32, f.h as i32));
            if f.w > 0 {
                // The black bars around the picture.
                ExcludeClipRect(dc, x, y, x + w, y + h);
            }
            let all = RECT { left: 0, top: 0, right: cw, bottom: ch };
            FillRect(dc, &all, GetStockObject(BLACK_BRUSH));
            SelectClipRgn(dc, std::ptr::null_mut());
            if f.w > 0 && f.bgra.len() == f.w * f.h * 4 {
                let mut info: BITMAPINFO = std::mem::zeroed();
                info.bmiHeader = BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: f.w as i32,
                    biHeight: -(f.h as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..std::mem::zeroed()
                };
                SetStretchBltMode(dc, HALFTONE);
                SetBrushOrgEx(dc, 0, 0, std::ptr::null_mut());
                StretchDIBits(dc, x, y, w, h, 0, 0, f.w as i32, f.h as i32, f.bgra.as_ptr().cast(), &info, DIB_RGB_COLORS, SRCCOPY);
            }
            drop(f);
            EndPaint(hwnd, &ps);
        }
    }

    unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        // SAFETY: Windows calls this on the window's thread with its own messages.
        unsafe {
            match msg {
                WM_FRAME => {
                    if wparam == 1 {
                        fit(hwnd);
                    }
                    InvalidateRect(hwnd, std::ptr::null(), 0);
                    0
                }
                WM_PAINT => {
                    paint(hwnd);
                    0
                }
                WM_ERASEBKGND => 1,
                WM_LBUTTONDOWN => {
                    SetCapture(hwnd);
                    let (x, y) = at(point(lparam), picture_rect(hwnd));
                    emit(json!({ "kind": "down", "x": x, "y": y }));
                    0
                }
                WM_MOUSEMOVE => {
                    if GetCapture() == hwnd {
                        let (x, y) = at(point(lparam), picture_rect(hwnd));
                        emit(json!({ "kind": "move", "x": x, "y": y }));
                    }
                    0
                }
                WM_LBUTTONUP => {
                    if GetCapture() == hwnd {
                        ReleaseCapture();
                        let (x, y) = at(point(lparam), picture_rect(hwnd));
                        emit(json!({ "kind": "up", "x": x, "y": y }));
                    }
                    0
                }
                WM_RBUTTONDOWN => {
                    emit(json!({ "kind": "back" }));
                    0
                }
                WM_MBUTTONDOWN => {
                    emit(json!({ "kind": "home" }));
                    0
                }
                // The mouse's back button (XBUTTON1) is Back.
                WM_XBUTTONDOWN => {
                    if (wparam >> 16) & 0xffff == 1 {
                        emit(json!({ "kind": "back" }));
                    }
                    1
                }
                WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
                    let delta = ((wparam >> 16) & 0xffff) as i16 as f64 / NOTCH;
                    let (sx, sy) = point(lparam);
                    let mut p = POINT { x: sx, y: sy };
                    ScreenToClient(hwnd, &mut p);
                    let (x, y) = at((p.x, p.y), picture_rect(hwnd));
                    // Windows' positive is away from you (up) and to the right; the phone's dy, as a
                    // wheel turned towards you.
                    let (dx, dy) = if msg == WM_MOUSEWHEEL { (0.0, -delta) } else { (delta, 0.0) };
                    emit(json!({ "kind": "scroll", "x": x, "y": y, "dx": dx, "dy": dy }));
                    0
                }
                WM_KEYDOWN | WM_SYSKEYDOWN => {
                    let (ctrl, shift, alt) = (down(VK_CONTROL as i32), down(VK_SHIFT as i32), down(VK_MENU as i32));
                    let vk = wparam as u16;
                    if ctrl && !alt && vk == b'W' as u16 {
                        PostMessageW(hwnd, WM_CLOSE, 0, 0);
                        return 0;
                    }
                    if let Some(v) = key_input(vk, ctrl, shift, alt) {
                        emit(v);
                        return 0;
                    }
                    // Alt+F4 and the like stay Windows'.
                    DefWindowProcW(hwnd, msg, wparam, lparam)
                }
                WM_CHAR => {
                    let unit = wparam as u16;
                    let mut high = HIGH.lock().unwrap();
                    let c = if (0xD800..0xDC00).contains(&unit) {
                        *high = Some(unit);
                        None
                    } else if let Some(h) = high.take() {
                        char::decode_utf16([h, unit]).next().and_then(|r| r.ok())
                    } else {
                        char::from_u32(unit as u32)
                    };
                    drop(high);
                    if let Some(v) = c.and_then(char_input) {
                        emit(v);
                    }
                    0
                }
                WM_GETMINMAXINFO => {
                    let info = &mut *(lparam as *mut MINMAXINFO);
                    info.ptMinTrackSize = POINT { x: 200, y: 200 };
                    0
                }
                WM_DPICHANGED => {
                    let r = &*(lparam as *const RECT);
                    SetWindowPos(hwnd, std::ptr::null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                    0
                }
                WM_CLOSE => {
                    DestroyWindow(hwnd);
                    0
                }
                WM_DESTROY => {
                    PostQuitMessage(0);
                    0
                }
                _ => DefWindowProcW(hwnd, msg, wparam, lparam),
            }
        }
    }

    fn post(msg: u32, wparam: WPARAM) {
        let hwnd = WINDOW.load(Ordering::Acquire);
        if !hwnd.is_null() {
            // SAFETY: posting to our own window, from the decoding thread.
            unsafe { PostMessageW(hwnd, msg, wparam, 0) };
        }
    }

    /// Reads the stream, decodes it into [FRAME] and tells the window; closes it at the end.
    fn decode(mut input: impl Read) {
        // SAFETY: this thread joins the multithreaded apartment for the decoder.
        let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED) };
        let mut decoder = match Decoder::new() {
            Ok(d) => Some(d),
            Err(e) => {
                tracing::warn!("no decoder: {e:#}");
                None
            }
        };
        let mut buf: Vec<u8> = Vec::with_capacity(512 * 1024);
        let mut chunk = vec![0u8; 64 * 1024];
        let mut units = Units::default();
        loop {
            let n = match input.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            buf.extend_from_slice(&chunk[..n]);
            let (nals, used) = vyoma_core::mirror::split_nals(&buf);
            let whole: Vec<Vec<u8>> = nals.into_iter().filter_map(|nal| units.push(nal)).collect();
            buf.drain(..used);
            let Some(d) = decoder.as_mut() else { continue };
            for unit in whole {
                let shown = |w: usize, h: usize, bgra: &[u8]| {
                    let mut f = FRAME.lock().unwrap();
                    let resized = (f.w, f.h) != (w, h);
                    f.bgra.clear();
                    f.bgra.extend_from_slice(bgra);
                    (f.w, f.h) = (w, h);
                    drop(f);
                    SHOWN.fetch_add(1, Ordering::Relaxed);
                    post(WM_FRAME, resized as WPARAM);
                };
                if let Err(e) = d.decode(&unit, shown) {
                    tracing::debug!("a picture didn't decode: {e:#}");
                }
            }
        }
        // The phone stopped (or the agent went): the window closes.
        post(WM_CLOSE, 0);
    }

    /// Runs the window until it's closed or [input] ends.
    pub fn run(title: &str, input: impl Read + Send + 'static) -> Result<()> {
        let class = wide(CLASS);
        let title = wide(title);
        // SAFETY: a window class with our procedure (registering it again is harmless), one
        // window of it, and the standard message loop on this thread.
        unsafe {
            SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let instance = GetModuleHandleW(std::ptr::null());
            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                hIcon: LoadIconW(instance, 1 as _),
                hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
                lpszClassName: class.as_ptr(),
                ..std::mem::zeroed()
            };
            RegisterClassExW(&wc);
            let hwnd = CreateWindowExW(0, class.as_ptr(), title.as_ptr(), WS_OVERLAPPEDWINDOW, CW_USEDEFAULT, CW_USEDEFAULT, 420, 860, std::ptr::null_mut(), std::ptr::null_mut(), instance, std::ptr::null());
            if hwnd.is_null() {
                anyhow::bail!("couldn't make the window");
            }
            WINDOW.store(hwnd, Ordering::Release);
            ShowWindow(hwnd, SW_SHOW);
            SetForegroundWindow(hwnd);
            let reader = std::thread::Builder::new().name("vyoma-mirror-decode".into()).spawn(move || decode(input))?;
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            WINDOW.store(std::ptr::null_mut(), Ordering::Release);
            // The reader ends with its input; a closed window leaves it to the process's exit.
            if reader.is_finished() {
                let _ = reader.join();
            }
        }
        Ok(())
    }
}

/// The phone's sound: s16le PCM through core's jitter buffer into WASAPI's shared output.
pub mod sound {
    use std::net::IpAddr;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use anyhow::{Context, Result};
    use serde_json::Value;
    use vyoma_core::identity::Identity;
    use vyoma_core::mirror::{self, Queue, Shared, Sound};
    use windows::Win32::Media::Audio::{
        AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, IAudioClient, IAudioRenderClient, IMMDeviceEnumerator,
        MMDeviceEnumerator, WAVE_FORMAT_PCM, WAVEFORMATEX, eConsole, eRender,
    };
    use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};

    /// About this much sound waits in Windows' buffer: enough not to break up, little enough
    /// that it keeps up with the picture.
    const AHEAD_MS: u32 = 40;
    const TICK: Duration = Duration::from_millis(5);

    /// Plays the phone's sound until it stops.
    pub async fn play(me: &Identity, pinned: &[u8], phone: IpAddr, p: &Value, name: &str) -> Result<()> {
        let sound = Sound::of(&p["body"])?;
        let queue = Queue::shared();
        let _output = Output::start(sound, queue.clone())?;
        tracing::info!("{name}: playing the phone's sound");
        mirror::receive(me, pinned, phone, p, sound, &queue).await?;
        tracing::info!("{name}: phone's sound stopped");
        Ok(())
    }

    /// The output thread; it stops when this is dropped.
    struct Output {
        stop: Arc<AtomicBool>,
    }

    impl Drop for Output {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
        }
    }

    impl Output {
        fn start(sound: Sound, queue: Shared) -> Result<Output> {
            let stop = Arc::new(AtomicBool::new(false));
            let (ready_tx, ready) = std::sync::mpsc::channel::<Result<()>>();
            let s = stop.clone();
            std::thread::Builder::new().name("vyoma-sound".into()).spawn(move || {
                if let Err(e) = run(sound, &queue, &s, &ready_tx) {
                    let _ = ready_tx.send(Err(e));
                }
            })?;
            ready.recv_timeout(Duration::from_secs(5)).context("the sound didn't start")??;
            Ok(Output { stop })
        }
    }

    fn run(sound: Sound, queue: &Shared, stop: &AtomicBool, ready: &std::sync::mpsc::Sender<Result<()>>) -> Result<()> {
        let block = 2 * sound.channels;
        let format = WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_PCM as u16,
            nChannels: sound.channels as u16,
            nSamplesPerSec: sound.rate,
            nAvgBytesPerSec: sound.rate * block,
            nBlockAlign: block as u16,
            wBitsPerSample: 16,
            cbSize: 0,
        };
        // SAFETY: COM on this thread; the default speaker's shared stream in our format (Windows
        // converts it to the speaker's own); the buffer it lends is written within the frames
        // asked for and handed back.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let devices: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let device = devices.GetDefaultAudioEndpoint(eRender, eConsole).context("no speaker")?;
            let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
            client
                .Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, 1_000_000, 0, &format, None)
                .context("this PC couldn't play the phone's sound")?;
            let render: IAudioRenderClient = client.GetService()?;
            let capacity = client.GetBufferSize()?;
            let ahead = (sound.rate * AHEAD_MS / 1000).min(capacity);
            client.Start()?;
            let _ = ready.send(Ok(()));
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(TICK);
                let Ok(padding) = client.GetCurrentPadding() else { break };
                let want = ahead.saturating_sub(padding);
                if want == 0 {
                    continue;
                }
                let Ok(data) = render.GetBuffer(want) else { continue };
                let out = std::slice::from_raw_parts_mut(data, (want * block) as usize);
                match queue.lock() {
                    Ok(mut q) => q.fill(out),
                    Err(_) => out.fill(0),
                }
                let _ = render.ReleaseBuffer(want, 0);
            }
            let _ = client.Stop();
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn keys_as_a_windows_user_expects() {
        assert_eq!(key_input(VK_ESCAPE, false, false, false).unwrap()["kind"], "back");
        assert_eq!(key_input(0x56, true, false, false).unwrap()["kind"], "paste");
        assert_eq!(key_input(0x43, true, false, false).unwrap()["key"], "copy");
        assert_eq!(key_input(0x41, true, false, false).unwrap()["key"], "selectall");
        assert!(key_input(0x43, true, false, true).is_none(), "AltGr+C types, it isn't Copy");
        assert!(key_input(0x51, true, false, false).is_none(), "other shortcuts aren't typing");
        let word = key_input(VK_BACK, true, false, false).unwrap();
        assert_eq!((word["key"].as_str(), word["ctrl"].as_bool()), (Some("backspace"), Some(true)));
        assert_eq!(key_input(VK_LEFT, false, true, false).unwrap()["shift"], true);
        assert_eq!(key_input(VK_TAB, false, false, false).unwrap()["text"], "\t");
        assert!(key_input(0x41, false, false, false).is_none(), "letters come as characters");
        assert_eq!(char_input('é').unwrap()["text"], "é");
        assert!(char_input('\r').is_none() && char_input('\u{1}').is_none());
    }

    #[test]
    fn the_picture_keeps_its_shape_in_the_window() {
        // A tall phone in a wide window: bars at the sides.
        assert_eq!(letterbox((1000, 500), (1080, 2400)), (387, 0, 225, 500));
        assert_eq!(letterbox((300, 1000), (1080, 2400)), (0, 167, 300, 666));
        assert_eq!(at((387, 0), (387, 0, 225, 500)), (0.0, 0.0));
        assert_eq!(at((612, 500), (387, 0, 225, 500)), (1.0, 1.0));
        assert_eq!(at((10, 250), (387, 0, 225, 500)), (0.0, 0.5), "outside: the nearest edge");
        assert_eq!(fit_picture((1080, 2400), (1920, 1040)), (398, 884));
        assert_eq!(fit_picture((400, 300), (1920, 1040)), (400, 300), "never larger than its pixels");
    }

    #[test]
    fn nv12_turns_back_into_its_colours() {
        let (w, h) = (4usize, 2usize);
        let mut bgra = Vec::new();
        // Red left half, blue right half, through the encoder side's conversion and back.
        let mut src = Vec::new();
        for _ in 0..h {
            for x in 0..w {
                src.extend_from_slice(if x < 2 { &[0, 0, 255, 255] } else { &[255, 0, 0, 255] });
            }
        }
        let mut nv12 = Vec::new();
        crate::cast::to_nv12(&src, w * 4, w, h, w, h, &mut nv12);
        let (y, uv) = nv12.split_at(w * h);
        nv12_to_bgra(y, uv, w, (0, 0), w, h, &mut bgra);
        let near = |a: &[u8], b: [u8; 3]| a.iter().zip(b).all(|(x, y)| (*x as i32 - y as i32).abs() <= 12);
        assert!(near(&bgra[0..3], [0, 0, 255]), "{:?}", &bgra[0..4]);
        assert!(near(&bgra[12..15], [255, 0, 0]), "{:?}", &bgra[12..16]);
    }

    #[test]
    fn parameter_sets_go_in_with_their_picture() {
        let mut u = Units::default();
        assert!(u.push(&[0x67, 1]).is_none());
        assert!(u.push(&[0x68, 2]).is_none());
        assert_eq!(u.push(&[0x65, 3]).unwrap(), [0, 0, 0, 1, 0x67, 1, 0, 0, 0, 1, 0x68, 2, 0, 0, 0, 1, 0x65, 3]);
        assert_eq!(u.push(&[0x41, 4]).unwrap(), [0, 0, 0, 1, 0x41, 4]);
    }

    /// Pictures from this PC's own encoder (the live screen's), as the phone's would come.
    pub(crate) fn some_video(frames: usize) -> Option<Vec<u8>> {
        // SAFETY: COM and Media Foundation for this thread (both count their starts).
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_LITE).ok()?;
        }
        let mut encoder = crate::cast::Encoder::software(320, 240, 30).ok()?;
        let mut out = Vec::new();
        let mut nv12 = Vec::new();
        for n in 0..frames {
            let shade = (n * 8) as u8;
            let bgra: Vec<u8> = (0..320 * 240).flat_map(|i| [shade, (i % 256) as u8, 255 - shade, 255]).collect();
            crate::cast::to_nv12(&bgra, 320 * 4, 320, 240, 320, 240, &mut nv12);
            out.extend(encoder.encode(&nv12).ok()?);
        }
        println!("{} bytes of video", out.len());
        Some(out)
    }

    #[test]
    fn decodes_what_an_encoder_made() {
        let Some(video) = some_video(120) else {
            println!("no encoder here: skipped");
            return;
        };
        let mut d = match Decoder::new() {
            Ok(d) => d,
            Err(e) => {
                println!("no decoder here ({e:#}): skipped");
                return;
            }
        };
        let (nals, used) = vyoma_core::mirror::split_nals(&video);
        let mut units = Units::default();
        let mut sizes = Vec::new();
        let mut aus: Vec<Vec<u8>> = nals.into_iter().filter_map(|n| units.push(n)).collect();
        // The last unit has no start code after it.
        let rest = [&video[used..], &[0, 0, 0, 1]].concat();
        let (tail, _) = vyoma_core::mirror::split_nals(&rest);
        aus.extend(tail.into_iter().filter_map(|n| units.push(n)));
        for au in aus {
            d.decode(&au, |w, h, bgra| {
                assert_eq!(bgra.len(), w * h * 4);
                sizes.push((w, h));
            })
            .unwrap();
        }
        println!("decoded {} pictures", sizes.len());
        assert!(!sizes.is_empty(), "nothing came out");
        assert!(sizes.iter().all(|&s| s == (320, 240)), "{sizes:?}");
    }

    #[test]
    fn the_window_shows_the_stream_and_closes_at_its_end() {
        let Some(video) = some_video(120) else {
            println!("no encoder here: skipped");
            return;
        };
        let done = std::thread::spawn(move || view::run("Test phone", std::io::Cursor::new(video)));
        let started = std::time::Instant::now();
        while !done.is_finished() {
            assert!(started.elapsed() < Duration::from_secs(20), "the window never closed");
            std::thread::sleep(Duration::from_millis(50));
        }
        done.join().unwrap().unwrap();
        println!("the window showed {} pictures", view::SHOWN.load(std::sync::atomic::Ordering::Relaxed));
        assert!(view::SHOWN.load(std::sync::atomic::Ordering::Relaxed) > 0);
    }
}
