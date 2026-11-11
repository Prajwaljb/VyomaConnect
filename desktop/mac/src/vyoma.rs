//! Vyoma's own features on this Mac, the ones it has so far: the stats on the phone's Home (while
//! it's open), what this Mac is (the phone's About this PC), sleep, restart, shut down and log out,
//! waking the screen, the wallpaper's colours for the phone's background, and the Mac's files
//! from the phone (browse, open, upload; resumable sends and whole folders), through the shared
//! `vyoma_core::files` and `xfer`. The phone offers only what [INCOMING] lists (its `PcFeature`s),
//! so nothing here can leave a tile waiting.
//!
//! macOS's own tools only, with fixed arguments: `system_profiler` and `sw_vers` for what this Mac
//! is (never its serial number or hardware UUID, which the same report holds), `ioreg` for the
//! graphics load, `pmset` for the charger and sleep, `caffeinate` to wake the screen, Apple events
//! to `loginwindow` for restart, shut down and log out (Apple's Technical Q&A 1134), `sips` to read
//! the wallpaper, whatever format it's in.
//! Built and tested on CI only; not yet run on a real Mac.

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use sysinfo::{Components, CpuRefreshKind, Disks, MemoryRefreshKind, RefreshKind, System};
use tokio::process::Command as Process;
use tokio::sync::mpsc;
use vyoma_core::activity::Activity;
use vyoma_core::device::Command;
use vyoma_core::identity::Identity;
use vyoma_core::link::{Sender, packet};
use vyoma_core::{activity, files, xfer};

pub const STATS_REQUEST: &str = "vyoma.stats.request";
pub const STATS: &str = "vyoma.stats";
pub const SPECS_REQUEST: &str = "vyoma.specs.request";
pub const SPECS: &str = "vyoma.specs";
pub const POWER: &str = "vyoma.pc.power";
pub const POWERED: &str = "vyoma.pc.powered";
pub const WAKE: &str = "vyoma.pc.wake";
pub const WALLPAPER_REQUEST: &str = "vyoma.wallpaper.request";
pub const WALLPAPER: &str = "vyoma.wallpaper";

pub const FILES_LIST: &str = "vyoma.files.list";
pub const FILES_GET: &str = "vyoma.files.get";
pub const FILES_PUT: &str = "vyoma.files.put";
pub const XFER_BEGIN: &str = "vyoma.xfer.begin";
pub const XFER_DATA: &str = "vyoma.xfer.data";
pub const XFER_END: &str = "vyoma.xfer.end";

pub const INCOMING: &[&str] = &[STATS_REQUEST, SPECS_REQUEST, POWER, WAKE, WALLPAPER_REQUEST, FILES_LIST, FILES_GET, FILES_PUT, XFER_BEGIN, XFER_DATA, XFER_END];
pub const OUTGOING: &[&str] = &[STATS, SPECS, POWERED, WALLPAPER, "vyoma.files.listing", "vyoma.files.file", "vyoma.files.saved", "vyoma.xfer.ready", "vyoma.xfer.done", vyoma_core::permits::DENIED];

/// The phone's stats come for this long after it last asked (it asks every few seconds while its
/// Home is on screen), one sample every [STATS_EVERY]: an unwatched Mac samples nothing.
const STATS_LEASE: Duration = Duration::from_secs(12);
const STATS_EVERY: Duration = Duration::from_secs(2);
/// The wallpaper as the phone's background: a grid this size, which it stretches smoothly.
const COLS: usize = 4;
const ROWS: usize = 4;
/// What a tool gets before it's given up on.
const TOOL_TIMEOUT: Duration = Duration::from_secs(15);

pub fn takes(kind: &str) -> bool {
    INCOMING.contains(&kind)
}

/// The phone a packet came from.
pub struct Phone<'a> {
    pub name: &'a str,
    /// Its certificate, which its payload connections must show.
    pub cert: &'a [u8],
    /// Where the packet came from: where its payload is fetched.
    pub from: IpAddr,
}

/// Each linked phone's stats stream and file transfers, and how to reach it.
pub struct Vyoma {
    me: Arc<Identity>,
    commands: mpsc::UnboundedSender<Command>,
    activity: Arc<Activity>,
    streams: HashMap<String, (Arc<Mutex<Instant>>, tokio::task::JoinHandle<()>)>,
    /// A sender per phone for the shared file code; dropping its transfers cancels them.
    links: HashMap<String, (Sender, tokio::task::JoinSet<()>)>,
}

