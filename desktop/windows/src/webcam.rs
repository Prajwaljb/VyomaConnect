//! The phone's camera as this PC's webcam on Windows 11, "Vyoma Phone Camera" in every app.
//!
//! The camera itself is `vyoma_camera.dll` (camera/): Windows' camera service loads it from a
//! machine-wide registration, so it's set up once as an administrator: `--install-camera` copies it
//! under Program Files (named with this version, so an update can replace one the camera service
//! still holds) and registers its class in HKLM; This PC's Set up and the installer's option run that
//! through Windows' administrator prompt. `--remove-camera` undoes it (the uninstaller asks).
//!
//! The agent adds the camera for its session at start (MFCreateVirtualCamera, looked up at run time
//! since Windows 10 hasn't got it): it's there while the agent runs. When the phone sends
//! `vyoma.webcam {w, h, rotate}` with live H.264, Windows' decoder turns it into pictures, turned
//! upright (`rotate`, and `vyoma.webcam.turn` as the phone turns), written into the camera's shared
//! section; the camera fits them to the size the app asked for, and shows Vyoma's orb while no phone sends.
//! The microphone isn't offered: Windows lets no app add one without a kernel audio driver, so
//! `vyoma.webcam.mic` is answered with `micError` and the phone says so on its Microphone row.
//! Built and tested on CI and under Wine (the decoding and the section; Wine has no camera
//! service); not yet run on Windows 11.

use std::ffi::c_void;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;
use tokio::sync::{mpsc, watch};
use tracing::{info, warn};
use vyoma_camera::frame::{self, MAX_SIDE};
use vyoma_camera::{NAME, Section, clsid_text, now_ms};
use vyoma_core::identity::Identity;
use vyoma_core::link::{Sender, packet};
use windows::Win32::Media::MediaFoundation::{
    IMFAsyncCallback, IMFVirtualCamera, MF_VERSION, MFSTARTUP_FULL, MFStartup, MFVirtualCameraAccess, MFVirtualCameraAccess_CurrentUser, MFVirtualCameraLifetime,
    MFVirtualCameraLifetime_Session, MFVirtualCameraType, MFVirtualCameraType_SoftwareCameraSource,
};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::core::{GUID, HRESULT, Interface, PCWSTR};

use crate::mirror::{Decoder, Units};
use crate::win::{self, Hive};

/// This PC's check, and its fix (FixCheck: Set up).
pub const CHECK: &str = "Phone as webcam";
/// As an administrator: copy the camera under Program Files and register it.
pub const INSTALL_ARG: &str = "--install-camera";
/// As an administrator: take it out again.
pub const REMOVE_ARG: &str = "--remove-camera";
const VERSION: &str = include_str!("../../../VERSION");
/// The DLL as it's installed beside the agent.
const DLL: &str = "vyoma_camera.dll";
/// The phone's words for a missing setup ("one-time setup" is what it looks for).
const SETUP_NEEDED: &str = "The PC needs a one-time setup for the phone camera: on the PC, open This PC in Vyoma's window and press Set up";
const NEEDS_11: &str = "The phone as a webcam needs Windows 11 on the PC";
const MIC_MISSING: &str = "Windows has no way for an app to add a microphone, so this PC can't take the phone's";
/// The picture stops while another app has the phone's camera; only a stream truly stuck runs into this.
const VIDEO_SILENCE: Duration = Duration::from_secs(60);
/// The camera's section is looked for again this often while it's missing (no app has the camera open yet).
const SECTION_RETRY: Duration = Duration::from_secs(1);

/// Where the camera stands on this PC.
#[derive(Debug, PartialEq)]
pub enum Status {
    NeedsWindows11,
    NotSetUp,
    /// Set up by an older Vyoma: works, but this version's camera isn't in place.
    Older,
    Ready,
}

fn version() -> &'static str {
    VERSION.trim()
}

fn class_key() -> String {
    format!(r"SOFTWARE\Classes\CLSID\{}", clsid_text())
}

/// MFCreateVirtualCamera, if this Windows has it (Windows 11).
type CreateCamera = unsafe extern "system" fn(MFVirtualCameraType, MFVirtualCameraLifetime, MFVirtualCameraAccess, PCWSTR, PCWSTR, *const GUID, u32, *mut *mut c_void) -> HRESULT;

fn create_camera_fn() -> Option<CreateCamera> {
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    // SAFETY: a system DLL by name and a function looked up in it; the DLL stays loaded.
    unsafe {
        let lib = LoadLibraryW(win::wide("mfsensorgroup.dll").as_ptr());
        if lib.is_null() {
            return None;
        }
        GetProcAddress(lib, c"MFCreateVirtualCamera".as_ptr().cast()).map(|f| std::mem::transmute::<_, CreateCamera>(f))
    }
}

