//! This Mac's screen on the phone: screenshots (`vyoma.screenshot.request`, macOS's own
//! `screencapture`) and the live screen (`vyoma.screen.request`), which the phone can also work by
//! touch (`vyoma.screen.input`, or a live stream of them, `vyoma.screen.touch`).
//!
//! The live screen is ScreenCaptureKit's picture of the main display, scaled to at most 1920 wide,
//! encoded to H.264 in hardware by VideoToolbox (real time, no B-frames, keyframes only to start), and
//! sent as the Linux app sends it: one open-ended payload of NAL units with 4-byte big-endian
//! lengths, the parameter sets before each keyframe, and a timestamp unit before each picture for
//! the phone's latency figures. ScreenCaptureKit only sends a frame when something changed, so a
//! still screen costs nothing. If the phone falls behind, new frames are skipped before encoding
//! instead of piling up.
//!
//! macOS asks once for Screen Recording (System Settings > Privacy & Security), and shows its own
//! recording indicator while the phone watches. ScreenCaptureKit is loaded only when the screen is
//! first watched (it came with macOS 12.3), so older Macs still run everything else.
//! Built and tested on CI only; not yet run on a real Mac.

// The capture itself is macOS's; the framing and its tests run everywhere.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use vyoma_core::identity::Identity;
use vyoma_core::link::{Sender, packet};
use vyoma_core::payload;


pub const SCREENSHOT: &str = "vyoma.screenshot";
pub const SCREEN: &str = "vyoma.screen";


/// Where macOS lets apps see the screen.
pub const SCREEN_RECORDING: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";
/// The phone's word when macOS hasn't allowed it, short enough for its status bubble.
pub const ALLOW_SHORT: &str = "The Mac hasn't allowed Vyoma to see its screen: on the Mac, System Settings > Privacy & Security > Screen Recording, turn on Vyoma";

/// The phone's screen is about 1080 px tall in landscape: more than 1920 wide is wasted bits.
const MAX_WIDTH: i32 = 1920;
const BITRATE: i32 = 8_000_000;
/// The most frames a second (the phone asks for what it shows, `maxFps`), the least, and for a
/// phone that doesn't say.
const MAX_FPS: u32 = 120;
const MIN_FPS: u32 = 30;
const DEFAULT_FPS: u32 = 60;
/// Encoded video waiting for the link beyond this, and new frames are skipped until it drains.
const MAX_PENDING: usize = 256 * 1024;
/// Small windows from the encoder to the socket, so a slow link holds frames back at the start.
const RELAY_BYTES: usize = 16 * 1024;
const STAMP_MAGIC: &[u8; 4] = b"VYTS";

/// The rate a phone asked for, within what the cast allows.
pub fn fps_for(asked: Option<u64>) -> u32 {
    asked.map_or(DEFAULT_FPS, |f| (f.min(MAX_FPS as u64) as u32).max(MIN_FPS))
}

/// Scales to fit [MAX_WIDTH] keeping the shape; even sizes, as H.264 wants.
pub fn fit((w, h): (i32, i32)) -> (i32, i32) {
    let (w, h) = if w > MAX_WIDTH { (MAX_WIDTH, (h as i64 * MAX_WIDTH as i64 / w as i64) as i32) } else { (w, h) };
    (w & !1, h & !1)
}

/// One encoded picture as the phone reads it: on a keyframe its parameter sets first, then the
/// picture's NAL units with a timestamp unit (an SEI-typed NAL: our marker and the send time in µs
/// since the epoch, which the phone takes out before decoding) before its first slice. [avcc] is
/// VideoToolbox's output, already 4-byte length prefixed.
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

fn now_us() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_micros() as u64).unwrap_or(0)
}

