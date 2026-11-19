//! The phone's screen in a window on this Mac, worked with the mouse, trackpad and keyboard. The
//! phone sends its screen as live H.264 (`vyoma.mirror`, Annex-B, a MediaProjection); the window's
//! clicks, drags, scrolls and typing go back as `vyoma.mirror.input` (0..1 across the picture),
//! which the phone's accessibility service performs. Closing the window ends it
//! (`vyoma.mirror.state {open:false}`). The phone's sound comes too (`vyoma.mirror.audio`, s16le PCM),
//! played through an AudioQueue; the phone copying text puts it on the Mac's clipboard, and its
//! volume keys turn the Mac's output up and down while its sound plays here.
//!
//! The window is this same binary run as a child (`--mirror-window`), as on Linux, so a decoder
//! problem can't take the agent down: AppKit's window with an AVSampleBufferDisplayLayer, which
//! decodes in hardware. It reads the stream on stdin and writes what the user does as JSON lines on
//! stdout. Mac keys as a Mac user expects: Command+C/X/A/V copy, cut, select all and paste (the
//! Mac's clipboard, typed into the phone), Option+arrows move by word, Command+arrows to the line's
//! ends, Escape or a right click is Back, a middle click Home.
//! Built and tested on CI only; not yet run on a real Mac.

use std::net::IpAddr;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use vyoma_core::identity::Identity;
use vyoma_core::link::{Sender, packet};
use vyoma_core::payload;


pub use vyoma_core::mirror::{INPUT, PASTE_MAX, STATE};


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
    // Sound as PCM: macOS plays it with nothing to decode.
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
                match crate::mac::clipboard().await {
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

/// The phone's volume keys while its sound plays here: the Mac's output, a step a press.
pub async fn volume(up: bool) {
    let script = if up {
        "set volume output volume ((output volume of (get volume settings)) + 6)"
    } else {
        "set volume output volume ((output volume of (get volume settings)) - 6)"
    };
    let _ = tokio::process::Command::new("osascript").args(["-e", script]).status().await;
}

/// What a key press in the window means on the phone. [code] is the Mac's virtual key code,
/// [chars] what it types, [flags] NSEventModifierFlags.
pub fn key_input(code: u16, chars: &str, flags: u64) -> Option<Value> {
    const SHIFT: u64 = 1 << 17;
    const CONTROL: u64 = 1 << 18;
    const OPTION: u64 = 1 << 19;
    const COMMAND: u64 = 1 << 20;
    let (shift, control, option, command) = (flags & SHIFT != 0, flags & CONTROL != 0, flags & OPTION != 0, flags & COMMAND != 0);
    let key = |k: &str, word: bool| Some(json!({ "kind": "key", "key": k, "ctrl": word, "shift": shift }));
    match code {
        53 => return Some(json!({ "kind": "back" })),
        48 if !command && !control => return Some(json!({ "kind": "text", "text": "\t" })),
        // Option+Delete deletes a word, as Ctrl+Backspace does on the phone.
        51 => return key("backspace", option),
        117 => return key("delete", option),
        36 | 76 => return key("enter", false),
        // Command+arrows: to the line's ends; Option+arrows: a word at a time.
        123 => return key(if command { "home" } else { "left" }, option),
        124 => return key(if command { "end" } else { "right" }, option),
        126 => return key("up", false),
        125 => return key("down", false),
        115 => return key("home", false),
        119 => return key("end", false),
        _ => {}
    }
    if command || control {
        let base = chars.to_lowercase();
        return match base.as_str() {
            "a" => Some(json!({ "kind": "key", "key": "selectall" })),
            "c" => Some(json!({ "kind": "key", "key": "copy" })),
            "x" => Some(json!({ "kind": "key", "key": "cut" })),
            "v" => Some(json!({ "kind": "paste" })),
            // Other shortcuts aren't typing.
            _ => None,
        };
    }
    // Option types real characters on a Mac (é, ©, …): what macOS says the key typed.
    let text: String = chars.chars().filter(|c| !c.is_control() && !('\u{f700}'..='\u{f8ff}').contains(c)).collect();
    (!text.is_empty()).then(|| json!({ "kind": "text", "text": text }))
}

/// The window process (`--mirror-window <title>`): H.264 on stdin, a window, inputs on stdout.
/// Exits when the window closes or the stream ends.
pub fn window(args: &[String]) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        view::run(args.first().cloned().unwrap_or_else(|| "Phone".into()))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = args;
        anyhow::bail!("only on a Mac")
    }
}

