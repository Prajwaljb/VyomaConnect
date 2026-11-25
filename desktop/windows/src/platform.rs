//! The engine's [Platform] on Windows: what each phone request does on this PC. The engine routes,
//! checks the switches, keeps Activity and ends what a phone started; this module only does the
//! work. So far: the Home stats, About this PC, screenshots, the live screen, the PC's files
//! (browse, open, send both ways), brightness and app volumes ([pc]), sleep / restart / shut down
//! / sign out, waking the screen, Task View and the virtual desktops, paste, the wallpaper's
//! colours, walk-away rules, the phone's screen in a window ([crate::mirror]), a photo with the
//! camera ([crate::camera]), the phone as a game controller ([crate::gamepad]), handoff of a web
//! video ([crate::handoff]) and the touchpad over Bluetooth ([crate::bluetooth]).
//! Built and tested on CI and under Wine only; not yet run on a real Windows PC.

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::{Value, json};
use sysinfo::{CpuRefreshKind, Disks, MemoryRefreshKind, RefreshKind, System};
use tokio::sync::mpsc;
use vyoma_core::away::Rules;
use vyoma_core::device::Command;
use vyoma_core::files;
use vyoma_core::link::{Sender, packet};
use vyoma_engine::{Cx, Feature, Handoff, Platform, Sampler, Started};

use crate::{input, pc, screen, system, tray, wallpaper, win};

/// A paste's text stays on the clipboard this long before what was there comes back.
const PASTE_RESTORE: Duration = Duration::from_millis(1200);
/// Task View left open this long has probably been closed some other way.
const TASK_VIEW_STALE: Duration = Duration::from_secs(20);

#[derive(Default)]
pub struct Windows {
    /// When Task View was opened from the phone (Windows doesn't say whether it's showing).
    task_view: Mutex<Option<Instant>>,
    /// Each phone's walk-away rules (carried out by [crate::away::Hands]).
    pub away: Arc<Rules>,
    /// The web video the phones are offered to continue.
    handoff: Arc<crate::handoff::Offer>,
}

impl Windows {
    /// What this PC offers, before there is one to ask (the identity is made first).
    pub fn offers(feature: Feature) -> bool {
        match feature {
            Feature::Stats | Feature::Specs | Feature::Screenshot | Feature::Screen | Feature::Files | Feature::Power | Feature::Wake | Feature::Desktop | Feature::Paste => true,
            Feature::Controls | Feature::Wallpaper | Feature::Away | Feature::Mirror | Feature::Camera => true,
            // Offered without the driver too: the phone then says how to get it.
            Feature::Gamepad => true,
            // Offered without a radio too: the phone then says Bluetooth is off or missing.
            Feature::Handoff | Feature::Touchpad => true,
            // Offered before their one-time setup too (UsbDk; the camera registered as an
            // administrator): the phone then says what's needed.
            Feature::Accessory | Feature::Webcam => true,
        }
    }
}

impl Platform for Windows {
    fn supports(&self, feature: Feature) -> bool {
        Windows::offers(feature)
    }

    fn install_file_hooks(&self) {
        files::set_hooks(Box::new(WindowsFiles));
    }