/// The registered camera's file, if it's there.
fn registered() -> Option<PathBuf> {
    let path = win::value(Hive::Machine, &format!(r"{}\InprocServer32", class_key()), None)?;
    let path = PathBuf::from(path);
    path.is_file().then_some(path)
}

/// The file name this version installs ("vyoma_camera-0.12.0.dll").
fn installed_name() -> String {
    format!("vyoma_camera-{}.dll", version())
}

pub fn status() -> Status {
    if create_camera_fn().is_none() {
        return Status::NeedsWindows11;
    }
    match registered() {
        None => Status::NotSetUp,
        Some(p) if p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(&installed_name())) => Status::Ready,
        Some(_) => Status::Older,
    }
}

/// This PC's check.
pub fn check() -> Value {
    let (state, detail, action) = match status() {
        Status::NeedsWindows11 => ("warn", "The phone as a webcam needs Windows 11", ""),
        Status::NotSetUp => ("warn", "Apps can use the phone's camera once it's set up. Set up asks Windows for an administrator once", "Set up"),
        Status::Older => ("warn", "This version of Vyoma brings a newer phone camera. Set up again (Windows asks for an administrator)", "Set up"),
        Status::Ready => ("ok", "Apps see the phone as Vyoma Phone Camera", ""),
    };
    json!({ "name": CHECK, "state": state, "detail": detail, "action": action })
}

/// Set up: this program as an administrator with [INSTALL_ARG] (Windows asks), then the camera added.
pub fn set_up() -> Result<()> {
    win::as_admin(INSTALL_ARG, "the phone camera wasn't set up")?;
    add();
    Ok(())
}

fn program_files() -> PathBuf {
    std::env::var_os("ProgramW6432").or_else(|| std::env::var_os("ProgramFiles")).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Program Files"))
}

/// `--install-camera` (as an administrator): the DLL beside this program copied under Program
/// Files and its class registered for the camera service. Exit status 0 when done.
pub fn install() -> Result<()> {
    let here = std::env::current_exe()?.with_file_name(DLL);
    anyhow::ensure!(here.is_file(), "{DLL} isn't beside the program; reinstall Vyoma");
    let dir = program_files().join("Vyoma");
    std::fs::create_dir_all(&dir).with_context(|| format!("couldn't make {}", dir.display()))?;
    let target = dir.join(installed_name());
    if let Err(e) = std::fs::copy(&here, &target) {
        // The camera service holds this very version: it's in place already.
        anyhow::ensure!(target.is_file(), "couldn't copy the camera: {e}");
    }
    let key = class_key();
    win::set_value(Hive::Machine, &key, None, NAME)?;
    win::set_value(Hive::Machine, &format!(r"{key}\InprocServer32"), None, &target.to_string_lossy())?;
    win::set_value(Hive::Machine, &format!(r"{key}\InprocServer32"), Some("ThreadingModel"), "Both")?;
    tidy(&dir, Some(&target));
    Ok(())
}

/// `--remove-camera` (as an administrator): the class and the files gone (a file the camera
/// service still holds goes at the next restart).
pub fn remove() -> Result<()> {
    win::delete_key(Hive::Machine, &class_key());
    let dir = program_files().join("Vyoma");
    tidy(&dir, None);
    let _ = std::fs::remove_dir(&dir);
    Ok(())
}

/// Every camera file in [dir] but [keep] removed, or marked to go at the next restart.
fn tidy(dir: &Path, keep: Option<&Path>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        let name = e.file_name().to_string_lossy().to_lowercase();
        if !name.starts_with("vyoma_camera") || keep.is_some_and(|k| k == path) {
            continue;
        }
        if std::fs::remove_file(&path).is_err() {
            use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_DELAY_UNTIL_REBOOT, MoveFileExW};
            // SAFETY: a NUL-terminated path; no new name: delete at restart.
            unsafe { MoveFileExW(win::wide(&path.to_string_lossy()).as_ptr(), std::ptr::null(), MOVEFILE_DELAY_UNTIL_REBOOT) };
        }
    }
}

static ADDED: AtomicBool = AtomicBool::new(false);