#[cfg(target_os = "macos")]
mod view {
    use std::cell::RefCell;
    use std::ffi::c_void;
    use std::io::{Read, Write};

    use anyhow::Result;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, ProtocolObject};
    use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSBackingStoreType, NSEvent, NSResponder, NSScreen, NSView, NSWindow, NSWindowStyleMask,
    };
    use objc2_foundation::{NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};
    use serde_json::{Value, json};

    type CFRef = *const c_void;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Dimensions {
        width: i32,
        height: i32,
    }

    #[link(name = "CoreMedia", kind = "framework")]
    unsafe extern "C" {
        fn CMVideoFormatDescriptionCreateFromH264ParameterSets(alloc: CFRef, count: usize, sets: *const *const u8, sizes: *const usize, header_length: i32, out: *mut CFRef) -> i32;
        fn CMVideoFormatDescriptionGetDimensions(format: CFRef) -> Dimensions;
        fn CMBlockBufferCreateWithMemoryBlock(alloc: CFRef, memory: *mut c_void, length: usize, block_alloc: CFRef, source: *const c_void, offset: usize, data_length: usize, flags: u32, out: *mut CFRef) -> i32;
        fn CMBlockBufferReplaceDataBytes(source: *const c_void, buffer: CFRef, offset: usize, length: usize) -> i32;
        fn CMSampleBufferCreateReady(alloc: CFRef, data: CFRef, format: CFRef, samples: isize, timings: isize, timing: *const c_void, sizes: isize, size: *const usize, out: *mut CFRef) -> i32;
        fn CMSampleBufferGetSampleAttachmentsArray(sample: CFRef, create: bool) -> CFRef;
        static kCMSampleAttachmentKey_DisplayImmediately: CFRef;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(object: CFRef);
        fn CFArrayGetValueAtIndex(array: CFRef, index: isize) -> CFRef;
        fn CFDictionarySetValue(dict: CFRef, key: CFRef, value: CFRef);
        static kCFBooleanTrue: CFRef;
    }

    #[link(name = "AVFoundation", kind = "framework")]
    unsafe extern "C" {
        static AVLayerVideoGravityResizeAspect: &'static NSString;
    }

    /// kCMBlockBufferAssureMemoryNowFlag.
    const MEMORY_NOW: u32 = 1;
    /// AVQueuedSampleBufferRenderingStatusFailed.
    const FAILED: isize = 2;
    /// A wheel notch is this many points of a trackpad's smooth scroll.
    const POINTS_A_NOTCH: f64 = 12.0;

    /// Writes one input line for the agent.
    fn emit(v: Value) {
        let mut out = std::io::stdout().lock();
        if writeln!(out, "{v}").and_then(|()| out.flush()).is_err() {
            // The agent's gone: nothing more to do.
            std::process::exit(0);
        }
    }

    struct ViewIvars {
        layer: Retained<AnyObject>,
    }

    define_class!(
        // The window's content: the decoded picture, and the mouse and keys over it.
        #[unsafe(super(NSView, NSResponder, NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "VyomaMirrorView"]
        #[ivars = ViewIvars]
        struct MirrorView;

        impl MirrorView {
            #[unsafe(method_id(makeBackingLayer))]
            fn make_backing_layer(&self) -> Retained<AnyObject> {
                self.ivars().layer.clone()
            }

            #[unsafe(method(acceptsFirstResponder))]
            fn accepts_first_responder(&self) -> bool {
                true
            }

            #[unsafe(method(acceptsFirstMouse:))]
            fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
                true
            }

            #[unsafe(method(mouseDown:))]
            fn mouse_down(&self, event: &NSEvent) {
                let (x, y) = self.at(event);
                emit(json!({ "kind": "down", "x": x, "y": y }));
            }

            #[unsafe(method(mouseDragged:))]
            fn mouse_dragged(&self, event: &NSEvent) {
                let (x, y) = self.at(event);
                emit(json!({ "kind": "move", "x": x, "y": y }));
            }

            #[unsafe(method(mouseUp:))]
            fn mouse_up(&self, event: &NSEvent) {
                let (x, y) = self.at(event);
                emit(json!({ "kind": "up", "x": x, "y": y }));
            }

            #[unsafe(method(rightMouseDown:))]
            fn right_mouse_down(&self, _event: &NSEvent) {
                emit(json!({ "kind": "back" }));
            }

            // The middle button is Home; a mouse's back button (3) is Back.
            #[unsafe(method(otherMouseDown:))]
            fn other_mouse_down(&self, event: &NSEvent) {
                emit(json!({ "kind": if event.buttonNumber() == 2 { "home" } else { "back" } }));
            }

            #[unsafe(method(scrollWheel:))]
            fn scroll_wheel(&self, event: &NSEvent) {
                let (x, y) = self.at(event);
                let scale = if event.hasPreciseScrollingDeltas() { POINTS_A_NOTCH } else { 1.0 };
                // macOS's positive is towards the top; the phone's, as a wheel turned towards you.
                let (dx, dy) = (-event.scrollingDeltaX() / scale, -event.scrollingDeltaY() / scale);
                if dx != 0.0 || dy != 0.0 {
                    emit(json!({ "kind": "scroll", "x": x, "y": y, "dx": dx, "dy": dy }));
                }
            }

            #[unsafe(method(keyDown:))]
            fn key_down(&self, event: &NSEvent) {
                let chars = event.characters().map(|c| c.to_string()).unwrap_or_default();
                if let Some(v) = super::key_input(event.keyCode(), &chars, event.modifierFlags().0 as u64) {
                    emit(v);
                }
            }

            // Command+key arrives here first; taken, so the menu bar (there's none) doesn't beep.
            #[unsafe(method(performKeyEquivalent:))]
            fn perform_key_equivalent(&self, event: &NSEvent) -> bool {
                let chars = event.characters().map(|c| c.to_string()).unwrap_or_default();
                // Command+W and Command+Q close the window, as everywhere on a Mac.
                let command = event.modifierFlags().0 as u64 & (1 << 20) != 0;
                if command && matches!(chars.to_lowercase().as_str(), "w" | "q") {
                    if let Some(window) = self.window() {
                        window.performClose(None);
                    }
                    true
                } else if let Some(v) = super::key_input(event.keyCode(), &chars, event.modifierFlags().0 as u64) {
                    emit(v);
                    true
                } else {
                    false
                }
            }
        }
    );

    impl MirrorView {
        fn new(mtm: MainThreadMarker, layer: Retained<AnyObject>) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(ViewIvars { layer });
            // SAFETY: NSView's init on a freshly allocated view.
            unsafe { msg_send![super(this), init] }
        }

        /// Where [event] is, 0..1 across and down the picture.
        fn at(&self, event: &NSEvent) -> (f64, f64) {
            let p = self.convertPoint_fromView(event.locationInWindow(), None);
            let b = self.bounds();
            let (w, h) = (b.size.width.max(1.0), b.size.height.max(1.0));
            ((p.x / w).clamp(0.0, 1.0), (1.0 - p.y / h).clamp(0.0, 1.0))
        }
    }

    define_class!(
        // Closing the window quits this process (the agent sees its stdout end).
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "VyomaMirrorApp"]
        struct AppDelegate;

        unsafe impl NSObjectProtocol for AppDelegate {}

        unsafe impl NSApplicationDelegate for AppDelegate {
            #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
            fn last_window_closed(&self, _app: &NSApplication) -> bool {
                true
            }
        }
    );

    impl AppDelegate {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            // SAFETY: NSObject's init on a freshly allocated object.
            unsafe { msg_send![Self::alloc(mtm), init] }
        }
    }

    thread_local! {
        static WINDOW: RefCell<Option<Retained<NSWindow>>> = const { RefCell::new(None) };
    }

    /// The window fitted to a [w]x[h] picture: as large as its pixels, at most 85% of the screen.
    fn fit(mtm: MainThreadMarker, w: i32, h: i32) {
        WINDOW.with_borrow(|window| {
            let Some(window) = window else { return };
            let (screen, scale) = NSScreen::mainScreen(mtm).map_or((NSSize::new(1200.0, 800.0), 2.0), |s| (s.visibleFrame().size, s.backingScaleFactor()));
            let (w, h) = (w.max(1) as f64, h.max(1) as f64);
            let mut size = NSSize::new(w / scale, h / scale);
            let shrink = (screen.width * 0.85 / size.width).min(screen.height * 0.85 / size.height).min(1.0);
            size = NSSize::new((size.width * shrink).round(), (size.height * shrink).round());
            window.setContentAspectRatio(NSSize::new(w, h));
            window.setContentSize(size);
            window.center();
        });
    }

    /// Reads the stream, decodes it into [layer], and fits the window to each new picture size.
    fn decode(layer: usize) {
        let layer = layer as *mut AnyObject;
        let mut input = std::io::stdin().lock();
        let mut buf: Vec<u8> = Vec::with_capacity(512 * 1024);
        let mut chunk = vec![0u8; 64 * 1024];
        let (mut sps, mut pps): (Vec<u8>, Vec<u8>) = (Vec::new(), Vec::new());
        let mut format: CFRef = std::ptr::null();
        let mut fresh = false;
        loop {
            let n = match input.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            buf.extend_from_slice(&chunk[..n]);
            let (nals, used) = vyoma_core::mirror::split_nals(&buf);
            for nal in nals {
                match nal[0] & 0x1f {
                    7 => {
                        fresh |= sps != nal;
                        sps = nal.to_vec();
                    }
                    8 => {
                        fresh |= pps != nal;
                        pps = nal.to_vec();
                    }
                    1 | 5 => {
                        if fresh && !sps.is_empty() && !pps.is_empty() {
                            fresh = false;
                            // SAFETY: CoreMedia makes a description from our own parameter sets.
                            unsafe {
                                let sets = [sps.as_ptr(), pps.as_ptr()];
                                let sizes = [sps.len(), pps.len()];
                                let mut made: CFRef = std::ptr::null();
                                if CMVideoFormatDescriptionCreateFromH264ParameterSets(std::ptr::null(), 2, sets.as_ptr(), sizes.as_ptr(), 4, &mut made) == 0 {
                                    if !format.is_null() {
                                        CFRelease(format);
                                    }
                                    format = made;
                                    let d = CMVideoFormatDescriptionGetDimensions(format);
                                    dispatch2::DispatchQueue::main().exec_async(move || {
                                        if let Some(mtm) = MainThreadMarker::new() {
                                            fit(mtm, d.width, d.height);
                                        }
                                    });
                                }
                            }
                        }
                        if !format.is_null() {
                            // SAFETY: the layer lives as long as this process; the sample is ours.
                            unsafe { show(layer, format, nal) };
                        }
                    }
                    _ => {}
                }
            }
            buf.drain(..used);
        }
        // The phone stopped.
        std::process::exit(0);
    }

    /// One picture onto the layer, shown as soon as it's decoded.
    unsafe fn show(layer: *mut AnyObject, format: CFRef, nal: &[u8]) {
        unsafe {
            let len = nal.len() + 4;
            let mut block: CFRef = std::ptr::null();
            if CMBlockBufferCreateWithMemoryBlock(std::ptr::null(), std::ptr::null_mut(), len, std::ptr::null(), std::ptr::null(), 0, len, MEMORY_NOW, &mut block) != 0 {
                return;
            }
            let avcc = [&(nal.len() as u32).to_be_bytes()[..], nal].concat();
            CMBlockBufferReplaceDataBytes(avcc.as_ptr().cast(), block, 0, len);
            let mut sample: CFRef = std::ptr::null();
            let made = CMSampleBufferCreateReady(std::ptr::null(), block, format, 1, 0, std::ptr::null(), 1, &len, &mut sample);
            CFRelease(block);
            if made != 0 || sample.is_null() {
                return;
            }
            let attachments = CMSampleBufferGetSampleAttachmentsArray(sample, true);
            if !attachments.is_null() {
                CFDictionarySetValue(CFArrayGetValueAtIndex(attachments, 0), kCMSampleAttachmentKey_DisplayImmediately, kCFBooleanTrue);
            }
            let status: isize = msg_send![layer, status];
            if status == FAILED {
                let _: () = msg_send![layer, flush];
            }
            let _: () = msg_send![layer, enqueueSampleBuffer: sample];
            CFRelease(sample);
        }
    }

    pub fn run(title: String) -> Result<()> {
        let mtm = MainThreadMarker::new().ok_or_else(|| anyhow::anyhow!("not on the main thread"))?;
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        let delegate = AppDelegate::new(mtm);
        app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

        let class = AnyClass::get(c"AVSampleBufferDisplayLayer").ok_or_else(|| anyhow::anyhow!("no AVSampleBufferDisplayLayer"))?;
        // SAFETY: AVFoundation's documented layer and its video gravity.
        let layer: Retained<AnyObject> = unsafe { msg_send![class, new] };
        unsafe {
            let _: () = msg_send![&*layer, setVideoGravity: AVLayerVideoGravityResizeAspect];
        }
        let view = MirrorView::new(mtm, layer.clone());
        view.setWantsLayer(true);

        let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Miniaturizable | NSWindowStyleMask::Resizable;
        // SAFETY: a window made and used on the main thread; released by us, not on close.
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(NSWindow::alloc(mtm), NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(360.0, 760.0)), style, NSBackingStoreType::Buffered, false)
        };
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&NSString::from_str(&format!("{title} \u{b7} Vyoma")));
        window.setContentView(Some(&view));
        window.center();
        window.makeKeyAndOrderFront(None);
        window.makeFirstResponder(Some(&view));
        WINDOW.with_borrow_mut(|w| *w = Some(window));

        let raw = Retained::as_ptr(&layer) as usize;
        std::thread::Builder::new().name("decode".into()).spawn(move || decode(raw))?;
        #[allow(deprecated)] // NSApp.activate is macOS 14+.
        app.activateIgnoringOtherApps(true);
        app.run();
        Ok(())
    }
}