impl Vyoma {
    pub fn new(me: Arc<Identity>, commands: mpsc::UnboundedSender<Command>, activity: Arc<Activity>) -> Self {
        files::set_hooks(Box::new(MacFiles));
        Self { me, commands, activity, streams: HashMap::new(), links: HashMap::new() }
    }

    /// Phone [id]'s sender and its set of transfers.
    fn link(&mut self, id: &str) -> (Sender, &mut tokio::task::JoinSet<()>) {
        let (commands, to) = (self.commands.clone(), id.to_owned());
        let (sender, tasks) = self.links.entry(id.to_owned()).or_insert_with(|| {
            let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
            tokio::spawn(async move {
                while let Some(p) = rx.recv().await {
                    if commands.send(Command::Send(to.clone(), p)).is_err() {
                        break;
                    }
                }
            });
            (Sender::queue(tx), tokio::task::JoinSet::new())
        });
        (sender.clone(), tasks)
    }

    /// One of [INCOMING] from phone [id]. [wallpaper] is the desktop picture's file, which macOS
    /// only tells the main thread (see `wallpaper_path`).
    pub fn handle(&mut self, id: &str, phone: &Phone, p: &Value, wallpaper: Option<PathBuf>) {
        let reply = {
            let (commands, id) = (self.commands.clone(), id.to_owned());
            move |kind: &str, body: Value| {
                let _ = commands.send(Command::Send(id.clone(), packet(kind, body)));
            }
        };
        match p["type"].as_str().unwrap_or("") {
            STATS_REQUEST => {
                let until = Instant::now() + STATS_LEASE;
                if let Some((lease, task)) = self.streams.get(id)
                    && !task.is_finished()
                {
                    *lease.lock().unwrap() = until;
                    return;
                }
                tracing::info!("{id}: stats while its Home is open");
                let lease = Arc::new(Mutex::new(until));
                let task = tokio::spawn(stream_stats(lease.clone(), reply));
                self.streams.insert(id.to_owned(), (lease, task));
            }
            SPECS_REQUEST => {
                tokio::spawn(async move { reply(SPECS, specs().await) });
            }
            POWER => {
                let action = p["body"]["action"].as_str().unwrap_or_default().to_owned();
                tracing::info!("{id}: asked this Mac to {action}");
                // Before it's done: a shut down may not leave time after.
                let what = match action.as_str() {
                    "sleep" => Some("Put this Mac to sleep"),
                    "restart" => Some("Restarted this Mac"),
                    "shutdown" => Some("Shut down this Mac"),
                    "logout" => Some("Logged out of this Mac"),
                    _ => None,
                };
                if let Some(what) = what {
                    self.activity.add(id, phone.name, "power", what);
                }
                tokio::spawn(async move {
                    let mut out = json!({ "action": action });
                    if let Err(e) = power(&action).await {
                        tracing::warn!("{action}: {e:#}");
                        out["error"] = json!(format!("{e:#}"));
                    }
                    reply(POWERED, out);
                });
            }
            WAKE => {
                self.activity.add(id, phone.name, "power", "Woke this Mac's screen");
                tokio::spawn(async {
                    if let Err(e) = wake().await {
                        tracing::warn!("couldn't wake the screen: {e:#}");
                    }
                });
            }
            WALLPAPER_REQUEST => {
                tokio::spawn(async move {
                    let body = match wallpaper {
                        None => json!({ "error": "couldn't find this Mac's wallpaper" }),
                        Some(path) => match colors(&path).await {
                            Ok(c) => json!({ "cols": COLS, "rows": ROWS, "colors": c }),
                            Err(e) => json!({ "error": format!("{e:#}") }),
                        },
                    };
                    reply(WALLPAPER, body);
                });
            }
            FILES_LIST | FILES_GET | FILES_PUT | XFER_BEGIN | XFER_DATA | XFER_END => self.file(id, phone, p),
            _ => {}
        }
    }

