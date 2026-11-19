//! The engine's [Platform] on macOS: what each phone request does on a Mac. The engine routes,
//! checks the switches, keeps Activity and ends what a phone started; this module only does the
//! work, through the modules that already know how (screen, camera, mirror, vyoma, brightness).
//! Built and tested on CI only; not yet run on a real Mac.

use std::net::IpAddr;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use vyoma_core::link::packet;
use vyoma_engine::{Cx, Feature, Platform, Sampler};

use crate::{away, brightness, camera, input, mac, mirror, screen, vyoma};

/// A paste's text stays on the clipboard this long before what was there comes back.
const PASTE_RESTORE: Duration = Duration::from_millis(1200);
/// Mission Control left open this long has probably been closed some other way.
const MISSION_STALE: Duration = Duration::from_secs(20);
/// The flags macOS gives the arrow keys (kCGEventFlagMaskSecondaryFn | NumericPad): its own
/// shortcuts on them (Mission Control, Spaces) only answer a press that carries them.
const ARROW: u64 = 0x80_0000 | 0x20_0000;
/// Arrow key codes.
const UP: u16 = 126;
const LEFT: u16 = 123;
const RIGHT: u16 = 124;

pub struct Mac {
    /// The desktop picture's file, which AppKit tells only the main thread: the agent's loop keeps
    /// this current ([Mac::set_wallpaper]).
    wallpaper: Mutex<Option<PathBuf>>,
    /// The built-in display's brightness worker.
    brightness: mpsc::UnboundedSender<(String, Value)>,
    pub away: Arc<away::Away>,
    /// When Mission Control was opened from the phone (macOS doesn't say whether it's showing).
    mission: Mutex<Option<Instant>>,
}

impl Mac {
    pub fn new(brightness: mpsc::UnboundedSender<(String, Value)>, away: Arc<away::Away>) -> Self {
        Self { wallpaper: Mutex::new(vyoma::wallpaper_path()), brightness, away, mission: Mutex::new(None) }
    }

    /// From the main thread, every few seconds and before a phone's wallpaper request.
    pub fn set_wallpaper(&self, path: Option<PathBuf>) {
        *self.wallpaper.lock().unwrap() = path;
    }

    /// What a Mac offers, before there is one to ask (the identity is made first).
    pub fn offers(feature: Feature) -> bool {
        match feature {
            Feature::Stats | Feature::Specs | Feature::Screenshot | Feature::Camera | Feature::Files | Feature::Screen => true,
            Feature::Power | Feature::Wake | Feature::Wallpaper | Feature::Desktop | Feature::Mirror | Feature::Paste | Feature::Away => true,
            // Only with a built-in display to dim; macOS has no per-app volumes.
            Feature::Controls => brightness::available(),
            // Not on a Mac yet: no browser's media handoff, virtual gamepad, USB accessory, phone
            // camera as a webcam or Bluetooth touchpad.
            Feature::Handoff | Feature::Gamepad | Feature::Accessory | Feature::Webcam | Feature::Touchpad => false,
        }
    }
}

impl Platform for Mac {
    fn supports(&self, feature: Feature) -> bool {
        Mac::offers(feature)
    }

    fn install_file_hooks(&self) {
        vyoma::install_file_hooks();
    }

