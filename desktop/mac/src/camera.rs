//! A photo with this Mac's camera, from the phone (`vyoma.camera.request` → `vyoma.camera`, a JPEG,
//! as from the Linux app). AVFoundation's photo output takes it, a moment after the camera starts so
//! its exposure has settled. Every photo also shows a notification here: someone at the Mac should
//! always know its camera was used from elsewhere (the green light comes on as well).
//!
//! macOS asks once whether Vyoma may use the camera; that needs Vyoma.app (its Info.plist says
//! why), so the bare binary refuses rather than be stopped by macOS.
//! Built and tested on CI only; not yet run on a real Mac.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use serde_json::json;
use vyoma_core::identity::Identity;
use vyoma_core::link::{Sender, packet};
use vyoma_core::payload;


pub const CAMERA: &str = "vyoma.camera";

/// System Settings > Privacy & Security > Camera.
pub const CAMERA_SETTINGS: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_Camera";
pub const ALLOW_SHORT: &str = "The Mac hasn't allowed Vyoma to use its camera: on the Mac, System Settings > Privacy & Security > Camera, turn on Vyoma";
const NO_CAMERA: &str = "this Mac has no camera";
const LID_CLOSED: &str = "the MacBook's lid is closed, so its camera can't see";
const NEEDS_APP: &str = "photos with the camera need Vyoma.app: open Vyoma from Applications";

/// How long the camera runs before the photo: it starts dark and needs a moment to expose.
const WARMUP: Duration = Duration::from_millis(900);
/// How long macOS's question waits for an answer.
const ASK_WAIT: Duration = Duration::from_secs(120);
/// The photo itself, once asked for.
const TAKE_WAIT: Duration = Duration::from_secs(15);
/// How long a look at the cameras holds (the stats ask every 2 s).
const STATE_HOLDS: Duration = Duration::from_secs(10);

/// What macOS says about Vyoma and the camera (AVAuthorizationStatus).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    NotAsked,
    Refused,
    Allowed,
}

impl Access {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn of(status: isize) -> Access {
        match status {
            3 => Access::Allowed,
            0 => Access::NotAsked,
            // 1 restricted (by a profile), 2 denied.
            _ => Access::Refused,
        }
    }
}

/// Why no photo can be taken now, for the phone: no camera at all, or only a closed lid in the way.
fn unavailable(lid_closed: bool) -> &'static str {
    if lid_closed { LID_CLOSED } else { NO_CAMERA }
}

/// Whether a camera can take a photo now, and with none, whether a closed lid is why (the phone's
/// stats `camera` and `cameraLid`). Looked at once in [STATE_HOLDS].
pub async fn state() -> (bool, bool) {
    static LAST: Mutex<Option<(Instant, bool, bool)>> = Mutex::new(None);
    if let Some((at, ok, lid)) = *LAST.lock().unwrap()
        && at.elapsed() < STATE_HOLDS
    {
        return (ok, lid);
    }
    let ok = tokio::task::spawn_blocking(present).await.unwrap_or(false);
    let lid = !ok && lid_closed().await;
    *LAST.lock().unwrap() = Some((Instant::now(), ok, lid));
    (ok, lid)
}

/// A MacBook shut with a screen plugged in (clamshell): its camera is off (IOKit's power state).
async fn lid_closed() -> bool {
    let Ok(out) = tokio::process::Command::new("ioreg").args(["-r", "-k", "AppleClamshellState", "-d", "1"]).kill_on_drop(true).output().await else {
        return false;
    };
    clamshell(&String::from_utf8_lossy(&out.stdout))
}

fn clamshell(ioreg: &str) -> bool {
    ioreg.lines().any(|l| l.contains("\"AppleClamshellState\"") && l.trim_end().ends_with("Yes"))
}

/// Whether there's a camera at all (asking needs no permission).
fn present() -> bool {
    #[cfg(target_os = "macos")]
    {
        av::present()
    }
    #[cfg(not(target_os = "macos"))]
    false
}