    fn this_computer(&self) -> &'static str {
        "this PC"
    }

    fn wake_text(&self) -> &'static str {
        "Woke this PC's screen"
    }

    fn stats(&self) -> Result<Box<dyn Sampler>> {
        Ok(Box::new(Stats::new()?))
    }

    async fn on_ac(&self) -> Option<bool> {
        win::on_ac()
    }

    async fn specs(&self) -> Value {
        tokio::task::spawn_blocking(specs).await.unwrap_or(Value::Null)
    }

    async fn screenshot(&self, cx: &Cx) -> Result<()> {
        screen::screenshot(&cx.sender, &cx.me, &cx.pinned, &cx.name).await
    }

    async fn gamepad(&self, cx: &Cx, input: mpsc::Receiver<vyoma_engine::GamepadInput>) {
        crate::gamepad::run(input, cx.sender.clone(), cx.name.clone()).await
    }

    fn handoff_offer(&self) -> Option<String> {
        self.handoff.title()
    }

    async fn handoff(&self) -> Option<Handoff> {
        tokio::task::spawn_blocking(crate::handoff::find).await.ok().flatten().map(Into::into)
    }

    async fn pause_handoff(&self, h: &Handoff) {
        let app = h.player.clone();
        let _ = tokio::task::spawn_blocking(move || crate::media::pause_app(&app)).await;
    }

    fn touchpad(&self, cx: &Cx) -> Option<Started> {
        let (tx, rx) = mpsc::channel(256);
        let task = tokio::spawn(crate::bluetooth::touchpad(cx.sender.clone(), cx.name.clone(), tx.clone(), rx));
        Some(Started { task, close: Box::new(move || tx.try_send(vyoma_core::pointer::Frame::Close).is_ok()) })
    }

    async fn bluetooth_address(&self) -> Option<String> {
        tokio::task::spawn_blocking(crate::bluetooth::address).await.ok().flatten()
    }

    async fn camera_state(&self) -> (bool, bool) {
        crate::camera::state().await
    }

    async fn camera_photo(&self, cx: &Cx) -> Result<()> {
        crate::camera::take(&cx.sender, &cx.me, &cx.pinned, &cx.name).await
    }

    async fn webcam(&self, cx: &Cx, phone: IpAddr, packet: &Value, turn: tokio::sync::watch::Receiver<u32>) -> Result<()> {
        crate::webcam::camera(&cx.sender, &cx.me, &cx.pinned, phone, packet, turn, &cx.name).await
    }

    async fn webcam_mic(&self, cx: &Cx, _phone: IpAddr, _packet: &Value) -> Result<()> {
        crate::webcam::mic(&cx.sender).await;
        Ok(())
    }

    fn usb_accessory(&self, reswitch: bool) -> Result<()> {
        crate::accessory::start(reswitch)
    }

    async fn mirror(&self, cx: &Cx, phone: IpAddr, packet: &Value) -> Result<()> {
        crate::mirror::show(&cx.sender, &cx.me, &cx.pinned, phone, packet, &cx.name).await
    }

    async fn mirror_sound(&self, cx: &Cx, phone: IpAddr, packet: &Value) -> Result<()> {
        crate::mirror::sound::play(&cx.me, &cx.pinned, phone, packet, &cx.name).await
    }

    async fn mirror_copy(&self, text: &str) {
        let text = text.to_owned();
        if let Ok(Err(e)) = tokio::task::spawn_blocking(move || win::set_clipboard(&text)).await {
            tracing::info!("couldn't put the phone's copy on the clipboard: {e:#}");
        }
    }

    async fn mirror_volume(&self, up: bool) {
        let _ = tokio::task::spawn_blocking(move || crate::mirror::volume(up)).await;
    }

    fn screen_fps(&self, asked: Option<u64>) -> u32 {
        crate::cast::fps_for(asked)
    }

    async fn screen(&self, cx: &Cx, fps: u32, inputs: mpsc::UnboundedReceiver<Value>) -> Result<()> {
        crate::cast::stream(&cx.sender, &cx.me, &cx.pinned, &cx.name, fps, inputs).await
    }

    async fn paste(&self, text: &str) -> Result<()> {
        paste(text).await
    }

    async fn desktop_step(&self, up: bool) -> Result<&'static str> {
        let open = self.task_view.lock().unwrap().is_some_and(|t| t.elapsed() < TASK_VIEW_STALE);
        let (press, view) = match (up, open) {
            (true, false) => (true, "overview"),
            (true, true) => (false, "overview"),
            (false, true) => (true, "windows"),
            (false, false) => (false, "windows"),
        };
        if press {
            // Windows+Tab opens Task View, and again closes it.
            input::press(input::TAB, &[input::WIN])?;
            *self.task_view.lock().unwrap() = (view == "overview").then(Instant::now);
        }
        Ok(view)
    }

    async fn desktop_switch(&self, right: bool) -> Result<()> {
        // Ctrl+Windows+Left / Right: the virtual desktop beside this one.
        input::press(if right { input::RIGHT } else { input::LEFT }, &[input::CONTROL, input::WIN])
    }

    async fn wallpaper_colors(&self) -> Result<Vec<[u8; 3]>> {
        tokio::task::spawn_blocking(wallpaper::colors).await?
    }

    fn wallpaper_signature(&self) -> Option<String> {
        Some(wallpaper::signature())
    }

    fn away(&self, phone: &str, body: &Value) {
        self.away.configure(phone, body);
    }

    /// Brightness, app volumes, power and wake in the order they came: a burst of sets for one
    /// slider (the phone sends while dragging) collapses to its newest, and the whole burst gets
    /// one `vyoma.pc` answer.
    fn pc_controls(&self, cx: &Cx) -> Option<mpsc::UnboundedSender<Value>> {
        let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
        let (id, sender) = (cx.id.clone(), cx.sender.clone());
        tokio::spawn(async move {
            while let Some(first) = rx.recv().await {
                let mut batch = vec![first];
                while let Ok(p) = rx.try_recv() {
                    batch.push(p);
                }
                let mut reply = false;
                for i in 0..batch.len() {
                    let p = &batch[i];
                    match p["type"].as_str().unwrap_or_default() {
                        pc::REQUEST => reply = true,
                        pc::SET => {
                            reply = true;
                            if pc::superseded(&batch, i) {
                                continue;
                            }
                            let body = p["body"].clone();
                            if let Ok(Err(e)) = tokio::task::spawn_blocking(move || pc::set(&body)).await {
                                tracing::info!("{id}: PC control: {e:#}");
                            }
                        }
                        "vyoma.pc.power" => {
                            let action = p["body"]["action"].as_str().unwrap_or_default().to_owned();
                            tracing::info!("{id}: asked this PC to {action}");
                            let mut out = json!({ "action": action });
                            // The answer goes first: the link may not outlive the action.
                            let done = {
                                let action = action.clone();
                                move || system::power(&action)
                            };
                            if let Err(e) = tokio::task::spawn_blocking(done).await.map_err(anyhow::Error::from).and_then(|r| r) {
                                tracing::warn!("{action}: {e:#}");
                                out["error"] = json!(format!("{e:#}"));
                            }
                            let _ = sender.send(&packet("vyoma.pc.powered", out)).await;
                        }
                        "vyoma.pc.wake" => {
                            if let Err(e) = system::wake() {
                                tracing::warn!("couldn't wake the screen: {e:#}");
                            }
                        }
                        other => tracing::debug!("{id}: no PC control {other} on Windows"),
                    }
                }
                if reply {
                    let state = tokio::task::spawn_blocking(pc::state).await.unwrap_or_else(|_| json!({ "displays": [], "apps": [] }));
                    if sender.send(&packet(pc::STATE, state)).await.is_err() {
                        return;
                    }
                }
            }
        });
        Some(tx)
    }
}