    /// The phone's file traffic, as the Linux app's session runs it.
    fn file(&mut self, id: &str, phone: &Phone, p: &Value) {
        let (me, cert, from, name, phone_id, p) = (self.me.clone(), phone.cert.to_vec(), phone.from, phone.name.to_owned(), id.to_owned(), p.clone());
        let rel = p["body"]["path"].as_str().unwrap_or_default().to_owned();
        let kind = p["type"].as_str().unwrap_or_default().to_owned();
        if kind == XFER_END {
            if let Some((folder, count)) = xfer::end(&name, &p["body"]) {
                self.activity.add(id, &name, "files", format!("Sent the folder \u{201c}{folder}\u{201d} to this Mac ({count} {})", if count == 1 { "file" } else { "files" }));
            }
            return;
        }
        let (activity, (sender, tasks)) = (self.activity.clone(), self.link(id));
        tasks.spawn(async move {
            let log = |text: String| activity.add(&phone_id, &name, "files", text);
            let done = match kind.as_str() {
                FILES_LIST => files::list(&sender, &rel).await,
                FILES_GET => files::send(&sender, &me, &cert, &rel, &name).await.map(|()| log(format!("Copied \u{201c}{}\u{201d} from this Mac", activity::file_name(&rel)))),
                FILES_PUT => files::receive(&sender, &me, &cert, from, &p, &name)
                    .await
                    .map(|()| log(format!("Saved \u{201c}{}\u{201d} to this Mac", activity::file_name(p["body"]["name"].as_str().unwrap_or("a file"))))),
                XFER_BEGIN => xfer::begin(&sender, &phone_id, &p["body"]).await,
                _ => xfer::data(&sender, &me, &cert, from, &name, &p).await.map(|saved| {
                    if let Some(file) = saved {
                        log(format!("Sent \u{201c}{file}\u{201d} to this Mac"));
                    }
                }),
            };
            if let Err(e) = done {
                tracing::warn!("{phone_id}: {kind}: {e:#}");
            }
        });
    }

    /// Phone [id] went: its stream ends, and its transfers with it (a resumable one picks up
    /// where it stopped when the phone is back).
    pub fn gone(&mut self, id: &str) {
        if let Some((_, task)) = self.streams.remove(id) {
            task.abort();
        }
        self.links.remove(id);
    }
}

/// What this Mac says around the phone's file traffic: a notification when the phone copies
/// files from it, "Received" for what it sends; sends land in ~/Downloads.
struct MacFiles;

impl files::Hooks for MacFiles {
    fn reading(&self, phone: &str, name: &str) {
        let body = format!("{phone} is copying \u{201c}{name}\u{201d} from this Mac.");
        tokio::spawn(async move { crate::mac::notify("File read from your phone", &body).await });
    }

    fn received(&self, phone: &str, path: &Path, count: Option<u64>) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let body = match count {
            Some(n) => format!("{name} ({n} file{}) from {phone}, in Downloads", if n == 1 { "" } else { "s" }),
            None => format!("{name} from {phone}, in Downloads"),
        };
        tokio::spawn(async move { crate::mac::notify("Received", &body).await });
    }

    fn incoming_dir(&self, _: &str) -> PathBuf {
        crate::mac::downloads()
    }
}

// ---- stats --------------------------------------------------------------------------------------

async fn stream_stats(lease: Arc<Mutex<Instant>>, reply: impl Fn(&str, Value)) {
    let (mut sys, mut components) = match (System::new(), Components::new_with_refreshed_list()) {
        (Ok(s), Ok(c)) => (s, c),
        _ => return tracing::warn!("no stats on this Mac"),
    };
    sys.refresh_cpu_usage();
    let mut tick = tokio::time::interval(STATS_EVERY);
    while Instant::now() < *lease.lock().unwrap() {
        tick.tick().await;
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        components.refresh(false);
        let temps: Vec<(String, f32)> = components.iter().filter_map(|c| Some((c.label().to_owned(), c.temperature()?))).collect();
        let mut body = json!({
            "cpu": round1(sys.global_cpu_usage()),
            "memUsed": sys.used_memory(),
            "memTotal": sys.total_memory(),
            "cpuTemp": cpu_temp(&temps).map(round1),
            "gpu": gpu_busy().await,
            "uptime": System::uptime().ok(),
        });
        if let Some(ac) = on_ac().await {
            body["ac"] = json!(ac);
        }
        reply(STATS, body);
    }
}

/// The processor's temperature: Apple silicon's die sensors (`PMU tdie…`), the hottest; on an
/// Intel Mac the CPU's own.
fn cpu_temp(readings: &[(String, f32)]) -> Option<f32> {
    let ok: Vec<(String, f32)> = readings.iter().map(|(l, t)| (l.to_lowercase(), *t)).filter(|(_, t)| t.is_finite() && *t > 0.0 && *t < 150.0).collect();
    let hottest = |want: &dyn Fn(&str) -> bool| ok.iter().filter(|(l, _)| want(l)).map(|(_, t)| *t).max_by(f32::total_cmp);
    hottest(&|l| l.contains("tdie")).or_else(|| hottest(&|l| l.contains("cpu") || l.contains("core") || l.contains("package")))
}

