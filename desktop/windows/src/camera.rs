//! A photo with this PC's camera, from the phone (`vyoma.camera.request` → `vyoma.camera`, a JPEG,
//! as from the Linux and Mac sides). Media Foundation's source reader runs the first camera at its
//! largest picture up to 1080p, converted to RGB; the pictures from the first moment are dropped
//! while its exposure settles, the next is kept and Windows' imaging component (WIC) makes it a JPEG.
//! Every photo also shows a notification here: someone at the PC should always know its camera was
//! used from elsewhere (the camera's light comes on as well).
//!
//! Windows lets desktop apps use the camera unless Settings > Privacy & security > Camera says
//! otherwise; then the phone is told where to turn it on. A camera another app holds can't be read.
//! Windows doesn't tell desktop apps that a laptop's lid is closed, so `cameraLid` stays unsaid.
//! Built and tested on CI and under Wine only; not yet run on a real Windows PC.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde_json::json;
use vyoma_core::identity::Identity;
use vyoma_core::link::{Sender, packet};
use vyoma_core::payload;
use windows::Win32::Foundation::E_ACCESSDENIED;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatJpeg, GUID_WICPixelFormat24bppBGR, IWICBitmapFrameEncode, IWICImagingFactory, WICBitmapEncoderNoCache,
};
use windows::Win32::Media::MediaFoundation::{
    IMF2DBuffer, IMFActivate, IMFAttributes, IMFMediaSource, IMFMediaType, IMFSample, IMFSourceReader, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID, MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING,
    MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_SOURCE_READERF_ENDOFSTREAM, MF_SOURCE_READERF_ERROR, MF_VERSION, MFCreateAttributes, MFCreateMediaType,
    MFCreateSourceReaderFromMediaSource, MFEnumDeviceSources, MFMediaType_Video, MFSTARTUP_LITE, MFStartup, MFVideoFormat_RGB32,
};
use windows::Win32::System::Com::StructuredStorage::{CreateStreamOnHGlobal, IPropertyBag2};
use windows::core::Interface;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, STATFLAG_NONAME, STREAM_SEEK_SET};

pub const CAMERA: &str = "vyoma.camera";
/// This PC's check: the window's This PC page, and its fix (FixCheck).
pub const CHECK: &str = "Camera";
/// Settings > Privacy & security > Camera.
pub const SETTINGS: &str = "ms-settings:privacy-webcam";
/// Where Windows keeps this person's camera switches: camera access, and desktop apps' under it.
const CONSENT: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\webcam";

const NO_CAMERA: &str = "this PC has no camera";
const PRIVACY: &str = "Windows doesn't let apps on this PC use the camera: on the PC, Settings > Privacy & security > Camera, turn on Let desktop apps access your camera";
const BUSY: &str = "this PC's camera couldn't start: another app may be using it";

/// How long the camera runs before the photo: it starts dark and needs a moment to expose.
const WARMUP: Duration = Duration::from_millis(900);
/// Longest the whole photo may take.
const TAKE_WAIT: Duration = Duration::from_secs(15);
/// How long a look at the cameras holds (the stats ask every 2 s).
const STATE_HOLDS: Duration = Duration::from_secs(10);
/// The largest picture asked of the camera (a photo for a phone's screen; 4K webcams are slow).
const MAX_PIXELS: u64 = 1920 * 1080;

/// Whether a camera can take a photo now, and (never on Windows) whether a closed lid is why not
/// (the phone's stats `camera` and `cameraLid`). Looked at once in [STATE_HOLDS].
pub async fn state() -> (bool, bool) {
    static LAST: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
    if let Some((at, ok)) = *LAST.lock().unwrap()
        && at.elapsed() < STATE_HOLDS
    {
        return (ok, false);
    }
    let ok = tokio::task::spawn_blocking(|| cameras().is_ok_and(|c| !c.is_empty())).await.unwrap_or(false);
    *LAST.lock().unwrap() = Some((Instant::now(), ok));
    (ok, false)
}

/// Whether Windows' switches let desktop apps use the camera: "Deny" in either refuses (an
/// unset one is allowed; a company's policy can still refuse, which [photo] says when it happens).
pub fn allowed(camera: Option<&str>, desktop_apps: Option<&str>) -> bool {
    ![camera, desktop_apps].iter().any(|v| v.is_some_and(|v| v.eq_ignore_ascii_case("deny")))
}