/// A sender whose packets go to phone [id] (through the hub), for the shared payload code.
pub fn sender(commands: &mpsc::UnboundedSender<Command>, id: &str) -> Sender {
    let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
    let (commands, to) = (commands.clone(), id.to_owned());
    tokio::spawn(async move {
        while let Some(p) = rx.recv().await {
            if commands.send(Command::Send(to.clone(), p)).is_err() {
                break;
            }
        }
    });
    Sender::queue(tx)
}

/// What this PC says around the phone's file traffic: a notification when the phone copies files
/// from it, "Received" for what it sends; sends land in Downloads.
struct WindowsFiles;

impl files::Hooks for WindowsFiles {
    fn reading(&self, phone: &str, name: &str) {
        tray::notify("File read from your phone", &format!("{phone} is copying \u{201c}{name}\u{201d} from this PC."));
    }

    fn received(&self, phone: &str, path: &Path, count: Option<u64>) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let body = match count {
            Some(n) => format!("{name} ({n} file{}) from {phone}, in Downloads", if n == 1 { "" } else { "s" }),
            None => format!("{name} from {phone}, in Downloads"),
        };
        tray::notify("Received", &body);
    }

    fn incoming_dir(&self, _: &str) -> PathBuf {
        win::downloads()
    }
}

// ---- stats --------------------------------------------------------------------------------------

/// The phone's Home stats, sampled while it's open. Windows gives programs no temperatures without
/// a driver, and no graphics load without performance counters (a later step): those stay empty.
pub struct Stats {
    sys: System,
}

impl Stats {
    pub fn new() -> Result<Self> {
        let mut sys = System::new().map_err(|e| anyhow::anyhow!("no stats on this PC: {e}"))?;
        sys.refresh_cpu_usage();
        Ok(Self { sys })
    }

    pub fn sample(&mut self) -> Value {
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();
        json!({
            "cpu": round1(self.sys.global_cpu_usage()),
            "memUsed": self.sys.used_memory(),
            "memTotal": self.sys.total_memory(),
            "cpuTemp": null,
            "gpu": null,
            "uptime": System::uptime().ok(),
        })
    }
}

impl Sampler for Stats {
    fn sample(&mut self) -> Pin<Box<dyn Future<Output = Value> + Send + '_>> {
        let v = Stats::sample(self);
        Box::pin(async move { v })
    }
}

fn round1(v: f32) -> f32 {
    (v * 10.0).round() / 10.0
}

// ---- what this PC is ----------------------------------------------------------------------------

/// About this PC. Never the serial number or the product key.
fn specs() -> Value {
    let sys = System::new_with_specifics(RefreshKind::nothing().with_cpu(CpuRefreshKind::nothing()).with_memory(MemoryRefreshKind::nothing().with_ram())).ok();
    let (gpus, displays) = screen::displays();
    let cpu = sys.as_ref().and_then(|s| s.cpus().first()).map(|c| c.brand().trim().to_owned()).filter(|b| !b.is_empty());
    let mut out = json!({
        "model": model(),
        "os": System::long_os_version().ok(),
        "desktop": null,
        "session": null,
        "cpu": cpu,
        "gpus": gpus,
        "displays": displays,
        "kernel": System::kernel_version().ok().map(|k| format!("Windows NT {k}")),
        "cores": System::physical_core_count().ok().filter(|&n| n > 0),
        "threads": sys.as_ref().map(|s| s.cpus().len()).filter(|&n| n > 0),
        "memTotal": sys.as_ref().map(|s| s.total_memory()).filter(|&m| m > 0),
    });
    if let Some((total, free)) = home_disk() {
        out["diskTotal"] = json!(total);
        out["diskFree"] = json!(free);
    }
    out
}