/// The graphics processor's load, as Activity Monitor reads it: each accelerator's
/// `"Device Utilization %"` in `ioreg`.
async fn gpu_busy() -> Option<u8> {
    let out = tool("ioreg", &["-r", "-d", "1", "-c", "IOAccelerator"]).await.ok()?;
    parse_gpu(&out)
}

fn parse_gpu(text: &str) -> Option<u8> {
    const KEY: &str = "\"Device Utilization %\"=";
    text.match_indices(KEY)
        .filter_map(|(i, _)| text[i + KEY.len()..].chars().take_while(char::is_ascii_digit).collect::<String>().parse::<u8>().ok())
        .map(|v| v.min(100))
        .max()
}

/// On its charger: `pmset -g batt` says what it's drawing from. None if it won't say.
async fn on_ac() -> Option<bool> {
    let out = tool("pmset", &["-g", "batt"]).await.ok()?;
    let first = out.lines().next()?;
    first.contains("Now drawing from").then(|| first.contains("'AC Power'"))
}

fn round1(v: f32) -> f32 {
    (v * 10.0).round() / 10.0
}

// ---- what this Mac is ---------------------------------------------------------------------------

async fn specs() -> Value {
    let profile = tool("system_profiler", &["-json", "SPHardwareDataType", "SPDisplaysDataType"])
        .await
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .unwrap_or(Value::Null);
    let version = tool("sw_vers", &["-productVersion"]).await.ok().map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
    let sys = System::new_with_specifics(RefreshKind::nothing().with_cpu(CpuRefreshKind::nothing()).with_memory(MemoryRefreshKind::nothing().with_ram())).ok();
    let mut out = read_specs(&profile, version.as_deref());
    out["kernel"] = json!(System::kernel_version().ok().map(|k| format!("Darwin {k}")));
    out["cores"] = json!(System::physical_core_count().ok().filter(|&n| n > 0));
    out["threads"] = json!(sys.as_ref().map(|s| s.cpus().len()).filter(|&n| n > 0));
    out["memTotal"] = json!(sys.as_ref().map(|s| s.total_memory()).filter(|&m| m > 0));
    if let Some((total, free)) = home_disk() {
        out["diskTotal"] = json!(total);
        out["diskFree"] = json!(free);
    }
    out
}

/// What `system_profiler -json SPHardwareDataType SPDisplaysDataType` says, in the phone's terms.
/// Only the fields named here are read: the same report holds the serial number and the hardware
/// UUID, which never leave this Mac.
fn read_specs(profile: &Value, version: Option<&str>) -> Value {
    let hw = &profile["SPHardwareDataType"][0];
    let s = |v: &Value| v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
    // Apple silicon names its chip; an Intel Mac its processor ("Quad-Core Intel Core i7").
    let cpu = s(&hw["chip_type"]).or_else(|| s(&hw["cpu_type"]));
    let graphics = profile["SPDisplaysDataType"].as_array().cloned().unwrap_or_default();
    let mut gpus: Vec<String> = graphics.iter().filter_map(|g| s(&g["sppci_model"])).collect();
    gpus.dedup();
    let mut displays: Vec<(bool, Value)> = graphics
        .iter()
        .flat_map(|g| g["spdisplays_ndrvs"].as_array().cloned().unwrap_or_default())
        .filter_map(|d| {
            // "2880 x 1864": the panel's own pixels ("_spdisplays_resolution" is the scaled size).
            let pixels = s(&d["_spdisplays_pixels"]).or_else(|| s(&d["spdisplays_pixelresolution"]))?;
            let mut it = pixels.split('x').map(|n| n.trim().split(' ').next().unwrap_or("").parse::<u32>());
            let (w, h) = (it.next()?.ok()?, it.next()?.ok()?);
            let internal = s(&d["spdisplays_connection_type"]).is_some_and(|c| c.contains("internal"));
            Some((internal, json!({ "w": w, "h": h, "internal": internal })))
        })
        .collect();
    // The built-in screen first, as the Linux app lists them.
    displays.sort_by_key(|(internal, _)| !*internal);
    json!({
        "model": s(&hw["machine_name"]),
        "os": version.map(macos_name),
        "desktop": null,
        "session": null,
        "cpu": cpu,
        "gpus": gpus,
        "displays": displays.into_iter().map(|(_, d)| d).collect::<Vec<_>>(),
    })
}

