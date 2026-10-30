//! Plug the phone in, and Vyoma's USB link just works: when a phone with USB debugging appears,
//! set up the adb reverse tunnel Vyoma dials over the cable, and the two one-time grants its
//! background clipboard sync needs. No adb server runs until a phone is actually plugged in.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use tracing::{info, warn};

const PACKAGES: [&str; 2] = ["dev.vyoma.connect", "dev.vyoma.connect.debug"];
/// Must match LanBackend.USB_REVERSE_PORT on the phone; kdeconnectd listens on 1716.
const REVERSE: [&str; 2] = ["tcp:1765", "tcp:1716"];
const POLL: Duration = Duration::from_secs(3);
/// How long to keep retrying a phone that shows "unauthorized" (the RSA prompt is on its screen).
const AUTH_WAIT: Duration = Duration::from_secs(90);
/// While a phone is plugged in, check its tunnel every this many polls (12 s). The tunnel dies with
/// the adb server (Android Studio and `adb kill-server` restart it), and while the server is down
/// the phone's end stays open, so the phone would sit on a dead link instead of using Wi-Fi.
/// Asking adb restarts the server, which makes the phone drop that link, and we re-create it.
const TUNNEL_CHECK_POLLS: u32 = 4;

pub async fn watch() {
    let Some(adb) = find_adb() else {
        return info!("adb not found; USB auto-setup is off");
    };
    let mut known: HashSet<String> = HashSet::new();
    let mut polls = 0u32;
    loop {
        let now = adb_interfaces();
        for dev in now.difference(&known) {
            info!("phone with USB debugging plugged in ({dev})");
            let adb = adb.clone();
            tokio::spawn(async move { setup_all(&adb).await });
        }
        known = now;
        polls = polls.wrapping_add(1);
        if !known.is_empty() && polls % TUNNEL_CHECK_POLLS == 0 {
            let adb = adb.clone();
            tokio::spawn(async move { recheck_tunnels(&adb).await });
        }
        tokio::time::sleep(POLL).await;
    }
}

/// USB devices exposing an adb interface (class ff, subclass 42, protocol 01). Just sysfs reads.
fn adb_interfaces() -> HashSet<String> {
    let Ok(entries) = std::fs::read_dir("/sys/bus/usb/devices") else { return HashSet::new() };
    entries
        .flatten()
        .filter(|e| {
            let p = e.path();
            let read = |f: &str| std::fs::read_to_string(p.join(f)).map(|s| s.trim().to_string()).unwrap_or_default();
            read("bInterfaceClass") == "ff" && read("bInterfaceSubClass") == "42" && read("bInterfaceProtocol") == "01"
        })
        .map(|e| e.file_name().to_string_lossy().split(':').next().unwrap_or_default().to_string())
        .collect()
}

async fn setup_all(adb: &PathBuf) {
    let deadline = tokio::time::Instant::now() + AUTH_WAIT;
    loop {
        let devices = list_devices(adb).await;
        let waiting = devices.iter().any(|(_, state)| state == "unauthorized" || state == "authorizing");
        for (serial, state) in &devices {
            if state == "device" {
                setup(adb, serial).await;
            }
        }
        if !waiting || tokio::time::Instant::now() > deadline {
            if waiting {
                warn!("phone still asks to allow USB debugging; plug it in again after allowing");
            }
            return;
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

/// Re-creates the tunnel on any plugged-in Vyoma phone that has lost it.
async fn recheck_tunnels(adb: &PathBuf) {
    for (serial, state) in list_devices(adb).await {
        if state != "device" || NOT_VYOMA.lock().unwrap().contains(&serial) {
            continue;
        }
        let Some(list) = run(adb, &serial, &["reverse", "--list"]).await else { continue };
        if !list.contains(REVERSE[0]) {
            info!("{serial}: USB tunnel was gone (adb server restarted?); setting it up again");
            setup(adb, &serial).await;
        }
    }
}

/// Phones seen without Vyoma installed, so the periodic check leaves them alone.
static NOT_VYOMA: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

async fn setup(adb: &PathBuf, serial: &str) {
    let installed: Vec<&str> = {
        let mut v = Vec::new();
        for pkg in PACKAGES {
            if run(adb, serial, &["shell", "pm", "path", pkg]).await.is_some_and(|o| o.starts_with("package:")) {
                v.push(pkg);
            }
        }
        v
    };
    if installed.is_empty() {
        NOT_VYOMA.lock().unwrap().push(serial.to_string());
        return; // Not a Vyoma phone: leave it alone.
    }
    NOT_VYOMA.lock().unwrap().retain(|s| s != serial);
    match run(adb, serial, &["reverse", REVERSE[0], REVERSE[1]]).await {
        Some(_) => info!("{serial}: USB tunnel ready"),
        None => warn!("{serial}: couldn't set up the USB tunnel"),
    }
    for pkg in installed {
        // Both are idempotent; they only ever touch Vyoma's own permissions.
        let logs = run(adb, serial, &["shell", "pm", "grant", pkg, "android.permission.READ_LOGS"]).await;
        let overlay = run(adb, serial, &["shell", "appops", "set", pkg, "SYSTEM_ALERT_WINDOW", "allow"]).await;
        if logs.is_some() && overlay.is_some() {
            info!("{serial}: background clipboard sync allowed for {pkg}");
        }
    }
}

async fn list_devices(adb: &PathBuf) -> Vec<(String, String)> {
    let Ok(out) = tokio::process::Command::new(adb).arg("devices").output().await else { return Vec::new() };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .skip(1)
        .filter_map(|l| l.split_once('\t'))
        .map(|(s, st)| (s.to_string(), st.trim().to_string()))
        .collect()
}

/// Runs an adb command for one device; stdout on success.
async fn run(adb: &PathBuf, serial: &str, args: &[&str]) -> Option<String> {
    let out = tokio::time::timeout(
        Duration::from_secs(15),
        tokio::process::Command::new(adb).args(["-s", serial]).args(args).kill_on_drop(true).output(),
    )
    .await
    .ok()?
    .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// $ANDROID_HOME, the Android Studio default (~/Android/Sdk), then PATH.
fn find_adb() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let candidates = [
        std::env::var_os("ANDROID_HOME").map(|h| PathBuf::from(h).join("platform-tools/adb")),
        std::env::var_os("ANDROID_SDK_ROOT").map(|h| PathBuf::from(h).join("platform-tools/adb")),
        Some(home.join("Android/Sdk/platform-tools/adb")),
    ];
    candidates.into_iter().flatten().find(|p| p.is_file()).or_else(|| {
        std::env::var_os("PATH").and_then(|path| std::env::split_paths(&path).map(|d| d.join("adb")).find(|p| p.is_file()))
    })
}