/// Adds the camera for this session if it's set up (at start, and after Set up); it stays while
/// the agent runs.
pub fn add() {
    if registered().is_none() || ADDED.swap(true, Ordering::SeqCst) {
        return;
    }
    let spawned = std::thread::Builder::new().name("vyoma-camera".into()).spawn(|| match create() {
        Ok(camera) => {
            info!("added {NAME} for this session");
            // Held for as long as the agent runs: the camera goes with it.
            loop {
                std::thread::park();
                let _ = &camera;
            }
        }
        Err(e) => {
            warn!("couldn't add {NAME}: {e:#}");
            ADDED.store(false, Ordering::SeqCst);
        }
    });
    if spawned.is_err() {
        ADDED.store(false, Ordering::SeqCst);
    }
}

fn create() -> Result<IMFVirtualCamera> {
    let make = create_camera_fn().context("this Windows has no virtual cameras (Windows 11 has)")?;
    let (name, source) = (win::wide(NAME), win::wide(&clsid_text()));
    // SAFETY: COM and MF started on this thread; NUL-terminated strings; the camera comes back owned.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        MFStartup(MF_VERSION, MFSTARTUP_FULL)?;
        let mut raw = std::ptr::null_mut();
        make(
            MFVirtualCameraType_SoftwareCameraSource,
            MFVirtualCameraLifetime_Session,
            MFVirtualCameraAccess_CurrentUser,
            PCWSTR(name.as_ptr()),
            PCWSTR(source.as_ptr()),
            std::ptr::null(),
            0,
            &mut raw,
        )
        .ok()
        .context("Windows didn't add the camera")?;
        let camera = IMFVirtualCamera::from_raw(raw);
        camera.Start(None::<&IMFAsyncCallback>).context("the camera didn't start")?;
        Ok(camera)
    }
}

async fn state(sender: &Sender, body: Value) {
    let _ = sender.send(&packet("vyoma.webcam.state", body)).await;
}

/// The phone's camera until it hangs up. [turn] carries its latest `rotate`.
pub async fn camera(sender: &Sender, me: &Identity, pinned: &[u8], phone: IpAddr, packet: &Value, mut turn: watch::Receiver<u32>, name: &str) -> Result<()> {
    match tokio::task::spawn_blocking(status).await? {
        Status::NeedsWindows11 => {
            state(sender, json!({ "error": NEEDS_11 })).await;
            bail!("no virtual cameras before Windows 11");
        }
        Status::NotSetUp => {
            state(sender, json!({ "error": SETUP_NEEDED })).await;
            bail!("{NAME} isn't set up");
        }
        Status::Older | Status::Ready => {}
    }
    add();
    // The engine's watch starts at the packet's `rotate` and follows `vyoma.webcam.turn`.
    let rotate = Arc::new(AtomicU32::new(*turn.borrow_and_update()));
    let mut input = vyoma_core::payload::fetch_stream(me, pinned, phone, packet).await?;
    state(sender, json!({ "camera": true, "device": NAME })).await;
    info!("{name}: phone camera live as {NAME}");
    let (tx, rx) = mpsc::channel::<Vec<u8>>(64);
    let feeding = {
        let rotate = rotate.clone();
        tokio::task::spawn_blocking(move || feed(rx, &rotate))
    };
    let mut buf = vec![0u8; 64 * 1024];
    let result = loop {
        tokio::select! {
            n = tokio::time::timeout(VIDEO_SILENCE, input.read(&mut buf)) => match n {
                Err(_) => break Err(anyhow::anyhow!("the phone's camera went quiet")),
                Ok(Ok(0)) => break Ok(()),
                Ok(Ok(n)) => {
                    if tx.send(buf[..n].to_vec()).await.is_err() {
                        break Err(anyhow::anyhow!("the decoder stopped"));
                    }
                }
                Ok(Err(e)) => break Err(anyhow::Error::from(e).context("the phone's stream")),
            },
            changed = turn.changed() => {
                if changed.is_err() { break Ok(()) }
                rotate.store(*turn.borrow_and_update(), Ordering::Relaxed);
            }
        }
    };
    drop(tx);
    let _ = feeding.await;
    info!("{name}: phone camera stopped");
    result
}

/// The decoder's side: the phone's H.264 in, upright pictures into the camera's section.
fn feed(mut rx: mpsc::Receiver<Vec<u8>>, rotate: &AtomicU32) {
    // SAFETY: this thread joins the multithreaded apartment for the decoder.
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let mut decoder = match Decoder::new() {
        Ok(d) => d,
        Err(e) => return warn!("phone camera: no decoder: {e:#}"),
    };
    let mut out = Writer::default();
    let (mut buf, mut units) = (Vec::with_capacity(512 * 1024), Units::default());
    while let Some(chunk) = rx.blocking_recv() {
        buf.extend_from_slice(&chunk);
        let (nals, used) = vyoma_core::mirror::split_nals(&buf);
        let whole: Vec<Vec<u8>> = nals.into_iter().filter_map(|nal| units.push(nal)).collect();
        buf.drain(..used);
        for unit in whole {
            let turn = rotate.load(Ordering::Relaxed);
            if let Err(e) = decoder.decode_nv12(&unit, |v| out.picture(v, turn)) {
                tracing::debug!("a picture didn't decode: {e:#}");
            }
        }
    }
    out.stop();
}