/// "macOS Sequoia 15.1" from "15.1".
fn macos_name(version: &str) -> String {
    let name = match version.split('.').next().and_then(|m| m.parse::<u32>().ok()) {
        Some(11) => Some("Big Sur"),
        Some(12) => Some("Monterey"),
        Some(13) => Some("Ventura"),
        Some(14) => Some("Sonoma"),
        Some(15) => Some("Sequoia"),
        Some(26) => Some("Tahoe"),
        _ => None,
    };
    match name {
        Some(n) => format!("macOS {n} {version}"),
        None => format!("macOS {version}"),
    }
}

/// The disk the home folder is on: total and free.
fn home_disk() -> Option<(u64, u64)> {
    let home = crate::mac::home();
    let disks = Disks::new_with_refreshed_list().ok()?;
    disks
        .iter()
        .filter(|d| home.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .map(|d| (d.total_space(), d.available_space()))
}

// ---- power ------------------------------------------------------------------------------------

/// The phone asked first, so no "are you sure" here: loginwindow's "really" events restart, shut
/// down and log out at once, asking open apps to quit as the Apple menu does.
async fn power(action: &str) -> Result<()> {
    let event = match action {
        "sleep" => {
            tool("pmset", &["sleepnow"]).await.context("this Mac didn't go to sleep")?;
            return Ok(());
        }
        "restart" => "aevtrrst",
        "shutdown" => "aevtrsdn",
        "logout" => "aevtrlgo",
        other => bail!("unknown power action {other}"),
    };
    let script = format!("tell application \"loginwindow\" to \u{ab}event {event}\u{bb}");
    match tool("osascript", &["-e", &script]).await {
        Ok(_) => Ok(()),
        Err(e) if format!("{e:#}").contains("-1743") => {
            bail!("allow Vyoma in System Settings > Privacy & Security > Automation first")
        }
        Err(e) => Err(e),
    }
}

/// Lights the screen as a key press would (`caffeinate -u` declares the user active).
async fn wake() -> Result<()> {
    tool("caffeinate", &["-u", "-t", "2"]).await.map(|_| ())
}

// ---- wallpaper --------------------------------------------------------------------------------

/// The desktop picture's file, from AppKit, which answers only on the main thread (the one the
/// daemon's loop runs on); None elsewhere or when it isn't a file (a moving wallpaper).
#[cfg(target_os = "macos")]
pub fn wallpaper_path() -> Option<std::path::PathBuf> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSScreen, NSWorkspace};
    let mtm = MainThreadMarker::new()?;
    objc2::rc::autoreleasepool(|_| {
        let screen = NSScreen::mainScreen(mtm)?;
        let url = NSWorkspace::sharedWorkspace().desktopImageURLForScreen(&screen)?;
        url.path().map(|p| std::path::PathBuf::from(p.to_string())).filter(|p| p.is_file())
    })
}

#[cfg(not(target_os = "macos"))]
pub fn wallpaper_path() -> Option<std::path::PathBuf> {
    None
}

/// The picture averaged down to [COLS]×[ROWS]: sips makes a small PNG of it (it reads HEIC, which
/// macOS's own wallpapers are), and its blocks are averaged here.
async fn colors(path: &Path) -> Result<Vec<[u8; 3]>> {
    let dir = std::env::temp_dir().join(format!("vyoma-wallpaper-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let small = dir.join("small.png");
    let source = path.to_str().context("the wallpaper's path isn't text")?;
    let made = tool("sips", &["-s", "format", "png", "-Z", "64", source, "--out", small.to_str().unwrap_or_default()]).await;
    let png = made.and_then(|_| std::fs::read(&small).context("sips made nothing"));
    let _ = std::fs::remove_dir_all(&dir);
    average(&png?)
}

fn average(png: &[u8]) -> Result<Vec<[u8; 3]>> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().context("not a picture sips made")?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf)?;
    let channels = match info.color_type {
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Indexed => bail!("an indexed picture"),
    };
    let (w, h) = (info.width as usize, info.height as usize);
    if w < COLS || h < ROWS {
        bail!("the wallpaper is too small");
    }
    let mut out = Vec::with_capacity(COLS * ROWS);
    for row in 0..ROWS {
        for col in 0..COLS {
            let (mut sum, mut n) = ([0u64; 3], 0u64);
            for y in row * h / ROWS..(row + 1) * h / ROWS {
                for x in col * w / COLS..(col + 1) * w / COLS {
                    let px = &buf[(y * w + x) * channels..][..channels];
                    let rgb = if channels >= 3 { [px[0], px[1], px[2]] } else { [px[0]; 3] };
                    for (s, v) in sum.iter_mut().zip(rgb) {
                        *s += v as u64;
                    }
                    n += 1;
                }
            }
            out.push(sum.map(|s| (s / n.max(1)) as u8));
        }
    }
    Ok(out)
}