/// The maker and model as the firmware names them ("LENOVO 21CB…"): the registry's copy of SMBIOS,
/// which the BIOS key holds without the serial number.
fn model() -> Option<String> {
    let read = |name: &str| reg_string(r"HARDWARE\DESCRIPTION\System\BIOS", name);
    let (maker, product) = (read("SystemManufacturer"), read("SystemProductName"));
    let junk = |s: &String| {
        let l = s.to_lowercase();
        l.is_empty() || l.contains("to be filled") || l.contains("default string") || l == "system product name" || l == "system manufacturer"
    };
    match (maker.filter(|m| !junk(m)), product.filter(|p| !junk(p))) {
        (Some(m), Some(p)) if !p.to_lowercase().starts_with(&m.to_lowercase()) => Some(format!("{m} {p}")),
        (_, Some(p)) => Some(p),
        (Some(m), None) => Some(m),
        (None, None) => None,
    }
}

/// A string value under HKEY_LOCAL_MACHINE, read-only.
fn reg_string(key: &str, name: &str) -> Option<String> {
    use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
    let mut buf = [0u16; 512];
    let mut size = (buf.len() * 2) as u32;
    // SAFETY: reading a REG_SZ into a buffer of [size] bytes; Windows adds the NUL.
    let err = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, win::wide(key).as_ptr(), win::wide(name).as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut size) };
    if err != 0 {
        return None;
    }
    // SAFETY: NUL-terminated by RegGetValueW.
    let s = unsafe { win::from_wide(buf.as_ptr()) };
    Some(s.trim().to_owned()).filter(|s| !s.is_empty())
}

/// The drive the user's folder is on: total and free.
fn home_disk() -> Option<(u64, u64)> {
    let home = win::home();
    let disks = Disks::new_with_refreshed_list().ok()?;
    disks
        .iter()
        .filter(|d| home.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .map(|d| (d.total_space(), d.available_space()))
}

/// Puts [text] on the clipboard for the phone's paste, and what was there before back a moment
/// later, unless something newer was copied meanwhile.
async fn paste(text: &str) -> Result<()> {
    static GENERATION: AtomicU64 = AtomicU64::new(0);
    let before = win::clipboard();
    win::set_clipboard(text)?;
    let mine = GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
    let text = text.to_owned();
    tokio::spawn(async move {
        tokio::time::sleep(PASTE_RESTORE).await;
        if GENERATION.load(Ordering::Relaxed) == mine
            && win::clipboard().as_deref() == Some(text.as_str())
            && let Some(before) = before
            && let Err(e) = win::set_clipboard(&before)
        {
            tracing::debug!("the clipboard stayed on the paste: {e:#}");
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offers_what_it_serves() {
        let (incoming, _) = vyoma_engine::platform::capabilities_for(Windows::offers);
        for kind in ["vyoma.stats.request", "vyoma.screenshot.request", "vyoma.paste", "vyoma.desktop.step", "vyoma.pc.power", "vyoma.pc.wake", "vyoma.files.list", "vyoma.xfer.begin"] {
            assert!(incoming.contains(&kind), "{kind}");
        }
        assert!(incoming.contains(&"vyoma.screen.request"), "the live screen");
        for kind in ["vyoma.pc.request", "vyoma.pc.set", "vyoma.wallpaper.request", "vyoma.away"] {
            assert!(incoming.contains(&kind), "{kind}");
        }
        for kind in ["vyoma.camera.request", "vyoma.mirror", "vyoma.mirror.audio", "vyoma.mirror.copy", "vyoma.mirror.volume"] {
            assert!(incoming.contains(&kind), "{kind}");
        }
        assert!(incoming.contains(&"vyoma.gamepad.state"), "the controller");
        for kind in ["vyoma.webcam", "vyoma.webcam.turn", "vyoma.usb.accessory"] {
            assert!(incoming.contains(&kind), "{kind}");
        }
    }

    #[test]
    fn says_what_this_pc_is() {
        let s = specs();
        assert!(s["os"].as_str().is_some_and(|o| !o.is_empty()), "{s}");
        assert!(s["threads"].as_u64().unwrap_or(0) > 0, "{s}");
        assert!(s.get("serial").is_none());
        let mut stats = Stats::new().unwrap();
        let v = stats.sample();
        assert!(v["memTotal"].as_u64().unwrap() > 0);
    }
}
