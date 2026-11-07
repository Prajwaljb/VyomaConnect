//! Plug the phone in, and Vyoma's USB link just works: when a phone with USB debugging appears,
//! set up the adb reverse tunnel Vyoma dials over the cable, and the two one-time grants its
//! background clipboard sync needs. No adb server runs until a phone is actually plugged in.
//!
//! Any app can call itself dev.vyoma.connect. The tunnels don't care (every link and payload still
//! proves its certificate), but the grants are real power on the phone, so they only go to an APK
//! whose signing certificate the person at this PC has allowed once.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use tracing::{info, warn};

static RECHECKING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
const PACKAGES: [&str; 2] = ["dev.vyoma.connect", "dev.vyoma.connect.debug"];
/// Must match LanBackend.USB_REVERSE_PORT on the phone; kdeconnectd listens on 1716.
const REVERSE: [&str; 2] = ["tcp:1765", "tcp:1716"];
/// kdeconnectd offers payloads (album art, files) on one of these ports. Each is mirrored on the
/// phone at port + PAYLOAD_SHIFT, clear of the phone's own payload ports; must match
/// LanPayloads.USB_PAYLOAD_SHIFT. Without them album art never arrived over the cable.
const PAYLOAD_PORTS: std::ops::RangeInclusive<u16> = 1739..=1764;
const PAYLOAD_SHIFT: u16 = 100;
/// The other way: the phone's own payload ports (it offers files on 1739..1764 too) are reached
/// from here at port + FORWARD_SHIFT; over the cable the phone announces the shifted port, so
/// kdeconnectd's "connect to the phone's address" lands on this PC's mirror. Without them
/// nothing sent from the phone ever arrived over the cable. Must match LanPayloads.USB_FORWARD_SHIFT.
const FORWARD_SHIFT: u16 = 200;
/// The controller's wired channel: the phone's 1866 reaches the companion's own 127.0.0.1:1766
/// (gamepad.rs). Outside 1716..1764 like the tunnel; must match CompanionPlugin.GAMEPAD_WIRED_PORT.
const WIRED: [&str; 2] = ["tcp:1866", "tcp:1766"];
/// The phone's own listener, reached from here through `adb forward`, so the companion can link
/// over the cable alone (Wi-Fi off, or a network that keeps them apart).
const PHONE_LISTENER: &str = "tcp:1716";
const POLL: Duration = Duration::from_secs(3);
/// How long the "Allow Vyoma?" question stays up, and how long fetching an APK to check may take.
const ASK_WAIT: Duration = Duration::from_secs(300);
const PULL_WAIT: Duration = Duration::from_secs(60);
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
        let plugged_in = now.difference(&known).next().is_some();
        for dev in now.difference(&known) {
            info!("phone with USB debugging plugged in ({dev})");
            let adb = adb.clone();
            tokio::spawn(async move { setup_all(&adb).await });
        }
        known = now;
        if known.is_empty() {
            CABLE.lock().unwrap().clear();
        }
        polls = polls.wrapping_add(1);
        // Just plugged in, it's being set up: no tunnel yet isn't a lost tunnel.
        if !known.is_empty() && !plugged_in && polls % TUNNEL_CHECK_POLLS == 0 {
            // One at a time: with a stuck adb server, each tick would add another waiting on it.
            if !RECHECKING.swap(true, std::sync::atomic::Ordering::AcqRel) {
                let adb = adb.clone();
                tokio::spawn(async move {
                    recheck_tunnels(&adb).await;
                    RECHECKING.store(false, std::sync::atomic::Ordering::Release);
                });
            }
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

/// Local ports that reach a plugged-in phone's own listener (adb forwards), by serial.
static CABLE: std::sync::Mutex<Vec<(String, u16)>> = std::sync::Mutex::new(Vec::new());
/// Phones being set up: plugging in starts it, and the periodic check can come along meanwhile.
static SETTING_UP: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Where the companion can dial plugged-in Vyoma phones over the cable (127.0.0.1 ports).
pub fn cable_ports() -> Vec<u16> {
    CABLE.lock().unwrap().iter().map(|(_, p)| *p).collect()
}

/// Re-creates the tunnel on any plugged-in Vyoma phone that has lost it.
async fn recheck_tunnels(adb: &PathBuf) {
    let devices = list_devices(adb).await;
    CABLE.lock().unwrap().retain(|(serial, _)| devices.iter().any(|(s, st)| s == serial && st == "device"));
    for (serial, state) in devices {
        if state != "device" || NOT_VYOMA.lock().unwrap().contains(&serial) || SETTING_UP.lock().unwrap().contains(&serial) {
            continue;
        }
        let Some(list) = run(adb, &serial, &["reverse", "--list"]).await else { continue };
        let mirror = format!("tcp:{}", PAYLOAD_PORTS.start() + PAYLOAD_SHIFT);
        let forwards = run(adb, &serial, &["forward", "--list"]).await.unwrap_or_default();
        let forward = format!("tcp:{} tcp:{}", PAYLOAD_PORTS.start() + FORWARD_SHIFT, PAYLOAD_PORTS.start());
        let cable = CABLE.lock().unwrap().iter().find(|(s, _)| *s == serial).map(|(_, p)| format!("tcp:{p} {PHONE_LISTENER}"));
        if !list.contains(REVERSE[0]) || !list.contains(&mirror) || !list.contains(WIRED[0]) || !forwards.contains(&forward)
            || !cable.is_some_and(|c| forwards.contains(&c))
        {
            info!("{serial}: USB tunnel was gone (adb server restarted?); setting it up again");
            setup(adb, &serial).await;
        }
    }
}

/// Phones seen without Vyoma installed, so the periodic check leaves them alone.
static NOT_VYOMA: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

async fn setup(adb: &PathBuf, serial: &str) {
    {
        let mut running = SETTING_UP.lock().unwrap();
        if running.iter().any(|s| s == serial) {
            return;
        }
        running.push(serial.to_string());
    }
    set_up(adb, serial).await;
    SETTING_UP.lock().unwrap().retain(|s| s != serial);
}

async fn set_up(adb: &PathBuf, serial: &str) {
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
    if run(adb, serial, &["reverse", WIRED[0], WIRED[1]]).await.is_none() {
        warn!("{serial}: no wired channel for the controller");
    }
    // tcp:0 lets adb pick a free local port and print it.
    match run(adb, serial, &["forward", "tcp:0", PHONE_LISTENER]).await.and_then(|o| o.trim().parse::<u16>().ok()) {
        Some(port) => {
            let old = {
                let mut cable = CABLE.lock().unwrap();
                let old = cable.iter().find(|(s, _)| s == serial).map(|(_, p)| *p);
                cable.retain(|(s, _)| s != serial);
                cable.push((serial.to_string(), port));
                old
            };
            if let Some(old) = old.filter(|old| *old != port) {
                run(adb, serial, &["forward", "--remove", &format!("tcp:{old}")]).await;
            }
        }
        None => warn!("{serial}: can't reach the phone's listener over the cable"),
    }
    let mut mirrored = 0;
    for port in PAYLOAD_PORTS {
        let (phone, pc) = (format!("tcp:{}", port + PAYLOAD_SHIFT), format!("tcp:{port}"));
        if run(adb, serial, &["reverse", &phone, &pc]).await.is_some() {
            mirrored += 1;
        }
    }
    let mut forwarded = 0;
    for port in PAYLOAD_PORTS {
        let (pc, phone) = (format!("tcp:{}", port + FORWARD_SHIFT), format!("tcp:{port}"));
        if run(adb, serial, &["forward", &pc, &phone]).await.is_some() {
            forwarded += 1;
        }
    }
    info!("{serial}: payload ports mirrored over USB ({mirrored} to the phone, {forwarded} from it)");
    for pkg in installed {
        if has_grants(adb, serial, pkg).await {
            continue;
        }
        let digest = match signing_digest(adb, serial, pkg).await {
            Ok(d) => d,
            Err(e) => {
                warn!("{serial}: no clipboard grants for {pkg}: {e:#}");
                continue;
            }
        };
        if !allowed_apps().contains(&digest) {
            ask_to_allow(adb.clone(), serial.to_string(), pkg, digest);
            continue;
        }
        grant(adb, serial, pkg).await;
    }
}

/// Both are idempotent; they only ever touch this package's own permissions.
async fn grant(adb: &PathBuf, serial: &str, pkg: &str) {
    let logs = run(adb, serial, &["shell", "pm", "grant", pkg, "android.permission.READ_LOGS"]).await;
    let overlay = run(adb, serial, &["shell", "appops", "set", pkg, "SYSTEM_ALERT_WINDOW", "allow"]).await;
    if logs.is_some() && overlay.is_some() {
        info!("{serial}: background clipboard sync allowed for {pkg}");
    }
}

/// Nothing to ask or fetch when it already has them (the usual case: they last until uninstalled).
async fn has_grants(adb: &PathBuf, serial: &str, pkg: &str) -> bool {
    let logs = run(adb, serial, &["shell", "dumpsys", "package", pkg]).await;
    let overlay = run(adb, serial, &["shell", "appops", "get", pkg, "SYSTEM_ALERT_WINDOW"]).await;
    logs.is_some_and(|o| o.contains("android.permission.READ_LOGS: granted=true")) && overlay.is_some_and(|o| o.contains("allow"))
}

/// Answered "Not now" since the companion started: don't ask at every plug-in.
static DECLINED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
/// A question already on screen.
static ASKING: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

fn allowed_apps_path() -> PathBuf {
    crate::identity::dir().join("allowed-apps")
}

fn allowed_apps() -> Vec<String> {
    std::fs::read_to_string(allowed_apps_path()).unwrap_or_default().lines().map(str::to_string).collect()
}

/// Runs on its own: notify-send waits for the click.
fn ask_to_allow(adb: PathBuf, serial: String, pkg: &'static str, digest: String) {
    if DECLINED.lock().unwrap().contains(&digest) {
        return;
    }
    {
        let mut asking = ASKING.lock().unwrap();
        if asking.contains(&digest) {
            return;
        }
        asking.push(digest.clone());
    }
    tokio::spawn(async move {
        let model = run(&adb, &serial, &["shell", "getprop", "ro.product.model"]).await.unwrap_or_else(|| "this phone".to_string());
        let body = format!(
            "Background clipboard sync needs log access and drawing over other apps, which only a computer can grant. \
             Allow it only if you installed this app yourself. Signing key {}.",
            digest[..8].to_uppercase(),
        );
        let answer = tokio::time::timeout(
            ASK_WAIT,
            tokio::process::Command::new("notify-send")
                .args(["--app-name=Vyoma", "--icon=dialog-password", "--action=allow=Allow", "--action=no=Not now"])
                .arg(format!("Allow Vyoma on {model}?"))
                .arg(body)
                .kill_on_drop(true)
                .output(),
        )
        .await;
        let answer = match answer {
            Ok(Ok(out)) => String::from_utf8_lossy(&out.stdout).trim().to_string(),
            _ => String::new(), // unanswered: asks again at the next plug-in
        };
        match answer.as_str() {
            "allow" => {
                let saved = std::fs::create_dir_all(crate::identity::dir())
                    .and_then(|()| std::fs::write(allowed_apps_path(), allowed_apps().into_iter().chain([digest.clone()]).collect::<Vec<_>>().join("\n") + "\n"));
                if let Err(e) = saved {
                    warn!("couldn't remember the allowed app: {e}");
                }
                grant(&adb, &serial, pkg).await;
            }
            "no" => DECLINED.lock().unwrap().push(digest.clone()),
            _ => {}
        }
        ASKING.lock().unwrap().retain(|d| *d != digest);
    });
}

/// SHA-256 of the signing certificate of the APK installed as `pkg`. `dumpsys` only gives a 32-bit
/// hash of it, which can be forged, so this reads the real one out of the APK.
async fn signing_digest(adb: &PathBuf, serial: &str, pkg: &str) -> Result<String> {
    let paths = run(adb, serial, &["shell", "pm", "path", pkg]).await.context("can't find the app")?;
    let apk = paths.lines().find_map(|l| l.strip_prefix("package:")).filter(|p| p.ends_with("/base.apk")).context("can't find the app's APK")?;
    // Private to this user (XDG_RUNTIME_DIR is 0700), and deleted as soon as it's read.
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    // One per phone: two set up at once mustn't judge one's app by the other's file.
    let tag: String = serial.chars().filter(char::is_ascii_alphanumeric).collect();
    let local = dir.join(format!("vyoma-check-{}-{tag}.apk", std::process::id()));
    let local_str = local.to_string_lossy().into_owned();
    let pulled = tokio::time::timeout(
        PULL_WAIT,
        tokio::process::Command::new(adb).args(["-s", serial, "pull", apk, &local_str]).kill_on_drop(true).output(),
    )
    .await;
    let read = tokio::fs::read(&local).await;
    let _ = tokio::fs::remove_file(&local).await;
    pulled.context("fetching the APK took too long")??;
    let bytes = read.context("couldn't fetch the APK")?;
    Ok(hex::encode(Sha256::digest(signing_certificate(&bytes)?)))
}

/// The one certificate that signed this APK, from its APK Signing Block. Android checked the block
/// when installing, so it can't be someone else's key with a valid signature. But it skips signers
/// meant for other Android versions, and a block could name our key in one of those; so every
/// signer in every scheme has to agree.
fn signing_certificate(apk: &[u8]) -> Result<&[u8]> {
    const EOCD: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];
    const MAGIC: &[u8] = b"APK Sig Block 42";
    // v2, v3 and v3.1. Each is a list of signers shaped alike up to the certificates.
    const SCHEMES: [u32; 3] = [0x7109_871a, 0xf053_68c0, 0x1b93_ad49];
    let last = apk.len().checked_sub(22).context("not an APK")?;
    let eocd = (last.saturating_sub(0xffff)..=last).rev().find(|&i| apk[i..].starts_with(&EOCD)).context("not an APK")?;
    let directory = u32::from_le_bytes(apk[eocd + 16..eocd + 20].try_into()?) as usize;
    let tail = directory.checked_sub(24).filter(|&t| t + 24 <= apk.len()).context("APK has no signing block")?;
    if &apk[tail + 8..directory] != MAGIC {
        bail!("APK has no signing block");
    }
    let size = u64::from_le_bytes(apk[tail..tail + 8].try_into()?) as usize;
    let start = directory.checked_sub(size).and_then(|s| s.checked_sub(8)).context("bad signing block")?;
    let mut pairs = apk.get(start + 8..tail).context("bad signing block")?;
    let mut found: Option<&[u8]> = None;
    while !pairs.is_empty() {
        let len = u64::from_le_bytes(pairs.get(..8).context("bad signing block")?.try_into()?) as usize;
        let pair = pairs.get(8..).and_then(|p| p.get(..len)).context("bad signing block")?;
        pairs = &pairs[8 + len..];
        let id = u32::from_le_bytes(pair.get(..4).context("bad signing block")?.try_into()?);
        if !SCHEMES.contains(&id) {
            continue;
        }
        let mut value = &pair[4..];
        let mut signers = prefixed(&mut value)?;
        while !signers.is_empty() {
            let mut signer = prefixed(&mut signers)?;
            let mut signed = prefixed(&mut signer)?;
            prefixed(&mut signed)?; // digests
            let mut certificates = prefixed(&mut signed)?;
            let certificate = prefixed(&mut certificates)?;
            if found.is_some_and(|f| f != certificate) {
                bail!("APK names more than one signer");
            }
            found = Some(certificate);
        }
    }
    found.context("APK has no v2 or v3 signature")
}