/// This PC's Camera check, where there's a camera (blocking).
pub fn check() -> Option<serde_json::Value> {
    if !cameras().is_ok_and(|c| !c.is_empty()) {
        return None;
    }
    let ok = allowed(crate::win::user_value(CONSENT, Some("Value")).as_deref(), crate::win::user_value(&format!(r"{CONSENT}\NonPackaged"), Some("Value")).as_deref());
    let detail = if ok { "Phones you allow can take a photo with it; you're told each time" } else { "Windows keeps apps from the camera, so phones can't take a photo. Turn on camera access and Let desktop apps access your camera" };
    Some(json!({ "name": CHECK, "state": if ok { "ok" } else { "warn" }, "detail": detail, "action": if ok { "" } else { "Open settings" } }))
}

/// Takes a photo and offers it to the phone; an error goes to the phone in words.
pub async fn take(sender: &Sender, me: &Identity, pinned: &[u8], phone: &str) -> Result<()> {
    let photo = match tokio::time::timeout(TAKE_WAIT, tokio::task::spawn_blocking(photo)).await {
        Ok(Ok(Ok(jpeg))) => Ok(jpeg),
        Ok(Ok(Err(e))) => Err(e),
        Ok(Err(e)) => Err(e.into()),
        Err(_) => Err(anyhow::anyhow!("this PC's camera took too long")),
    };
    let photo = match photo {
        Ok(b) => b,
        Err(e) => {
            sender.send(&packet(CAMERA, json!({ "error": format!("{e:#}") }))).await?;
            return Err(e);
        }
    };
    let text = format!("{phone} took a photo with this PC's camera.");
    tokio::task::spawn_blocking(move || crate::tray::notify("Camera used from your phone", &text));
    payload::offer(sender, me, pinned, packet(CAMERA, json!({ "format": "jpg" })), photo.len() as u64, photo.as_slice()).await
}

/// The video capture devices Windows has, in its order.
fn cameras() -> Result<Vec<IMFActivate>> {
    // SAFETY: COM and Media Foundation on this thread; the list Windows allocates is read once per
    // entry and freed.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        MFStartup(MF_VERSION, MFSTARTUP_LITE).context("no Media Foundation (a Windows N edition without the Media Feature Pack?)")?;
        let mut attrs: Option<IMFAttributes> = None;
        MFCreateAttributes(&mut attrs, 1)?;
        let attrs = attrs.context("no attributes")?;
        attrs.SetGUID(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID)?;
        let mut list: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut count = 0u32;
        MFEnumDeviceSources(&attrs, &mut list, &mut count)?;
        if list.is_null() {
            return Ok(Vec::new());
        }
        let found: Vec<Option<IMFActivate>> = (0..count as usize).map(|i| std::ptr::read(list.add(i))).collect();
        CoTaskMemFree(Some(list.cast()));
        Ok(found.into_iter().flatten().collect())
    }
}

/// Why the camera couldn't be used, in words for the phone.
fn explain(e: windows::core::Error) -> anyhow::Error {
    if e.code() == E_ACCESSDENIED { anyhow::anyhow!(PRIVACY) } else { anyhow::anyhow!("{BUSY} ({e})") }
}

/// One JPEG from the first camera (blocking).
fn photo() -> Result<Vec<u8>> {
    let camera = cameras()?.into_iter().next().context(NO_CAMERA)?;
    // SAFETY: the camera's source, read through a source reader on this thread; it's shut down
    // once the picture is in hand.
    let picture = unsafe {
        let source: IMFMediaSource = camera.ActivateObject().map_err(explain)?;
        let result = read_picture(&source);
        let _ = source.Shutdown();
        let _ = camera.ShutdownObject();
        result?
    };
    jpeg(&picture)
}

/// A picture as top-down BGR, three bytes a pixel.
struct Picture {
    w: usize,
    h: usize,
    bgr: Vec<u8>,
}