/// What macOS lets Vyoma do with the camera.
pub fn access() -> Access {
    #[cfg(target_os = "macos")]
    {
        av::access()
    }
    #[cfg(not(target_os = "macos"))]
    Access::NotAsked
}

/// Has macOS ask whether Vyoma may use the camera (the This Mac check's Allow); its answer.
pub async fn ask() -> Access {
    #[cfg(target_os = "macos")]
    {
        if !av::in_app() {
            return Access::NotAsked;
        }
        tokio::task::spawn_blocking(|| av::ask(ASK_WAIT)).await.unwrap_or(Access::NotAsked)
    }
    #[cfg(not(target_os = "macos"))]
    Access::NotAsked
}

/// One JPEG from the camera, asking macOS first if it never has been.
async fn capture() -> Result<Vec<u8>> {
    #[cfg(target_os = "macos")]
    {
        if !present() {
            bail!(unavailable(lid_closed().await));
        }
        if !av::in_app() {
            bail!(NEEDS_APP);
        }
        let allowed = match access() {
            Access::Allowed => true,
            Access::NotAsked => ask().await == Access::Allowed,
            Access::Refused => false,
        };
        if !allowed {
            bail!(ALLOW_SHORT);
        }
        tokio::task::spawn_blocking(|| av::photo(WARMUP, TAKE_WAIT)).await?
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (unavailable(false), NEEDS_APP, ALLOW_SHORT, WARMUP, TAKE_WAIT, ASK_WAIT);
        bail!(NO_CAMERA)
    }
}

pub async fn take(sender: &Sender, me: &Identity, pinned: &[u8], phone: &str) -> Result<()> {
    let photo = match capture().await {
        Ok(b) => b,
        Err(e) => {
            sender.send(&packet(CAMERA, json!({ "error": format!("{e:#}") }))).await?;
            return Err(e);
        }
    };
    crate::mac::notify("Camera used from your phone", &format!("{phone} took a photo with this Mac's camera.")).await;
    payload::offer(sender, me, pinned, packet(CAMERA, json!({ "format": "jpg" })), photo.len() as u64, photo.as_slice()).await
}

#[cfg(target_os = "macos")]
mod av {
    use std::sync::Mutex;
    use std::time::Duration;