// ---- tools ------------------------------------------------------------------------------------

/// Runs one of macOS's tools with fixed arguments; its output, or what it said went wrong.
async fn tool(program: &str, args: &[&str]) -> Result<String> {
    let out = tokio::time::timeout(TOOL_TIMEOUT, Process::new(program).args(args).kill_on_drop(true).output())
        .await
        .with_context(|| format!("{program} took too long"))?
        .with_context(|| format!("no {program}"))?;
    if !out.status.success() {
        bail!("{program}: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_graphics_load() {
        let text = r#"+-o AGXAcceleratorG14X  <class AGXAcceleratorG14X>
    | "PerformanceStatistics" = {"In use system memory"=123,"Device Utilization %"=37,"Renderer Utilization %"=30}
"#;
        assert_eq!(parse_gpu(text), Some(37));
        assert_eq!(parse_gpu("nothing"), None);
    }

    #[test]
    fn picks_the_processor_temperature() {
        let t = |l: &str, v: f32| (l.to_owned(), v);
        assert_eq!(cpu_temp(&[t("PMU tdie1", 41.0), t("PMU tdie4", 47.5), t("NAND CH0 temp", 60.0)]), Some(47.5));
        assert_eq!(cpu_temp(&[t("CPU Proximity", 52.0), t("Battery", 30.0)]), Some(52.0));
        assert_eq!(cpu_temp(&[t("Battery", 30.0)]), None);
    }

    #[test]
    fn names_the_system() {
        assert_eq!(macos_name("15.1"), "macOS Sequoia 15.1");
        assert_eq!(macos_name("26.0"), "macOS Tahoe 26.0");
        assert_eq!(macos_name("30.0"), "macOS 30.0");
    }

    #[test]
    fn reads_what_this_mac_is_and_never_its_serial() {
        let profile = json!({
            "SPHardwareDataType": [{
                "machine_name": "MacBook Air", "machine_model": "Mac14,2", "chip_type": "Apple M2",
                "serial_number": "C02XXXXXXXXX", "platform_UUID": "00000000-0000-0000-0000-000000000000",
            }],
            "SPDisplaysDataType": [{
                "sppci_model": "Apple M2",
                "spdisplays_ndrvs": [
                    { "_spdisplays_pixels": "2560 x 1440", "spdisplays_connection_type": "spdisplays_displayport" },
                    { "_spdisplays_pixels": "2560 x 1664", "spdisplays_connection_type": "spdisplays_internal" },
                ],
            }],
        });
        let out = read_specs(&profile, Some("15.1"));
        assert_eq!(out["model"], "MacBook Air");
        assert_eq!(out["cpu"], "Apple M2");
        assert_eq!(out["os"], "macOS Sequoia 15.1");
        assert_eq!(out["gpus"], json!(["Apple M2"]));
        assert_eq!(out["displays"], json!([{ "w": 2560, "h": 1664, "internal": true }, { "w": 2560, "h": 1440, "internal": false }]));
        let text = out.to_string();
        assert!(!text.contains("C02X") && !text.contains("0000-0000") && !text.contains("Mac14,2"));
    }

    #[test]
    fn averages_the_wallpaper_into_its_grid() {
        // Left half red, right half blue, 8×8.
        let mut png_bytes = Vec::new();
        {
            let mut e = png::Encoder::new(&mut png_bytes, 8, 8);
            e.set_color(png::ColorType::Rgb);
            e.set_depth(png::BitDepth::Eight);
            let mut w = e.write_header().unwrap();
            let data: Vec<u8> = (0..64).flat_map(|i| if i % 8 < 4 { [255, 0, 0] } else { [0, 0, 255] }).collect();
            w.write_image_data(&data).unwrap();
        }
        let c = average(&png_bytes).unwrap();
        assert_eq!(c.len(), COLS * ROWS);
        assert_eq!(c[0], [255, 0, 0]);
        assert_eq!(c[3], [0, 0, 255]);
    }

    #[test]
    fn lists_what_it_takes() {
        assert!(INCOMING.iter().all(|k| takes(k)));
        assert!(!takes("vyoma.screen.request"));
    }
}