/// The largest frame size the camera offers up to [MAX_PIXELS] (index of the native type).
fn best_type(sizes: &[(u32, u32)]) -> Option<usize> {
    let fits = |&(_, (w, h)): &(usize, &(u32, u32))| (*w as u64) * (*h as u64) <= MAX_PIXELS;
    let area = |(_, (w, h)): &(usize, &(u32, u32))| (*w as u64) * (*h as u64);
    let indexed = || sizes.iter().enumerate();
    indexed().filter(fits).max_by_key(area).or_else(|| indexed().min_by_key(area)).map(|(i, _)| i)
}

fn frame_size(t: &IMFMediaType) -> Option<(u32, u32)> {
    // SAFETY: a plain read of the type's attribute.
    let size = unsafe { t.GetUINT64(&MF_MT_FRAME_SIZE).ok()? };
    Some(((size >> 32) as u32, size as u32))
}

fn read_picture(source: &IMFMediaSource) -> Result<Picture> {
    let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
    // SAFETY: a source reader over the camera, its types chosen from what it offers; samples read
    // one at a time on this thread.
    unsafe {
        let mut attrs: Option<IMFAttributes> = None;
        MFCreateAttributes(&mut attrs, 1)?;
        let attrs = attrs.context("no attributes")?;
        // The reader converts the camera's own format (YUY2, NV12, MJPEG) to RGB.
        attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)?;
        let reader: IMFSourceReader = MFCreateSourceReaderFromMediaSource(source, &attrs).map_err(explain)?;
        let mut native = Vec::new();
        while let Ok(t) = reader.GetNativeMediaType(stream, native.len() as u32) {
            native.push(t);
        }
        let sizes: Vec<(u32, u32)> = native.iter().map(|t| frame_size(t).unwrap_or((0, 0))).collect();
        if let Some(i) = best_type(&sizes) {
            let _ = reader.SetCurrentMediaType(stream, None, &native[i]);
        }
        let rgb = MFCreateMediaType()?;
        rgb.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        rgb.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
        reader.SetCurrentMediaType(stream, None, &rgb).context("the camera's pictures can't be turned into RGB")?;
        let current = reader.GetCurrentMediaType(stream)?;
        let (w, h) = frame_size(&current).context("the camera says no size")?;
        let stride = current.GetUINT32(&MF_MT_DEFAULT_STRIDE).ok().map(|s| s as i32);
        let started = Instant::now();
        loop {
            let (mut flags, mut sample) = (0u32, None::<IMFSample>);
            reader.ReadSample(stream, 0, None, Some(&mut flags), None, Some(&mut sample)).map_err(explain)?;
            if flags & (MF_SOURCE_READERF_ERROR.0 as u32 | MF_SOURCE_READERF_ENDOFSTREAM.0 as u32) != 0 {
                bail!(BUSY);
            }
            if started.elapsed() < WARMUP {
                continue;
            }
            let Some(sample) = sample else { continue };
            return to_bgr(&sample, w as usize, h as usize, stride);
        }
    }
}

/// An RGB32 sample as top-down BGR. Rows may run bottom-up (a negative stride).
fn to_bgr(sample: &IMFSample, w: usize, h: usize, stride: Option<i32>) -> Result<Picture> {
    let mut bgr = Vec::with_capacity(w * h * 3);
    // SAFETY: the sample's buffer stays locked while its rows are read, each within its pitch.
    unsafe {
        let buffer = sample.ConvertToContiguousBuffer()?;
        if let Ok(two_d) = buffer.cast::<IMF2DBuffer>() {
            let (mut scan0, mut pitch) = (std::ptr::null_mut(), 0i32);
            two_d.Lock2D(&mut scan0, &mut pitch)?;
            for y in 0..h {
                let row = std::slice::from_raw_parts(scan0.offset(y as isize * pitch as isize), w * 4);
                bgrx_row(row, &mut bgr);
            }
            two_d.Unlock2D()?;
        } else {
            let (mut data, mut len) = (std::ptr::null_mut(), 0u32);
            buffer.Lock(&mut data, None, Some(&mut len))?;
            let all = std::slice::from_raw_parts(data, len as usize);
            let pitch = stride.unwrap_or((w * 4) as i32);
            let step = pitch.unsigned_abs() as usize;
            if step >= w * 4 && all.len() >= step * h {
                for y in 0..h {
                    // A negative stride: the first row in memory is the bottom one.
                    let at = if pitch < 0 { (h - 1 - y) * step } else { y * step };
                    bgrx_row(&all[at..at + w * 4], &mut bgr);
                }
            }
            buffer.Unlock()?;
        }
    }
    if bgr.len() != w * h * 3 {
        bail!("the camera's picture was cut short");
    }
    Ok(Picture { w, h, bgr })
}