/// The phone's sound: s16le PCM in 10 ms pieces through a small jitter buffer into an AudioQueue.
pub mod sound {
    use std::net::IpAddr;

    use anyhow::Result;
    use serde_json::Value;
    use vyoma_core::identity::Identity;
    use vyoma_core::mirror::{self, Queue, Sound};

    /// Plays the phone's sound until it stops.
    pub async fn play(me: &Identity, pinned: &[u8], phone: IpAddr, p: &Value, name: &str) -> Result<()> {
        let sound = Sound::of(&p["body"])?;
        let queue = Queue::shared();
        let _output = output::Output::start(sound.rate, sound.channels, queue.clone())?;
        tracing::info!("{name}: playing the phone's sound");
        mirror::receive(me, pinned, phone, p, sound, &queue).await?;
        tracing::info!("{name}: phone's sound stopped");
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    mod output {
        pub struct Output;

        impl Output {
            pub fn start(_: u32, _: u32, _: vyoma_core::mirror::Shared) -> anyhow::Result<Self> {
                Ok(Output)
            }
        }
    }

    #[cfg(target_os = "macos")]
    mod output {
        use std::ffi::c_void;

        use anyhow::{Result, bail};

        type AQ = *mut c_void;

        #[repr(C)]
        struct Format {
            sample_rate: f64,
            format_id: u32,
            format_flags: u32,
            bytes_per_packet: u32,
            frames_per_packet: u32,
            bytes_per_frame: u32,
            channels: u32,
            bits: u32,
            reserved: u32,
        }

        #[repr(C)]
        struct Buffer {
            capacity: u32,
            data: *mut c_void,
            size: u32,
            user: *mut c_void,
            packet_capacity: u32,
            packets: *mut c_void,
            packet_count: u32,
        }

        type Callback = extern "C" fn(*mut c_void, AQ, *mut Buffer);

        #[link(name = "AudioToolbox", kind = "framework")]
        unsafe extern "C" {
            fn AudioQueueNewOutput(format: *const Format, callback: Callback, user: *mut c_void, run_loop: *const c_void, mode: *const c_void, flags: u32, out: *mut AQ) -> i32;
            fn AudioQueueAllocateBuffer(queue: AQ, size: u32, out: *mut *mut Buffer) -> i32;
            fn AudioQueueEnqueueBuffer(queue: AQ, buffer: *mut Buffer, packets: u32, descriptions: *const c_void) -> i32;
            fn AudioQueueStart(queue: AQ, at: *const c_void) -> i32;
            fn AudioQueueDispose(queue: AQ, immediate: bool) -> i32;
        }

        const LPCM: u32 = u32::from_be_bytes(*b"lpcm");
        /// kLinearPCMFormatFlagIsSignedInteger | kLinearPCMFormatFlagIsPacked.
        const SIGNED_PACKED: u32 = 0x4 | 0x8;
        /// Three buffers of 20 ms: about 40 ms queued in the output.
        const BUFFERS: usize = 3;
        const BUFFER_MS: u32 = 20;

        /// The AudioQueue's thread asks for more: from the jitter buffer, silence where there's none.
        extern "C" fn refill(user: *mut c_void, queue: AQ, buffer: *mut Buffer) {
            // SAFETY: user is the Shared leaked in [Output::start], freed after the queue's disposed;
            // the buffer is one of ours, handed back by the queue.
            unsafe {
                let shared = &*(user as *const std::sync::Mutex<vyoma_core::mirror::Queue>);
                let b = &mut *buffer;
                let out = std::slice::from_raw_parts_mut(b.data as *mut u8, b.capacity as usize);
                if let Ok(mut q) = shared.lock() {
                    q.fill(out);
                } else {
                    out.fill(0);
                }
                b.size = b.capacity;
                AudioQueueEnqueueBuffer(queue, buffer, 0, std::ptr::null());
            }
        }

        pub struct Output {
            queue: AQ,
            user: *const std::sync::Mutex<vyoma_core::mirror::Queue>,
        }

        // SAFETY: the queue is only disposed of (from whichever thread drops it), which AudioToolbox allows.
        unsafe impl Send for Output {}

        impl Output {
            pub fn start(rate: u32, channels: u32, shared: vyoma_core::mirror::Shared) -> Result<Self> {
                let format = Format {
                    sample_rate: rate as f64,
                    format_id: LPCM,
                    format_flags: SIGNED_PACKED,
                    bytes_per_packet: 2 * channels,
                    frames_per_packet: 1,
                    bytes_per_frame: 2 * channels,
                    channels,
                    bits: 16,
                    reserved: 0,
                };
                let user = std::sync::Arc::into_raw(shared);
                let mut queue: AQ = std::ptr::null_mut();
                // SAFETY: AudioToolbox calls with our own format and buffers; on failure the Shared is taken back.
                unsafe {
                    if AudioQueueNewOutput(&format, refill, user as *mut c_void, std::ptr::null(), std::ptr::null(), 0, &mut queue) != 0 || queue.is_null() {
                        drop(std::sync::Arc::from_raw(user));
                        bail!("This Mac couldn't play the sound");
                    }
                    let size = rate / 1000 * BUFFER_MS * 2 * channels;
                    for _ in 0..BUFFERS {
                        let mut buffer: *mut Buffer = std::ptr::null_mut();
                        if AudioQueueAllocateBuffer(queue, size, &mut buffer) == 0 && !buffer.is_null() {
                            refill(user as *mut c_void, queue, buffer);
                        }
                    }
                    if AudioQueueStart(queue, std::ptr::null()) != 0 {
                        AudioQueueDispose(queue, true);
                        drop(std::sync::Arc::from_raw(user));
                        bail!("This Mac couldn't play the sound");
                    }
                }
                Ok(Output { queue, user })
            }
        }

        impl Drop for Output {
            fn drop(&mut self) {
                // SAFETY: disposing at once stops the callbacks, then the Shared can go.
                unsafe {
                    AudioQueueDispose(self.queue, true);
                    drop(std::sync::Arc::from_raw(self.user));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_keys_as_a_mac_user_expects() {
        const SHIFT: u64 = 1 << 17;
        const OPTION: u64 = 1 << 19;
        const COMMAND: u64 = 1 << 20;
        assert_eq!(key_input(0, "a", 0).unwrap()["text"], "a");
        assert_eq!(key_input(0, "A", SHIFT).unwrap()["text"], "A");
        assert_eq!(key_input(14, "é", OPTION).unwrap()["text"], "é", "Option types characters");
        assert_eq!(key_input(53, "\u{1b}", 0).unwrap()["kind"], "back");
        assert_eq!(key_input(9, "v", COMMAND).unwrap()["kind"], "paste");
        assert_eq!(key_input(8, "c", COMMAND).unwrap()["key"], "copy");
        assert_eq!(key_input(0, "a", COMMAND).unwrap()["key"], "selectall");
        assert!(key_input(12, "q", COMMAND).is_none(), "other shortcuts aren't typing");
        let word = key_input(51, "\u{7f}", OPTION).unwrap();
        assert_eq!((word["key"].as_str(), word["ctrl"].as_bool()), (Some("backspace"), Some(true)));
        assert_eq!(key_input(123, "\u{f702}", COMMAND).unwrap()["key"], "home");
        assert_eq!(key_input(124, "\u{f703}", SHIFT).unwrap()["shift"], true);
        assert_eq!(key_input(48, "\t", 0).unwrap()["text"], "\t");
        assert!(key_input(122, "\u{f704}", 0).is_none(), "function keys type nothing");
    }
}
