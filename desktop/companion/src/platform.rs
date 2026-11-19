//! The engine's [Platform] on Linux: each call goes to the module that already picks among KDE,
//! GNOME (through Vyoma's extension), X11 desktops and the rest (portals, wl-clipboard, layer-shell).

use std::net::IpAddr;
use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;
use tokio::sync::{mpsc, watch};
use vyoma_engine::{Cx, Feature, GamepadInput, Handoff, Platform, Sampler, Started};

use crate::{accessory, away, bluetooth, camera, desktop, files, gamepad, media, mirror, paste, pc, pointer, screen, screenshot, specs, stats, wallpaper, webcam};

pub struct Linux {
    /// Tracks what's playing for handoff and walk-away pauses; None without a session bus.
    pub media: Option<Arc<media::Media>>,
    /// Walk-away rules each phone asked for.
    pub away: Arc<away::Away>,
}

impl Sampler for stats::Stats {
    fn sample(&mut self) -> std::pin::Pin<Box<dyn std::future::Future<Output = Value> + Send + '_>> {
        Box::pin(async move { stats::Stats::sample(self) })
    }
}

impl Platform for Linux {
    fn supports(&self, _: Feature) -> bool {
        true
    }

    fn install_file_hooks(&self) {
        files::install();
    }

    fn stats(&self) -> Result<Box<dyn Sampler>> {
        Ok(Box::new(stats::Stats::new()?))
    }

    async fn on_ac(&self) -> Option<bool> {
        on_ac()
    }

    async fn camera_state(&self) -> (bool, bool) {
        let available = camera::available();
        (available, !available && camera::behind_lid())
    }

    fn handoff_offer(&self) -> Option<String> {
        self.media.as_ref().and_then(|m| m.offer())
    }

    async fn handoff(&self) -> Option<Handoff> {
        let h = self.media.as_ref()?.handoff().await?;
        Some(Handoff { url: h.url, title: h.title, position_ms: h.position_ms, exact: h.exact, player: h.bus })
    }

    async fn pause_handoff(&self, h: &Handoff) {
        if let Some(m) = &self.media {
            m.pause_player(&h.player).await;
        }
    }

    async fn specs(&self) -> Value {
        tokio::task::spawn_blocking(specs::read).await.unwrap_or_else(|e| serde_json::json!({ "error": e.to_string() }))
    }

    async fn screenshot(&self, cx: &Cx) -> Result<()> {
        screenshot::capture_and_send(&cx.sender, &cx.me, &cx.pinned, &cx.name).await
    }

    async fn camera_photo(&self, cx: &Cx) -> Result<()> {
        camera::capture_and_send(&cx.sender, &cx.me, &cx.pinned, &cx.name).await
    }

    fn screen_fps(&self, asked: Option<u64>) -> u32 {
        screen::fps_for(asked)
    }

    async fn screen(&self, cx: &Cx, fps: u32, inputs: mpsc::UnboundedReceiver<Value>) -> Result<()> {
        screen::stream(&cx.sender, &cx.me, &cx.pinned, &cx.name, fps, inputs).await
    }

    async fn webcam(&self, cx: &Cx, phone: IpAddr, packet: &Value, turn: watch::Receiver<u32>) -> Result<()> {
        webcam::camera(&cx.sender, &cx.me, &cx.pinned, phone, packet, turn, &cx.name).await
    }

    async fn webcam_mic(&self, cx: &Cx, phone: IpAddr, packet: &Value) -> Result<()> {
        webcam::mic(&cx.sender, &cx.me, &cx.pinned, phone, packet, &cx.name).await
    }

    async fn mirror(&self, cx: &Cx, phone: IpAddr, packet: &Value) -> Result<()> {
        mirror::show(&cx.sender, &cx.me, &cx.pinned, phone, packet, &cx.name).await
    }

    async fn mirror_sound(&self, cx: &Cx, phone: IpAddr, packet: &Value) -> Result<()> {
        mirror::sound(&cx.me, &cx.pinned, phone, packet, &cx.name).await
    }

    async fn mirror_copy(&self, text: &str) {
        mirror::copy(text).await
    }

    async fn mirror_volume(&self, up: bool) {
        mirror::volume(up).await
    }

    async fn paste(&self, text: &str) -> Result<()> {
        paste::put(text).await
    }

    async fn desktop_step(&self, up: bool) -> Result<&'static str> {
        desktop::step(up).await
    }

    async fn desktop_switch(&self, right: bool) -> Result<()> {
        desktop::switch(right).await
    }

    async fn wallpaper_colors(&self) -> Result<Vec<[u8; 3]>> {
        wallpaper::colors().await
    }

    fn wallpaper_signature(&self) -> Option<String> {
        // Always something, so the watch runs: the picture may appear later.
        Some(format!("{:?}", wallpaper::signature()))
    }

    fn pc_controls(&self, cx: &Cx) -> Option<mpsc::UnboundedSender<Value>> {
        Some(pc::worker(cx.sender.clone(), cx.name.clone()))
    }

    async fn gamepad(&self, cx: &Cx, input: mpsc::Receiver<GamepadInput>) {
        gamepad::run(input, cx.sender.clone(), cx.name.clone()).await
    }

    fn usb_accessory(&self, reswitch: bool) -> Result<()> {
        accessory::start(reswitch)
    }

    fn touchpad(&self, cx: &Cx) -> Option<Started> {
        let (tx, rx) = mpsc::channel(256);
        let task = tokio::spawn(pointer::serve(cx.sender.clone(), cx.name.clone(), tx.clone(), rx));
        Some(Started { task, close: Box::new(move || tx.try_send(pointer::Frame::Close).is_ok()) })
    }

    async fn bluetooth_address(&self) -> Option<String> {
        bluetooth::address().await
    }

    fn away(&self, phone: &str, body: &Value) {
        self.away.configure(phone, body);
    }
}

/// Whether the PC runs from a charger (mains or a USB-C power source); None with no such supply (a desktop).
fn on_ac() -> Option<bool> {
    let mut seen = false;
    for entry in std::fs::read_dir("/sys/class/power_supply").ok()?.flatten() {
        let read = |f: &str| std::fs::read_to_string(entry.path().join(f)).map(|s| s.trim().to_owned()).unwrap_or_default();
        if !matches!(read("type").as_str(), "Mains" | "USB") {
            continue;
        }
        seen = true;
        if read("online") == "1" {
            return Some(true);
        }
    }
    seen.then_some(false)
}