    fn this_computer(&self) -> &'static str {
        "this Mac"
    }

    fn wake_text(&self) -> &'static str {
        "Woke this Mac's screen"
    }

    fn stats(&self) -> Result<Box<dyn Sampler>> {
        Ok(Box::new(vyoma::Stats::new()?))
    }

    async fn on_ac(&self) -> Option<bool> {
        vyoma::on_ac().await
    }

    async fn camera_state(&self) -> (bool, bool) {
        camera::state().await
    }

    async fn specs(&self) -> Value {
        vyoma::specs().await
    }

    async fn screenshot(&self, cx: &Cx) -> Result<()> {
        screen::screenshot(&cx.sender, &cx.me, &cx.pinned, &cx.name).await
    }

    async fn camera_photo(&self, cx: &Cx) -> Result<()> {
        camera::take(&cx.sender, &cx.me, &cx.pinned, &cx.name).await
    }

    fn screen_fps(&self, asked: Option<u64>) -> u32 {
        screen::fps_for(asked)
    }

    async fn screen(&self, cx: &Cx, fps: u32, inputs: mpsc::UnboundedReceiver<Value>) -> Result<()> {
        screen::stream(&cx.sender, &cx.me, &cx.pinned, &cx.name, fps, inputs).await
    }

    async fn mirror(&self, cx: &Cx, phone: IpAddr, packet: &Value) -> Result<()> {
        mirror::show(&cx.sender, &cx.me, &cx.pinned, phone, packet, &cx.name).await
    }

    async fn mirror_sound(&self, cx: &Cx, phone: IpAddr, packet: &Value) -> Result<()> {
        mirror::sound::play(&cx.me, &cx.pinned, phone, packet, &cx.name).await
    }

    async fn mirror_copy(&self, text: &str) {
        let _ = mac::set_clipboard(text).await;
    }

    async fn mirror_volume(&self, up: bool) {
        mirror::volume(up).await
    }

    async fn paste(&self, text: &str) -> Result<()> {
        paste(text).await
    }

    async fn desktop_step(&self, up: bool) -> Result<&'static str> {
        let (press, view) = {
            let open = self.mission.lock().unwrap().is_some_and(|t| t.elapsed() < MISSION_STALE);
            match (up, open) {
                (true, false) => (true, "overview"),
                (true, true) => (false, "overview"),
                (false, true) => (true, "windows"),
                (false, false) => (false, "windows"),
            }
        };
        if !press {
            return Ok(view);
        }
        // Control+Up opens Mission Control, and again closes it.
        if !press_key(UP, input::CONTROL | ARROW) {
            bail!(input::ALLOW_SHORT);
        }
        *self.mission.lock().unwrap() = (view == "overview").then(Instant::now);
        Ok(view)
    }

    async fn desktop_switch(&self, right: bool) -> Result<()> {
        // Control+Left / Right: the Space beside this one.
        if !press_key(if right { RIGHT } else { LEFT }, input::CONTROL | ARROW) {
            bail!(input::ALLOW_SHORT);
        }
        Ok(())
    }

    async fn wallpaper_colors(&self) -> Result<Vec<[u8; 3]>> {
        let path = self.wallpaper.lock().unwrap().clone();
        match path {
            Some(path) => vyoma::colors(&path).await,
            None => bail!("couldn't find this Mac's wallpaper"),
        }
    }

    fn wallpaper_signature(&self) -> Option<String> {
        // The file and when it last changed: a new picture, or the same file rewritten.
        let path = self.wallpaper.lock().unwrap().clone();
        let changed = path.as_ref().and_then(|p| std::fs::metadata(p).ok()?.modified().ok());
        Some(format!("{path:?} {changed:?}"))
    }

    fn pc_controls(&self, cx: &Cx) -> Option<mpsc::UnboundedSender<Value>> {
        let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
        let (brightness, id, sender) = (self.brightness.clone(), cx.id.clone(), cx.sender.clone());
        tokio::spawn(async move {
            while let Some(p) = rx.recv().await {
                match p["type"].as_str().unwrap_or_default() {
                    "vyoma.pc.power" => {
                        let action = p["body"]["action"].as_str().unwrap_or_default().to_owned();
                        tracing::info!("{id}: asked this Mac to {action}");
                        let mut out = json!({ "action": action });
                        if let Err(e) = vyoma::power(&action).await {
                            tracing::warn!("{action}: {e:#}");
                            out["error"] = json!(format!("{e:#}"));
                        }
                        let _ = sender.send(&packet("vyoma.pc.powered", out)).await;
                    }
                    "vyoma.pc.wake" => {
                        if let Err(e) = vyoma::wake().await {
                            tracing::warn!("couldn't wake the screen: {e:#}");
                        }
                    }
                    _ => {
                        let _ = brightness.send((id.clone(), p));
                    }
                }
            }
        });
        Some(tx)
    }

    fn away(&self, phone: &str, body: &Value) {
        self.away.configure(phone, body);
    }
}

impl Sampler for vyoma::Stats {
    fn sample(&mut self) -> Pin<Box<dyn Future<Output = Value> + Send + '_>> {
        Box::pin(vyoma::Stats::sample(self))
    }
}

/// Presses a key the agent itself sends (the Spaces gestures); false while macOS doesn't allow it.
fn press_key(code: u16, flags: u64) -> bool {
    #[cfg(target_os = "macos")]
    {
        input::post_key(code, flags)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (code, flags);
        true
    }
}

/// Puts [text] on the clipboard for the phone's paste, and what was there before back a moment
/// later, unless something newer was copied meanwhile.
async fn paste(text: &str) -> Result<()> {
    static GENERATION: AtomicU64 = AtomicU64::new(0);
    let before = mac::clipboard().await;
    mac::set_clipboard(text).await?;
    let mine = GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
    let text = text.to_owned();
    tokio::spawn(async move {
        tokio::time::sleep(PASTE_RESTORE).await;
        if GENERATION.load(Ordering::Relaxed) == mine
            && mac::clipboard().await.as_deref() == Some(text.as_str())
            && let Some(before) = before
        {
            let _ = mac::set_clipboard(&before).await;
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offers_what_it_serves() {
        let (incoming, _) = vyoma_engine::platform::capabilities_for(Mac::offers);
        for kind in ["vyoma.stats.request", "vyoma.screenshot.request", "vyoma.camera.request", "vyoma.screen.request", "vyoma.mirror", "vyoma.paste", "vyoma.desktop.step", "vyoma.away", "vyoma.pc.power", "vyoma.pc.wake", "vyoma.wallpaper.request", "vyoma.files.list", "vyoma.xfer.begin"] {
            assert!(incoming.contains(&kind), "{kind}");
        }
        for kind in ["vyoma.webcam", "vyoma.gamepad.state", "vyoma.handoff.request", "vyoma.pointer.request", "vyoma.usb.accessory"] {
            assert!(!incoming.contains(&kind), "{kind} isn't on a Mac yet");
        }
    }
}
