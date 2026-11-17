//! The USB cable on the Mac: plug the phone in with USB debugging on, and the shared adb setup
//! (`vyoma_core::adb`: the tunnel the phone dials over the cable, payloads both ways, and the
//! clipboard grants after the person here allows the app's signing key) runs as on Linux.
//!
//! IOKit tells when a USB interface that speaks adb (class ff, subclass 42, protocol 01) comes or
//! goes, so nothing polls and no adb server runs until a phone is plugged in. adb itself is Google's
//! platform tools: Android Studio's, Homebrew's, or a copy fetched into Vyoma's folder when the
//! person asks for it ([fetch_adb]: This Mac > USB cable > Get adb).
//! Built and tested on CI only; not yet run on a real Mac.

use std::collections::HashSet;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result, bail};

/// The adb interfaces plugged in now, by IOKit's registry id; kept for a watcher started later.
static PLUGGED: OnceLock<tokio::sync::watch::Sender<HashSet<String>>> = OnceLock::new();

/// Google's platform tools for macOS (Apache-2.0 sources; the binaries under Google's SDK terms).
const TOOLS_URL: &str = "https://dl.google.com/android/repository/platform-tools-latest-darwin.zip";
const FETCH_WAIT: Duration = Duration::from_secs(300);

/// Starts noticing phones on the cable, with the cable leading to this Mac's [port].
pub fn start(port: u16) {
    vyoma_core::adb::set_port(port);
    vyoma_core::adb::set_asker(Box::new(|q| Box::pin(ask(q))));
    let (tx, rx) = tokio::sync::watch::channel(HashSet::new());
    if PLUGGED.set(tx).is_err() {
        return;
    }
    #[cfg(target_os = "macos")]
    std::thread::Builder::new().name("usb".into()).spawn(iokit::run).ok();
    tokio::spawn(vyoma_core::adb::watch(rx));
}

/// Starts the shared watcher again once adb is there (it gives up at start without it).
fn rewatch() {
    if let Some(tx) = PLUGGED.get() {
        let mut rx = tx.subscribe();
        // So what's plugged in already is set up, not only what comes after.
        rx.mark_changed();
        tokio::spawn(vyoma_core::adb::watch(rx));
    }
}

/// Phones plugged in with USB debugging right now.
pub fn plugged() -> usize {
    PLUGGED.get().map_or(0, |tx| tx.borrow().len())
}

/// The question in a dialog with Allow and Not now; giving up after [vyoma_core::adb::ASK_WAIT] is no answer.
async fn ask(q: vyoma_core::adb::Question) -> Option<bool> {
    let child = crate::mac::ask(&q.title, &q.body, "Not now", "Allow", vyoma_core::adb::ASK_WAIT).ok()?;
    let out = child.wait_with_output().await.ok()?;
    match String::from_utf8_lossy(&out.stdout).trim() {
        "Allow" => Some(true),
        "Not now" => Some(false),
        _ => None,
    }
}

/// The window's Get adb: Google's platform tools for macOS into Vyoma's folder, then the cable works.
pub async fn fetch_adb() -> Result<()> {
    if vyoma_core::adb::find_adb().is_some() {
        rewatch();
        return Ok(());
    }
    let dir = vyoma_core::identity::dir();
    std::fs::create_dir_all(&dir)?;
    let zip = dir.join("platform-tools.zip");
    let fetched = tokio::time::timeout(
        FETCH_WAIT,
        tokio::process::Command::new("curl").args(["-fsSL", "--proto", "=https", "-o"]).arg(&zip).arg(TOOLS_URL).kill_on_drop(true).status(),
    )
    .await
    .context("the download took too long")?
    .context("no curl")?;
    if !fetched.success() {
        let _ = std::fs::remove_file(&zip);
        bail!("Couldn't download Google's platform tools. Check the internet connection");
    }
    // The zip holds platform-tools/: it lands as Vyoma's own copy (adb::own_tools).
    let old = vyoma_core::adb::own_tools();
    let _ = std::fs::remove_dir_all(&old);
    let unpacked = tokio::process::Command::new("ditto").args(["-x", "-k"]).arg(&zip).arg(&dir).status().await.context("no ditto")?;
    let _ = std::fs::remove_file(&zip);
    if !unpacked.success() || !old.join("adb").is_file() {
        bail!("The download wasn't Google's platform tools");
    }
    tracing::info!("adb is in place");
    rewatch();
    Ok(())
}

#[cfg(target_os = "macos")]
mod iokit {
    use std::collections::HashSet;
    use std::ffi::{c_char, c_void};