/// Takes one u32-length-prefixed piece off the front.
fn prefixed<'a>(input: &mut &'a [u8]) -> Result<&'a [u8]> {
    let len = u32::from_le_bytes(input.get(..4).context("bad signing block")?.try_into()?) as usize;
    let piece = input[4..].get(..len).context("bad signing block")?;
    *input = &input[4 + len..];
    Ok(piece)
}

async fn list_devices(adb: &PathBuf) -> Vec<(String, String)> {
    let listing = tokio::process::Command::new(adb).arg("devices").kill_on_drop(true).output();
    let Ok(Ok(out)) = tokio::time::timeout(Duration::from_secs(15), listing).await else { return Vec::new() };
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

#[cfg(test)]
mod tests {
    use super::*;

    fn lp(piece: &[u8]) -> Vec<u8> {
        [&(piece.len() as u32).to_le_bytes()[..], piece].concat()
    }

    /// One scheme's value: a list of signers, each with the certificate given.
    fn scheme(certs: &[&[u8]]) -> Vec<u8> {
        let signers: Vec<u8> = certs
            .iter()
            .flat_map(|c| {
                let signed = [lp(b"digests"), lp(&lp(c)), lp(b"attributes")].concat();
                lp(&[lp(&signed), lp(b"signatures"), lp(b"key")].concat())
            })
            .collect();
        lp(&signers)
    }

    fn apk(schemes: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let pairs: Vec<u8> = schemes
            .iter()
            .flat_map(|(id, value)| [&((value.len() + 4) as u64).to_le_bytes()[..], &id.to_le_bytes()[..], value].concat())
            .collect();
        let size = (pairs.len() + 24) as u64;
        let mut out = b"zip entries".to_vec();
        out.extend(size.to_le_bytes());
        out.extend(pairs);
        out.extend(size.to_le_bytes());
        out.extend(b"APK Sig Block 42");
        let directory = out.len() as u32;
        out.extend(b"central directory");
        out.extend([0x50, 0x4b, 0x05, 0x06]);
        out.extend([0; 12]);
        out.extend(directory.to_le_bytes());
        out.extend([0; 2]);
        out
    }

    const V2: u32 = 0x7109_871a;
    const V3: u32 = 0xf053_68c0;

    #[test]
    fn reads_the_signing_certificate() {
        let both = apk(&[(0x4242, b"other".to_vec()), (V2, scheme(&[b"mine"])), (V3, scheme(&[b"mine"]))]);
        assert_eq!(signing_certificate(&both).unwrap(), b"mine");
    }

    #[test]
    fn a_signer_for_another_android_version_cant_hide_the_real_one() {
        assert!(signing_certificate(&apk(&[(V3, scheme(&[b"mine", b"theirs"]))])).is_err());
        assert!(signing_certificate(&apk(&[(V2, scheme(&[b"theirs"])), (V3, scheme(&[b"mine"]))])).is_err());
    }

    #[test]
    fn refuses_what_isnt_signed_or_is_cut_short() {
        assert!(signing_certificate(&apk(&[(0x4242, b"other".to_vec())])).is_err());
        assert!(signing_certificate(b"short").is_err());
        let whole = apk(&[(V2, scheme(&[b"mine"]))]);
        for cut in 0..whole.len() {
            let _ = signing_certificate(&whole[..cut]); // must not panic
        }
        let mut lying = whole.clone();
        let at = whole.windows(4).position(|w| w == V2.to_le_bytes()).unwrap() + 4;
        lying[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(signing_certificate(&lying).is_err());
    }

    /// `VYOMA_APK=x.apk VYOMA_APK_DIGEST=<apksigner's SHA-256> cargo test real_apk -- --ignored`
    #[test]
    #[ignore]
    fn real_apk() {
        let bytes = std::fs::read(std::env::var("VYOMA_APK").unwrap()).unwrap();
        assert_eq!(hex::encode(Sha256::digest(signing_certificate(&bytes).unwrap())), std::env::var("VYOMA_APK_DIGEST").unwrap());
    }
}