/// A whole-screen PNG of the main display, through `screencapture` (no sound, no flash).
async fn capture() -> Result<Vec<u8>> {
    if !allowed(true) {
        bail!("{ALLOW_SHORT}");
    }
    // The Mac's TMPDIR is this user's own; deleted as soon as it's read.
    let path = std::env::temp_dir().join(format!("vyoma-shot-{}.png", std::process::id()));
    let status = tokio::time::timeout(Duration::from_secs(15), tokio::process::Command::new("screencapture").args(["-x", "-t", "png"]).arg(&path).kill_on_drop(true).status())
        .await
        .context("the screenshot took too long")?
        .context("no screencapture")?;
    let bytes = tokio::fs::read(&path).await;
    let _ = tokio::fs::remove_file(&path).await;
    match bytes {
        Ok(b) if status.success() && b.starts_with(b"\x89PNG") => Ok(b),
        _ => bail!("{ALLOW_SHORT}"),
    }
}

pub async fn screenshot(sender: &Sender, me: &Identity, pinned: &[u8], phone: &str) -> Result<()> {
    let image = match capture().await {
        Ok(b) => b,
        Err(e) => {
            sender.send(&packet(SCREENSHOT, json!({ "error": format!("{e:#}") }))).await?;
            return Err(e);
        }
    };
    // Someone at the Mac should always know its screen was taken from elsewhere.
    crate::mac::notify("Screenshot taken from your phone", &format!("{phone} took a screenshot of this Mac.")).await;
    payload::offer(sender, me, pinned, packet(SCREENSHOT, json!({ "format": "png" })), image.len() as u64, image.as_slice()).await
}

/// Whether macOS lets Vyoma see the screen; with [ask], has macOS show its question if not.
pub fn allowed(ask: bool) -> bool {
    #[cfg(target_os = "macos")]
    {
        cast::allowed(ask)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = ask;
        true
    }
}