/// Pictures into the camera's section, once an app has opened the camera (it makes the section).
#[derive(Default)]
pub struct Writer {
    section: Option<Section>,
    tried: Option<Instant>,
}

impl Writer {
    pub fn picture(&mut self, v: &frame::View, turn: u32) {
        if self.section.is_none() && self.tried.is_none_or(|t| t.elapsed() > SECTION_RETRY) {
            self.tried = Some(Instant::now());
            self.section = Section::open();
        }
        let Some(section) = &self.section else { return };
        let (pic, w, h) = frame::upright(v, turn);
        let (pic, w, h) = within_max(pic, w, h);
        section.shared().set_live(true);
        section.shared().write(w, h, &pic, now_ms());
    }

    pub fn stop(&self) {
        if let Some(s) = &self.section {
            s.shared().set_live(false);
        }
    }
}

/// A picture larger than the section takes, shrunk to fit (shape kept).
fn within_max(pic: Vec<u8>, w: usize, h: usize) -> (Vec<u8>, usize, usize) {
    if w <= MAX_SIDE && h <= MAX_SIDE {
        return (pic, w, h);
    }
    let scale = (MAX_SIDE as f64 / w as f64).min(MAX_SIDE as f64 / h as f64);
    let (nw, nh) = (((w as f64 * scale) as usize) & !1, ((h as f64 * scale) as usize) & !1);
    let mut out = vec![0u8; nw * nh * 3 / 2];
    frame::fit(&pic, w, h, nw, nh, &mut out);
    (out, nw, nh)
}

/// The phone's microphone: not on Windows; the phone shows why on its Microphone row.
pub async fn mic(sender: &Sender) {
    state(sender, json!({ "mic": false, "micError": MIC_MISSING })).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_carry_the_version() {
        assert_eq!(installed_name(), format!("vyoma_camera-{}.dll", VERSION.trim()));
        assert_eq!(class_key(), r"SOFTWARE\Classes\CLSID\{5F0C9A1E-3B7D-4E62-A8F1-2D6C4B9E7A13}");
    }

    #[test]
    fn big_pictures_shrink_to_fit() {
        let (pic, w, h) = within_max(vec![200; 3840 * 2160 * 3 / 2], 3840, 2160);
        assert_eq!((w, h), (1920, 1080));
        assert_eq!(pic.len(), 1920 * 1080 * 3 / 2);
        let (_, w, h) = within_max(vec![0; 1280 * 720 * 3 / 2], 1280, 720);
        assert_eq!((w, h), (1280, 720));
    }

    /// The phone's H.264 (here this PC's own encoder's) through the decoder, turned upright, into
    /// the section the camera made: the camera then reads a 240×320 picture.
    #[test]
    fn the_phones_video_reaches_the_camera_upright() {
        // SAFETY: a test-only name, set before either side opens the section.
        unsafe { std::env::set_var("VYOMA_CAMERA_SECTION", "Local\\VyomaWebcamFeed") };
        let Some(video) = crate::mirror::tests::some_video(90) else {
            return println!("no encoder here: skipped");
        };
        let camera = Section::create().expect("the camera's section");
        let (tx, rx) = mpsc::channel(64);
        let rotate = AtomicU32::new(90);
        let feeding = std::thread::spawn(move || feed(rx, &rotate));
        for chunk in video.chunks(4096) {
            tx.blocking_send(chunk.to_vec()).unwrap();
        }
        // The last picture's end: a start code after it.
        tx.blocking_send(vec![0, 0, 0, 1]).unwrap();
        drop(tx);
        feeding.join().unwrap();
        // The feed said it stopped; the last picture is still there (and fresh).
        camera.shared().set_live(true);
        let mut out = Vec::new();
        assert_eq!(camera.shared().read(&mut out, now_ms()), Some((240, 320)), "320×240 turned a quarter");
        assert_eq!(out.len(), 240 * 320 * 3 / 2);
    }

    /// Wine has no virtual cameras: the check says what's needed, and the phone is told so.
    #[test]
    fn the_check_reads_this_windows() {
        let c = check();
        assert_eq!(c["name"], CHECK);
        if create_camera_fn().is_none() {
            assert_eq!(status(), Status::NeedsWindows11);
            assert_eq!(c["action"], "");
        }
    }
}