    use anyhow::{Context, Result, bail};
    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Bool};
    use objc2::{AllocAnyThread, DefinedClass, define_class, msg_send};
    use objc2_foundation::{NSObject, NSString};

    use super::Access;

    #[link(name = "AVFoundation", kind = "framework")]
    unsafe extern "C" {
        static AVMediaTypeVideo: &'static NSString;
        static AVCaptureSessionPresetPhoto: &'static NSString;
        static AVVideoCodecKey: &'static NSString;
        static AVVideoCodecTypeJPEG: &'static NSString;
    }

    fn class(name: &std::ffi::CStr) -> Result<&'static AnyClass> {
        AnyClass::get(name).with_context(|| format!("no {}", name.to_string_lossy()))
    }

    fn describe(error: *mut AnyObject) -> String {
        if error.is_null() {
            return "unknown error".into();
        }
        // SAFETY: an NSError AVFoundation handed us.
        unsafe {
            let text: Retained<NSString> = msg_send![error, localizedDescription];
            text.to_string()
        }
    }

    /// Run from Vyoma.app, whose Info.plist tells macOS why it wants the camera: without that,
    /// macOS stops the process the moment it asks.
    pub fn in_app() -> bool {
        objc2::rc::autoreleasepool(|_| {
            // SAFETY: NSBundle's documented getters.
            unsafe {
                let Ok(bundle) = class(c"NSBundle") else { return false };
                let main: Option<Retained<AnyObject>> = msg_send![bundle, mainBundle];
                let Some(main) = main else { return false };
                let key = NSString::from_str("NSCameraUsageDescription");
                let why: Option<Retained<AnyObject>> = msg_send![&*main, objectForInfoDictionaryKey: &*key];
                why.is_some()
            }
        })
    }

    pub fn present() -> bool {
        objc2::rc::autoreleasepool(|_| {
            // SAFETY: AVCaptureDevice's class method; listing devices needs no permission.
            unsafe {
                let Ok(devices) = class(c"AVCaptureDevice") else { return false };
                let device: Option<Retained<AnyObject>> = msg_send![devices, defaultDeviceWithMediaType: AVMediaTypeVideo];
                device.is_some()
            }
        })
    }

    pub fn access() -> Access {
        // SAFETY: AVCaptureDevice's class method.
        unsafe {
            let Ok(devices) = class(c"AVCaptureDevice") else { return Access::Refused };
            let status: isize = msg_send![devices, authorizationStatusForMediaType: AVMediaTypeVideo];
            Access::of(status)
        }
    }

    /// macOS's question, waited for on this thread.
    pub fn ask(wait: Duration) -> Access {
        if access() != Access::NotAsked {
            return access();
        }
        let (tx, rx) = std::sync::mpsc::channel::<bool>();
        let answered = RcBlock::new(move |yes: Bool| {
            let _ = tx.send(yes.as_bool());
        });
        // SAFETY: AVCaptureDevice's class method; the block is copied by it.
        unsafe {
            let Ok(devices) = class(c"AVCaptureDevice") else { return Access::Refused };
            let _: () = msg_send![devices, requestAccessForMediaType: AVMediaTypeVideo, completionHandler: &*answered];
        }
        match rx.recv_timeout(wait) {
            Ok(true) => Access::Allowed,
            Ok(false) => Access::Refused,
            Err(_) => Access::NotAsked,
        }
    }

    type Taken = std::sync::mpsc::Sender<Result<Vec<u8>, String>>;

    struct Ivars {
        taken: Mutex<Option<Taken>>,
    }

    define_class!(
        // AVCapturePhotoCaptureDelegate: the photo arrives on AVFoundation's own queue.
        #[unsafe(super(NSObject))]
        #[name = "VyomaPhotoDelegate"]
        #[ivars = Ivars]
        struct Delegate;

        impl Delegate {
            #[unsafe(method(captureOutput:didFinishProcessingPhoto:error:))]
            fn did_finish(&self, _output: *mut AnyObject, photo: *mut AnyObject, error: *mut AnyObject) {
                let result = if !error.is_null() || photo.is_null() {
                    Err(describe(error))
                } else {
                    // SAFETY: the AVCapturePhoto handed to us; its JPEG copied out before returning.
                    unsafe {
                        let data: Option<Retained<AnyObject>> = msg_send![photo, fileDataRepresentation];
                        match data {
                            Some(data) => {
                                let len: usize = msg_send![&*data, length];
                                let bytes: *const u8 = msg_send![&*data, bytes];
                                if bytes.is_null() || len == 0 { Err("an empty photo".into()) } else { Ok(std::slice::from_raw_parts(bytes, len).to_vec()) }
                            }
                            None => Err("no photo data".into()),
                        }
                    }
                };
                if let Some(tx) = self.ivars().taken.lock().unwrap().take() {
                    let _ = tx.send(result);
                }
            }
        }
    );

    impl Delegate {
        fn new(taken: Taken) -> Retained<Self> {
            let this = Self::alloc().set_ivars(Ivars { taken: Mutex::new(Some(taken)) });
            // SAFETY: NSObject's init on a freshly allocated object.
            unsafe { msg_send![super(this), init] }
        }
    }

    /// Stops the session however [photo] ends.
    struct Running(Retained<AnyObject>);

    impl Drop for Running {
        fn drop(&mut self) {
            // SAFETY: stopping our own session.
            unsafe {
                let _: () = msg_send![&*self.0, stopRunning];
            }
        }
    }

    /// A JPEG from the default camera, [warmup] after it starts. Blocks: run it off the async threads.
    pub fn photo(warmup: Duration, wait: Duration) -> Result<Vec<u8>> {
        objc2::rc::autoreleasepool(|_| photo_in_pool(warmup, wait))
    }

    fn photo_in_pool(warmup: Duration, wait: Duration) -> Result<Vec<u8>> {
        // SAFETY: AVFoundation's documented capture classes and methods, with their declared types.
        unsafe {
            let device: Option<Retained<AnyObject>> = msg_send![class(c"AVCaptureDevice")?, defaultDeviceWithMediaType: AVMediaTypeVideo];
            let device = device.context(super::NO_CAMERA)?;
            let mut error: *mut AnyObject = std::ptr::null_mut();
            let input: Option<Retained<AnyObject>> = msg_send![class(c"AVCaptureDeviceInput")?, deviceInputWithDevice: &*device, error: &mut error];
            let Some(input) = input else {
                bail!("the camera wouldn't start: {}", describe(error));
            };
            let session: Retained<AnyObject> = msg_send![class(c"AVCaptureSession")?, new];
            let preset_ok: Bool = msg_send![&*session, canSetSessionPreset: AVCaptureSessionPresetPhoto];
            if preset_ok.as_bool() {
                let _: () = msg_send![&*session, setSessionPreset: AVCaptureSessionPresetPhoto];
            }
            let ok: Bool = msg_send![&*session, canAddInput: &*input];
            if !ok.as_bool() {
                bail!("the camera is in use by another app");
            }
            let _: () = msg_send![&*session, addInput: &*input];
            let output: Retained<AnyObject> = msg_send![class(c"AVCapturePhotoOutput")?, new];
            let ok: Bool = msg_send![&*session, canAddOutput: &*output];
            if !ok.as_bool() {
                bail!("this Mac's camera can't take photos");
            }
            let _: () = msg_send![&*session, addOutput: &*output];

            let _: () = msg_send![&*session, startRunning];
            let running = Running(session);
            let on: Bool = msg_send![&*running.0, isRunning];
            if !on.as_bool() {
                bail!("the camera is in use by another app");
            }
            std::thread::sleep(warmup);

            // JPEG when the camera offers it (it always has on Macs), else its own default.
            let codecs: Retained<AnyObject> = msg_send![&*output, availablePhotoCodecTypes];
            let jpeg: Bool = msg_send![&*codecs, containsObject: AVVideoCodecTypeJPEG];
            let settings_class = class(c"AVCapturePhotoSettings")?;
            let settings: Retained<AnyObject> = if jpeg.as_bool() {
                let format: Retained<AnyObject> = msg_send![class(c"NSDictionary")?, dictionaryWithObject: AVVideoCodecTypeJPEG, forKey: AVVideoCodecKey];
                msg_send![settings_class, photoSettingsWithFormat: &*format]
            } else {
                msg_send![settings_class, photoSettings]
            };
            let (tx, rx) = std::sync::mpsc::channel();
            let delegate = Delegate::new(tx);
            let _: () = msg_send![&*output, capturePhotoWithSettings: &*settings, delegate: &*delegate];
            let taken = rx.recv_timeout(wait).context("the camera took too long")?;
            drop(running);
            let bytes = taken.map_err(|why| anyhow::anyhow!("couldn't take a photo: {why}"))?;
            if !bytes.starts_with(&[0xff, 0xd8]) {
                bail!("the camera gave something other than a JPEG");
            }
            Ok(bytes)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_closed_lid_is_said_as_such() {
        assert_eq!(unavailable(true), LID_CLOSED);
        assert_eq!(unavailable(false), NO_CAMERA);
        let open = "    | |   \"AppleClamshellState\" = No\n";
        let shut = "    | |   \"AppleClamshellCausesSleep\" = No\n    | |   \"AppleClamshellState\" = Yes\n";
        assert!(!clamshell(open));
        assert!(clamshell(shut));
        assert!(!clamshell(""), "a desktop Mac has no lid");
    }

    #[test]
    fn macos_answers_read_as_access() {
        assert_eq!(Access::of(0), Access::NotAsked);
        assert_eq!(Access::of(1), Access::Refused, "restricted by a profile");
        assert_eq!(Access::of(2), Access::Refused);
        assert_eq!(Access::of(3), Access::Allowed);
    }
}