/// Casts until the phone hangs up or the stream fails. Failures before it starts reach the phone
/// as an error, so it never waits on a spinner.
pub async fn stream(sender: &Sender, me: &Identity, pinned: &[u8], name: &str, fps: u32, inputs: mpsc::UnboundedReceiver<Value>) -> Result<()> {
    #[cfg(target_os = "macos")]
    let started = tokio::task::spawn_blocking(move || cast::Cast::start(fps)).await.context("the cast didn't start")?;
    #[cfg(not(target_os = "macos"))]
    let started: Result<(cast::Cast, mpsc::UnboundedReceiver<Vec<u8>>)> = {
        let _ = fps;
        Err(anyhow::anyhow!("Only on a Mac"))
    };
    let (cast, mut video) = match started {
        Ok(c) => c,
        Err(e) => {
            sender.send(&packet(SCREEN, json!({ "error": format!("{e:#}") }))).await?;
            return Err(e);
        }
    };
    let (w, h) = cast.size;
    let control = crate::input::trusted();
    tracing::info!("{name}: casting the screen at {w}x{h}{}", if control { "" } else { ", watch only (no Accessibility)" });
    let (relay, source) = tokio::io::duplex(RELAY_BYTES);
    let pending = cast.pending();
    let write = async move {
        use tokio::io::AsyncWriteExt;
        let mut relay = relay;
        while let Some(piece) = video.recv().await {
            let n = piece.len();
            if relay.write_all(&piece).await.is_err() {
                break;
            }
            pending.fetch_sub(n, std::sync::atomic::Ordering::Relaxed);
        }
    };
    // Both end with this cast however it ends (the phone's stop aborts this task).
    let _writer = AbortOnDrop(tokio::spawn(write));
    // The phone's touches, as the pointer on the cast display, while the cast runs.
    let _touching = AbortOnDrop(tokio::spawn(follow(cast.display, inputs)));
    let announce = packet(SCREEN, json!({ "codec": "h264", "framing": "avc4", "width": w, "height": h, "control": control }));
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
async fn follow(display: u32, mut inputs: mpsc::UnboundedReceiver<Value>) {
    let mut pointer = crate::input::Pointer::default();
    pointer.cast_on(display);
    let mut held = false;
    while let Some(mut i) = inputs.recv().await {
        // Behind? Only the newest of a run of moves matters.
        while i["kind"] == "move" {
            match inputs.try_recv() {
                Ok(next) if next["kind"] == "move" => i = next,
                Ok(next) => {
                    let _ = pointer.handle(&crate::input::touch_acts(&i));
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
        let _ = pointer.handle(&crate::input::touch_acts(&i));
    }
    if held {
        let _ = pointer.handle(&[crate::input::Act::Button(crate::input::Button::Left, false)]);
    }
}

#[cfg(not(target_os = "macos"))]
mod cast {
    pub struct Cast {
        pub size: (i32, i32),
        pub display: u32,
    }

    impl Cast {
        pub fn pending(&self) -> std::sync::Arc<std::sync::atomic::AtomicUsize> {
            Default::default()
        }
    }
}

#[cfg(target_os = "macos")]
mod cast {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use anyhow::{Context, Result, bail};
    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Bool};
    use objc2::{AllocAnyThread, DefinedClass, define_class, msg_send};
    use objc2_core_media::{CMTime, CMTimeFlags};
    use objc2_foundation::{NSObject, NSString};
    use tokio::sync::mpsc;

    type CFRef = *const c_void;

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGMainDisplayID() -> u32;
        fn CGDisplayCopyDisplayMode(display: u32) -> CFRef;
        fn CGDisplayModeGetPixelWidth(mode: CFRef) -> usize;
        fn CGDisplayModeGetPixelHeight(mode: CFRef) -> usize;
        fn CGDisplayModeRelease(mode: CFRef);
        fn CGPreflightScreenCaptureAccess() -> bool;
        fn CGRequestScreenCaptureAccess() -> bool;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(object: CFRef);
        fn CFNumberCreate(alloc: CFRef, kind: isize, value: *const c_void) -> CFRef;
        fn CFStringCreateWithCString(alloc: CFRef, text: *const std::ffi::c_char, encoding: u32) -> CFRef;
        fn CFDictionaryCreate(alloc: *mut c_void, keys: *const *const c_void, values: *const *const c_void, count: isize, key_callbacks: *const c_void, value_callbacks: *const c_void) -> *mut c_void;
        fn CFArrayGetCount(array: CFRef) -> isize;
        fn CFArrayGetValueAtIndex(array: CFRef, index: isize) -> CFRef;
        fn CFDictionaryGetValue(dict: CFRef, key: CFRef) -> CFRef;
        fn CFBooleanGetValue(boolean: CFRef) -> u8;
        static kCFBooleanTrue: CFRef;
        static kCFBooleanFalse: CFRef;
        static kCFTypeDictionaryKeyCallBacks: u8;
        static kCFTypeDictionaryValueCallBacks: u8;
    }

    #[link(name = "CoreMedia", kind = "framework")]
    unsafe extern "C" {
        fn CMSampleBufferGetImageBuffer(sample: CFRef) -> CFRef;
        fn CMSampleBufferGetPresentationTimeStamp(sample: CFRef) -> CMTime;
        fn CMSampleBufferDataIsReady(sample: CFRef) -> bool;
        fn CMSampleBufferGetSampleAttachmentsArray(sample: CFRef, create: bool) -> CFRef;
        fn CMSampleBufferGetFormatDescription(sample: CFRef) -> CFRef;
        fn CMSampleBufferGetDataBuffer(sample: CFRef) -> CFRef;
        fn CMBlockBufferGetDataLength(buffer: CFRef) -> usize;
        fn CMBlockBufferCopyDataBytes(buffer: CFRef, offset: usize, length: usize, destination: *mut c_void) -> i32;
        fn CMVideoFormatDescriptionGetH264ParameterSetAtIndex(format: CFRef, index: usize, set: *mut *const u8, size: *mut usize, count: *mut usize, header_length: *mut i32) -> i32;
        static kCMSampleAttachmentKey_NotSync: CFRef;
    }

    type Encoded = extern "C" fn(*mut c_void, *mut c_void, i32, u32, CFRef);

    #[link(name = "VideoToolbox", kind = "framework")]
    unsafe extern "C" {
        fn VTCompressionSessionCreate(alloc: CFRef, width: i32, height: i32, codec: u32, spec: CFRef, source: CFRef, data_alloc: CFRef, callback: Encoded, refcon: *mut c_void, out: *mut CFRef) -> i32;
        fn VTSessionSetProperty(session: CFRef, key: CFRef, value: CFRef) -> i32;
        fn VTCompressionSessionPrepareToEncodeFrames(session: CFRef) -> i32;
        fn VTCompressionSessionEncodeFrame(session: CFRef, image: CFRef, pts: CMTime, duration: CMTime, props: CFRef, refcon: *mut c_void, flags: *mut u32) -> i32;
        fn VTCompressionSessionInvalidate(session: CFRef);
        static kVTCompressionPropertyKey_RealTime: CFRef;
        static kVTCompressionPropertyKey_ProfileLevel: CFRef;
        static kVTCompressionPropertyKey_AllowFrameReordering: CFRef;
        static kVTCompressionPropertyKey_AverageBitRate: CFRef;
        static kVTCompressionPropertyKey_MaxKeyFrameInterval: CFRef;
        static kVTCompressionPropertyKey_ExpectedFrameRate: CFRef;
        static kVTProfileLevel_H264_Main_AutoLevel: CFRef;
    }

    const UTF8: u32 = 0x0800_0100;
    const SINT32: isize = 3;
    const AVC1: u32 = u32::from_be_bytes(*b"avc1");
    /// kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange: what the encoder takes without converting.
    const NV12: u32 = u32::from_be_bytes(*b"420v");
    /// SCStreamErrorUserDeclined: Screen Recording isn't allowed.
    const DECLINED: isize = -3801;
    const KEYFRAME_FRAMES: i32 = 1024;
    const SETUP_WAIT: Duration = Duration::from_secs(10);

    pub fn allowed(ask: bool) -> bool {
        // SAFETY: plain CoreGraphics calls; the request shows macOS's own question once.
        unsafe { CGPreflightScreenCaptureAccess() || (ask && CGRequestScreenCaptureAccess()) }
    }

    /// What the capture queue, the encoder's thread and the writer share.
    struct Shared {
        /// The compression session, null once the cast is over.
        session: Mutex<usize>,
        out: mpsc::UnboundedSender<Vec<u8>>,
        /// Encoded bytes not yet handed to the link.
        pending: Arc<AtomicUsize>,
    }

    impl Shared {
        /// A captured frame into the encoder, unless the link is behind or it's an idle frame.
        fn encode(&self, sample: CFRef) {
            if self.pending.load(Ordering::Relaxed) > super::MAX_PENDING {
                return;
            }
            // SAFETY: CoreMedia getters on the sample ScreenCaptureKit hands us for this call; the
            // session is used under its lock, so it can't be invalidated meanwhile.
            unsafe {
                let image = CMSampleBufferGetImageBuffer(sample);
                if image.is_null() {
                    return; // Nothing changed: ScreenCaptureKit's idle frames carry no picture.
                }
                let pts = CMSampleBufferGetPresentationTimeStamp(sample);
                let session = self.session.lock().unwrap();
                if *session == 0 {
                    return;
                }
                let invalid = CMTime { value: 0, timescale: 0, flags: CMTimeFlags(0), epoch: 0 };
                VTCompressionSessionEncodeFrame(*session as CFRef, image, pts, invalid, std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut());
            }
        }
    }

    /// The encoder's output: one picture, framed for the phone.
    extern "C" fn encoded(refcon: *mut c_void, _: *mut c_void, status: i32, _: u32, sample: CFRef) {
        if status != 0 || sample.is_null() {
            return;
        }
        // SAFETY: refcon is the Shared the session was made with, alive until after it's invalidated;
        // the rest are CoreMedia getters on the sample VideoToolbox hands us for this call.
        unsafe {
            let shared = &*(refcon as *const Shared);
            if !CMSampleBufferDataIsReady(sample) {
                return;
            }
            let attachments = CMSampleBufferGetSampleAttachmentsArray(sample, false);
            let key = attachments.is_null() || CFArrayGetCount(attachments) == 0 || {
                let not_sync = CFDictionaryGetValue(CFArrayGetValueAtIndex(attachments, 0), kCMSampleAttachmentKey_NotSync);
                not_sync.is_null() || CFBooleanGetValue(not_sync) == 0
            };
            let mut params: Vec<&[u8]> = Vec::new();
            if key {
                let format = CMSampleBufferGetFormatDescription(sample);
                let (mut count, mut header) = (0usize, 0i32);
                let (mut set, mut size) = (std::ptr::null(), 0usize);
                if CMVideoFormatDescriptionGetH264ParameterSetAtIndex(format, 0, &mut set, &mut size, &mut count, &mut header) == 0 {
                    for i in 0..count {
                        if CMVideoFormatDescriptionGetH264ParameterSetAtIndex(format, i, &mut set, &mut size, std::ptr::null_mut(), std::ptr::null_mut()) == 0 && !set.is_null() {
                            params.push(std::slice::from_raw_parts(set, size));
                        }
                    }
                }
            }
            let data = CMSampleBufferGetDataBuffer(sample);
            if data.is_null() {
                return;
            }
            let len = CMBlockBufferGetDataLength(data);
            let mut avcc = vec![0u8; len];
            if CMBlockBufferCopyDataBytes(data, 0, len, avcc.as_mut_ptr().cast()) != 0 {
                return;
            }
            let bytes = super::frame(&params, &avcc, super::now_us());
            shared.pending.fetch_add(bytes.len(), Ordering::Relaxed);
            let _ = shared.out.send(bytes);
        }
    }

    struct Ivars {
        shared: Arc<Shared>,
    }

    define_class!(
        // ScreenCaptureKit's SCStreamOutput: frames arrive on the capture queue.
        #[unsafe(super(NSObject))]
        #[name = "VyomaScreenOutput"]
        #[ivars = Ivars]
        struct Output;

        impl Output {
            #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
            fn did_output(&self, _stream: *mut AnyObject, sample: *mut c_void, kind: isize) {
                // 0: the screen (not sound).
                if kind == 0 {
                    self.ivars().shared.encode(sample);
                }
            }
        }
    );

    impl Output {
        fn new(shared: Arc<Shared>) -> Retained<Self> {
            let this = Self::alloc().set_ivars(Ivars { shared });
            // SAFETY: NSObject's init on a freshly allocated object.
            unsafe { msg_send![super(this), init] }
        }
    }

    /// A Retained that crosses threads: only handed over, never used from two at once.
    struct Handover<T>(T);
    // SAFETY: see above; ScreenCaptureKit's objects are thread-safe to hand to another thread.
    unsafe impl<T> Send for Handover<T> {}

    /// A running cast: stopping it is dropping it.
    pub struct Cast {
        pub size: (i32, i32),
        /// The CGDirectDisplayID shown, for the touches.
        pub display: u32,
        stream: Retained<AnyObject>,
        _output: Retained<Output>,
        _queue: dispatch2::DispatchRetained<dispatch2::DispatchQueue>,
        shared: Arc<Shared>,
        /// The Shared the encoder holds (its refcon), released after it's invalidated.
        refcon: *const Shared,
    }

    // SAFETY: the stream and output are only told to stop from another thread, which
    // ScreenCaptureKit allows; the rest is Send already.
    unsafe impl Send for Cast {}

    fn describe(error: *mut AnyObject) -> (isize, String) {
        if error.is_null() {
            return (0, "unknown error".into());
        }
        // SAFETY: an NSError ScreenCaptureKit handed us.
        unsafe {
            let code: isize = msg_send![error, code];
            let text: Retained<NSString> = msg_send![error, localizedDescription];
            (code, text.to_string())
        }
    }

    fn property(session: CFRef, key: CFRef, value: CFRef, what: &str) {
        // SAFETY: a property on a session we made; the value is retained by it.
        let status = unsafe { VTSessionSetProperty(session, key, value) };
        if status != 0 {
            tracing::debug!("encoder took no {what} ({status})");
        }
    }

    fn number(n: i32) -> CFRef {
        // SAFETY: a CFNumber from a local value; the caller releases it.
        unsafe { CFNumberCreate(std::ptr::null(), SINT32, (&raw const n).cast()) }
    }

    /// A real-time H.264 session at [w]x[h] that hands each picture to [shared].
    unsafe fn encoder(w: i32, h: i32, fps: u32, refcon: *mut c_void) -> Result<CFRef> {
        unsafe {
            // Low-latency rate control (macOS 11.3+; the key by name, so older Macs still load this).
            let low = CFStringCreateWithCString(std::ptr::null(), c"EnableLowLatencyRateControl".as_ptr(), UTF8);
            let keys = [low];
            let values = [kCFBooleanTrue];
            let spec: CFRef = CFDictionaryCreate(std::ptr::null_mut(), keys.as_ptr(), values.as_ptr(), 1, (&raw const kCFTypeDictionaryKeyCallBacks).cast(), (&raw const kCFTypeDictionaryValueCallBacks).cast());
            let mut session: CFRef = std::ptr::null();
            let mut status = VTCompressionSessionCreate(std::ptr::null(), w, h, AVC1, spec, std::ptr::null(), std::ptr::null(), encoded, refcon, &mut session);
            if status != 0 {
                tracing::info!("no low-latency encoder ({status}); the usual one");
                status = VTCompressionSessionCreate(std::ptr::null(), w, h, AVC1, std::ptr::null(), std::ptr::null(), std::ptr::null(), encoded, refcon, &mut session);
            }
            CFRelease(spec);
            CFRelease(low);
            if status != 0 || session.is_null() {
                bail!("This Mac couldn't start a video encoder ({status})");
            }
            property(session, kVTCompressionPropertyKey_RealTime, kCFBooleanTrue, "real time");
            property(session, kVTCompressionPropertyKey_ProfileLevel, kVTProfileLevel_H264_Main_AutoLevel, "profile");
            property(session, kVTCompressionPropertyKey_AllowFrameReordering, kCFBooleanFalse, "frame order");
            for (key, value, what) in [
                (kVTCompressionPropertyKey_AverageBitRate, BITRATE_N, "bit rate"),
                (kVTCompressionPropertyKey_MaxKeyFrameInterval, KEYFRAME_FRAMES, "keyframe interval"),
                (kVTCompressionPropertyKey_ExpectedFrameRate, fps as i32, "frame rate"),
            ] {
                let n = number(value);
                property(session, key, n, what);
                CFRelease(n);
            }
            VTCompressionSessionPrepareToEncodeFrames(session);
            Ok(session)
        }
    }

    const BITRATE_N: i32 = super::BITRATE;

    /// Waits for a completion handler's answer.
    fn wait<T>(rx: std::sync::mpsc::Receiver<T>, what: &str) -> Result<T> {
        rx.recv_timeout(SETUP_WAIT).with_context(|| format!("macOS didn't answer ({what})"))
    }

    impl Cast {
        /// Starts capturing the main display; the encoded video comes on the receiver.
        pub fn start(fps: u32) -> Result<(Cast, mpsc::UnboundedReceiver<Vec<u8>>)> {
            // This thread has no pool of its own: what ScreenCaptureKit hands back autoreleased goes here.
            objc2::rc::autoreleasepool(|_| Self::start_in_pool(fps))
        }

        fn start_in_pool(fps: u32) -> Result<(Cast, mpsc::UnboundedReceiver<Vec<u8>>)> {
            // SAFETY: loading a system framework by its fixed path.
            let loaded = unsafe { libc::dlopen(c"/System/Library/Frameworks/ScreenCaptureKit.framework/ScreenCaptureKit".as_ptr(), libc::RTLD_LAZY) };
            let class = |name: &std::ffi::CStr| AnyClass::get(name);
            if loaded.is_null() || class(c"SCStream").is_none() {
                bail!("Showing this Mac's screen needs macOS 12.3 or newer");
            }
            if !allowed(true) {
                bail!("{}", super::ALLOW_SHORT);
            }
            let (Some(content_class), Some(filter_class), Some(config_class), Some(stream_class)) =
                (class(c"SCShareableContent"), class(c"SCContentFilter"), class(c"SCStreamConfiguration"), class(c"SCStream"))
            else {
                bail!("ScreenCaptureKit isn't all there");
            };
            // SAFETY: ScreenCaptureKit's documented methods, with the argument and return types it declares.
            unsafe {
                let (tx, rx) = std::sync::mpsc::channel::<Result<Handover<Retained<AnyObject>>, (isize, String)>>();
                let got = RcBlock::new(move |content: *mut AnyObject, error: *mut AnyObject| {
                    let _ = tx.send(Retained::retain(content).map(Handover).ok_or_else(|| describe(error)));
                });
                let _: () = msg_send![content_class, getShareableContentWithCompletionHandler: &*got];
                let content = match wait(rx, "screens")? {
                    Ok(c) => c.0,
                    Err((DECLINED, _)) => bail!("{}", super::ALLOW_SHORT),
                    Err((_, why)) => bail!("macOS wouldn't share the screen: {why}"),
                };
                let displays: Retained<AnyObject> = msg_send![&*content, displays];
                let count: usize = msg_send![&*displays, count];
                if count == 0 {
                    bail!("No screen to show");
                }
                let main = CGMainDisplayID();
                let mut display: Retained<AnyObject> = msg_send![&*displays, objectAtIndex: 0usize];
                for i in 0..count {
                    let d: Retained<AnyObject> = msg_send![&*displays, objectAtIndex: i];
                    let id: u32 = msg_send![&*d, displayID];
                    if id == main {
                        display = d;
                    }
                }
                let id: u32 = msg_send![&*display, displayID];
                // Its real pixels (a Retina screen has twice its points), scaled to fit.
                let mode = CGDisplayCopyDisplayMode(id);
                let pixels = if mode.is_null() {
                    let (w, h): (isize, isize) = (msg_send![&*display, width], msg_send![&*display, height]);
                    (w as i32, h as i32)
                } else {
                    let p = (CGDisplayModeGetPixelWidth(mode) as i32, CGDisplayModeGetPixelHeight(mode) as i32);
                    CGDisplayModeRelease(mode);
                    p
                };
                let (w, h) = super::fit(pixels);
                if w < 16 || h < 16 {
                    bail!("The screen has no size");
                }

                let none: Retained<AnyObject> = msg_send![AnyClass::get(c"NSArray").context("no NSArray")?, array];
                let filter: Retained<AnyObject> = msg_send![msg_send![filter_class, alloc], initWithDisplay: &*display, excludingWindows: &*none];
                let config: Retained<AnyObject> = msg_send![config_class, new];
                let _: () = msg_send![&*config, setWidth: w as usize];
                let _: () = msg_send![&*config, setHeight: h as usize];
                let interval = CMTime { value: 1, timescale: fps as i32, flags: CMTimeFlags::Valid, epoch: 0 };
                let _: () = msg_send![&*config, setMinimumFrameInterval: interval];
                let _: () = msg_send![&*config, setPixelFormat: NV12];
                let _: () = msg_send![&*config, setShowsCursor: true];
                let _: () = msg_send![&*config, setQueueDepth: 3isize];

                let (out, video) = mpsc::unbounded_channel();
                let shared = Arc::new(Shared { session: Mutex::new(0), out, pending: Arc::new(AtomicUsize::new(0)) });
                let refcon = Arc::into_raw(shared.clone());
                let session = match encoder(w, h, fps, refcon as *mut c_void) {
                    Ok(s) => s,
                    Err(e) => {
                        drop(Arc::from_raw(refcon));
                        return Err(e);
                    }
                };
                *shared.session.lock().unwrap() = session as usize;
                let output = Output::new(shared.clone());
                let queue = dispatch2::DispatchQueue::new("dev.vyoma.screen", None);
                let stream: Retained<AnyObject> =
                    msg_send![msg_send![stream_class, alloc], initWithFilter: &*filter, configuration: &*config, delegate: std::ptr::null::<AnyObject>()];
                let cast = Cast { size: (w, h), display: id, stream, _output: output, _queue: queue, shared, refcon };
                let mut error: *mut AnyObject = std::ptr::null_mut();
                let queue_object = &*(&*cast._queue as *const dispatch2::DispatchQueue as *const AnyObject);
                let added: Bool = msg_send![&*cast.stream, addStreamOutput: &*cast._output, type: 0isize, sampleHandlerQueue: queue_object, error: &mut error];
                if !added.as_bool() {
                    bail!("macOS wouldn't share the screen: {}", describe(error).1);
                }
                let (tx, rx) = std::sync::mpsc::channel::<Option<(isize, String)>>();
                let started = RcBlock::new(move |error: *mut AnyObject| {
                    let _ = tx.send((!error.is_null()).then(|| describe(error)));
                });
                let _: () = msg_send![&*cast.stream, startCaptureWithCompletionHandler: &*started];
                match wait(rx, "starting")? {
                    None => Ok((cast, video)),
                    Some((DECLINED, _)) => bail!("{}", super::ALLOW_SHORT),
                    Some((_, why)) => bail!("macOS wouldn't share the screen: {why}"),
                }
            }
        }

        pub fn pending(&self) -> Arc<AtomicUsize> {
            self.shared.pending.clone()
        }
    }

    impl Drop for Cast {
        fn drop(&mut self) {
            // SAFETY: stopping our own stream, then invalidating our session under its lock (so the
            // capture queue can't be inside it), and only then letting the encoder's Shared go.
            unsafe {
                let done = RcBlock::new(|_: *mut AnyObject| {});
                let _: () = msg_send![&*self.stream, stopCaptureWithCompletionHandler: &*done];
                let mut session = self.shared.session.lock().unwrap();
                if *session != 0 {
                    VTCompressionSessionInvalidate(*session as CFRef);
                    CFRelease(*session as CFRef);
                    *session = 0;
                }
                drop(session);
                drop(Arc::from_raw(self.refcon));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_keyframe_leads_with_its_parameter_sets_and_a_stamp() {
        // An SEI and an IDR slice, as VideoToolbox gives them.
        let avcc = [&[0, 0, 0, 2, 0x06, 0xAA][..], &[0, 0, 0, 3, 0x65, 1, 2]].concat();
        let out = frame(&[&[0x67, 9], &[0x68, 8]], &avcc, 42);
        let mut want = vec![0, 0, 0, 2, 0x67, 9, 0, 0, 0, 2, 0x68, 8, 0, 0, 0, 2, 0x06, 0xAA, 0, 0, 0, 13, 0x06];
        want.extend_from_slice(STAMP_MAGIC);
        want.extend_from_slice(&42u64.to_be_bytes());
        want.extend_from_slice(&[0, 0, 0, 3, 0x65, 1, 2]);
        assert_eq!(out, want);
    }

    #[test]
    fn other_pictures_get_one_stamp_and_cut_input_stops_cleanly() {
        let avcc = [&[0, 0, 0, 2, 0x41, 1][..], &[0, 0, 0, 2, 0x41, 2]].concat();
        let out = frame(&[], &avcc, 7);
        assert_eq!(out.len(), 4 + 13 + avcc.len(), "one stamp before the first slice only");
        assert_eq!(frame(&[], &avcc[..5], 7), Vec::<u8>::new(), "a cut unit is left out");
    }

    #[test]
    fn sizes_and_rates() {
        assert_eq!(fit((2880, 1800)), (1920, 1200));
        assert_eq!(fit((1366, 769)), (1366, 768));
        assert_eq!(fps_for(None), 60);
        assert_eq!(fps_for(Some(240)), 120);
        assert_eq!(fps_for(Some(10)), 30);
    }
}