    type CFRef = *const c_void;
    type IoObject = u32;

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IONotificationPortCreate(main_port: u32) -> *mut c_void;
        fn IONotificationPortGetRunLoopSource(port: *mut c_void) -> CFRef;
        fn IOServiceMatching(name: *const c_char) -> CFRef;
        fn IOServiceAddMatchingNotification(
            port: *mut c_void,
            kind: *const c_char,
            matching: CFRef,
            callback: extern "C" fn(*mut c_void, IoObject),
            refcon: *mut c_void,
            iterator: *mut IoObject,
        ) -> i32;
        fn IOIteratorNext(iterator: IoObject) -> IoObject;
        fn IOObjectRelease(object: IoObject) -> i32;
        fn IORegistryEntryGetRegistryEntryID(entry: IoObject, id: *mut u64) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRunLoopGetCurrent() -> CFRef;
        fn CFRunLoopAddSource(run_loop: CFRef, source: CFRef, mode: CFRef);
        fn CFRunLoopRun();
        fn CFDictionarySetValue(dict: CFRef, key: CFRef, value: CFRef);
        fn CFNumberCreate(alloc: CFRef, kind: isize, value: *const c_void) -> CFRef;
        fn CFStringCreateWithCString(alloc: CFRef, text: *const c_char, encoding: u32) -> CFRef;
        fn CFRetain(object: CFRef) -> CFRef;
        fn CFRelease(object: CFRef);
        static kCFRunLoopDefaultMode: CFRef;
    }

    const UTF8: u32 = 0x0800_0100;
    const SINT32: isize = 3;

    struct Watch {
        plugged: HashSet<String>,
    }

    fn id(service: IoObject) -> String {
        let mut id = 0u64;
        // SAFETY: a plain getter on a registry entry IOKit handed us.
        unsafe { IORegistryEntryGetRegistryEntryID(service, &mut id) };
        format!("usb-{id:x}")
    }

    /// Takes every service off [iterator] (which also arms it for the next), adding or removing each.
    fn drain(watch: &mut Watch, iterator: IoObject, add: bool) {
        loop {
            // SAFETY: an iterator IOKit gave us; each service is released after use.
            let service = unsafe { IOIteratorNext(iterator) };
            if service == 0 {
                break;
            }
            let id = id(service);
            if add {
                watch.plugged.insert(id);
            } else {
                watch.plugged.remove(&id);
            }
            unsafe { IOObjectRelease(service) };
        }
        if let Some(tx) = super::PLUGGED.get() {
            let now = watch.plugged.clone();
            tx.send_if_modified(|known| std::mem::replace(known, now) != *known);
        }
    }

    extern "C" fn added(refcon: *mut c_void, iterator: IoObject) {
        // SAFETY: refcon is the leaked Watch, only ever used on this thread's run loop.
        drain(unsafe { &mut *(refcon as *mut Watch) }, iterator, true);
    }

    extern "C" fn removed(refcon: *mut c_void, iterator: IoObject) {
        drain(unsafe { &mut *(refcon as *mut Watch) }, iterator, false);
    }

    /// An IOUSBHostInterface matching dictionary for adb's class, subclass and protocol.
    unsafe fn matching() -> CFRef {
        unsafe {
            let dict = IOServiceMatching(c"IOUSBHostInterface".as_ptr());
            if dict.is_null() {
                return dict;
            }
            for (key, value) in [(c"bInterfaceClass", 0xffi32), (c"bInterfaceSubClass", 0x42), (c"bInterfaceProtocol", 0x01)] {
                let k = CFStringCreateWithCString(std::ptr::null(), key.as_ptr(), UTF8);
                let v = CFNumberCreate(std::ptr::null(), SINT32, (&raw const value).cast());
                CFDictionarySetValue(dict, k, v);
                CFRelease(k);
                CFRelease(v);
            }
            dict
        }
    }

    /// This thread's run loop, for good: IOKit calls [added] and [removed] on it.
    pub fn run() {
        // SAFETY: IOKit and CoreFoundation calls on objects made here; each add consumes one
        // reference to the matching dictionary, so it's retained once for the second.
        unsafe {
            let port = IONotificationPortCreate(0);
            if port.is_null() {
                return tracing::warn!("no IOKit notifications: the USB cable won't be noticed");
            }
            CFRunLoopAddSource(CFRunLoopGetCurrent(), IONotificationPortGetRunLoopSource(port), kCFRunLoopDefaultMode);
            let dict = matching();
            if dict.is_null() {
                return;
            }
            CFRetain(dict);
            let watch = Box::into_raw(Box::new(Watch { plugged: HashSet::new() })) as *mut c_void;
            let (mut first, mut gone) = (0, 0);
            IOServiceAddMatchingNotification(port, c"IOServiceFirstMatch".as_ptr(), dict, added, watch, &mut first);
            IOServiceAddMatchingNotification(port, c"IOServiceTerminate".as_ptr(), dict, removed, watch, &mut gone);
            // What's plugged in already, and arming both.
            added(watch, first);
            removed(watch, gone);
            CFRunLoopRun();
        }
    }
}