fn bgrx_row(row: &[u8], out: &mut Vec<u8>) {
    for px in row.as_chunks::<4>().0 {
        out.extend_from_slice(&px[..3]);
    }
}

/// The picture as a JPEG (WIC's own quality, 0.9).
fn jpeg(p: &Picture) -> Result<Vec<u8>> {
    // SAFETY: WIC's encoder writes into a stream in memory; the frame's pixels are ours, of exactly
    // stride × height bytes; the stream is read back from its start.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let stream = CreateStreamOnHGlobal(Default::default(), true)?;
        let encoder = wic.CreateEncoder(&GUID_ContainerFormatJpeg, std::ptr::null())?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let (mut frame, mut options): (Option<IWICBitmapFrameEncode>, Option<IPropertyBag2>) = (None, None);
        encoder.CreateNewFrame(&mut frame, &mut options)?;
        let frame = frame.context("no JPEG frame")?;
        frame.Initialize(options.as_ref())?;
        frame.SetSize(p.w as u32, p.h as u32)?;
        let mut format = GUID_WICPixelFormat24bppBGR;
        frame.SetPixelFormat(&mut format)?;
        if format != GUID_WICPixelFormat24bppBGR {
            bail!("Windows' JPEG encoder wants another pixel format");
        }
        frame.WritePixels(p.h as u32, (p.w * 3) as u32, &p.bgr)?;
        frame.Commit()?;
        encoder.Commit()?;
        let mut stat = Default::default();
        stream.Stat(&mut stat, STATFLAG_NONAME)?;
        let size = stat.cbSize as usize;
        stream.Seek(0, STREAM_SEEK_SET, None)?;
        let mut out = vec![0u8; size];
        let mut read = 0u32;
        stream.Read(out.as_mut_ptr().cast(), size as u32, Some(&mut read)).ok()?;
        out.truncate(read as usize);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asks_the_camera_for_a_phone_sized_picture() {
        assert_eq!(best_type(&[(640, 480), (1920, 1080), (1280, 720), (3840, 2160)]), Some(1));
        assert_eq!(best_type(&[(3840, 2160), (4096, 2160)]), Some(0), "only large ones: the smallest");
        assert_eq!(best_type(&[]), None);
    }

    #[test]
    fn a_picture_becomes_a_jpeg() {
        let (w, h) = (64, 48);
        let bgr: Vec<u8> = (0..w * h).flat_map(|i| [(i % 256) as u8, 128, 255 - (i % 256) as u8]).collect();
        let jpeg = jpeg(&Picture { w, h, bgr }).unwrap();
        assert_eq!(&jpeg[..3], &[0xFF, 0xD8, 0xFF], "a JPEG's start");
        assert_eq!(&jpeg[jpeg.len() - 2..], &[0xFF, 0xD9], "and its end");
    }

    #[test]
    fn windows_switches_are_read_as_windows_means_them() {
        assert!(allowed(None, None), "never touched: allowed");
        assert!(allowed(Some("Allow"), Some("Allow")));
        assert!(!allowed(Some("Deny"), Some("Allow")), "camera access off");
        assert!(!allowed(Some("Allow"), Some("Deny")), "desktop apps off");
        assert!(check().is_none() || cameras().is_ok_and(|c| !c.is_empty()), "no camera, no check");
    }

    #[test]
    fn rows_read_either_way_up() {
        let mut out = Vec::new();
        bgrx_row(&[1, 2, 3, 0, 4, 5, 6, 0], &mut out);
        assert_eq!(out, [1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn looks_for_cameras() {
        // Only counted: a photo turns the camera on, so that needs asking for (VYOMA_CAMERA_TEST=1).
        let found = cameras().map(|c| c.len());
        println!("cameras: {found:?}");
        if std::env::var_os("VYOMA_CAMERA_TEST").is_some() {
            let jpeg = photo().unwrap();
            assert_eq!(&jpeg[..2], &[0xFF, 0xD8]);
        } else if found.is_ok_and(|n| n == 0) {
            assert_eq!(format!("{:#}", photo().unwrap_err()), NO_CAMERA);
        }
    }
}
