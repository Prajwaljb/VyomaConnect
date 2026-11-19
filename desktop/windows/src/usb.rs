//! The USB cable (core's [vyoma_core::adb]): plug the phone in with USB debugging on and it links
//! over the cable through adb's tunnel, steadier than Wi-Fi, and the phone's app can get the one-time
//! grants for background clipboard sync once the person here allows its signing key.
//!
//! Windows says which phones adb can reach: each shows Android's adb device interface (its USB
//! driver's, Google's or the phone maker's), listed here every couple of seconds through cfgmgr32
//! ([crate::win::device_interfaces]);
//! no adb server runs until one appears. The question goes as a toast with Allow and Not now (a
//! dialog where there are no toasts). adb comes from Android Studio or PATH, or This PC's USB cable
//! check fetches Google's platform tools into Vyoma's folder (Get adb).
//! Built and tested on CI and under Wine only; not yet run on a real Windows PC.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tokio::sync::oneshot;
use windows_sys::core::GUID;

use crate::toast::{self, Button, Shown, Toast};

/// The interface class adb looks for (AdbWinApi's ANDROID_USB_CLASS_ID).
const ADB_INTERFACE: GUID = GUID::from_u128(0xF72FE0D4_CBCB_407D_8814_9ED673D0DD6B);
const LOOK_EVERY: Duration = Duration::from_secs(2);
const TOOLS_URL: &str = "https://dl.google.com/android/repository/platform-tools-latest-windows.zip";
const FETCH_WAIT: Duration = Duration::from_secs(300);
/// This PC's check: the window's This PC page, and its fix (FixCheck: Get adb).
pub const CHECK: &str = "USB cable";
/// The toasts' tag.
const TAG: &str = "cable";

static PLUGGED: OnceLock<tokio::sync::watch::Sender<HashSet<String>>> = OnceLock::new();
/// Questions waiting for a toast's button, by token.
static WAITING: Mutex<Option<HashMap<String, oneshot::Sender<bool>>>> = Mutex::new(None);

/// Starts watching for phones (once); the cable leads to this PC's own listener on [port].
pub fn start(port: u16) {
    vyoma_core::adb::set_port(port);
    vyoma_core::adb::set_asker(Box::new(|q| Box::pin(ask(q))));
    let (tx, rx) = tokio::sync::watch::channel(HashSet::new());
    if PLUGGED.set(tx).is_err() {
        return;
    }
    if let Err(e) = std::thread::Builder::new().name("vyoma-usb".into()).spawn(look) {
        tracing::warn!("no USB watch: {e}");
    }
    tokio::spawn(vyoma_core::adb::watch(rx));
}

/// Watches again (adb just arrived: the first watch gave up without it).
fn rewatch() {
    if let Some(tx) = PLUGGED.get() {
        let mut rx = tx.subscribe();
        rx.mark_changed();
        tokio::spawn(vyoma_core::adb::watch(rx));
    }
}

/// How many phones adb can reach over USB now.
pub fn plugged() -> usize {
    PLUGGED.get().map_or(0, |tx| tx.borrow().len())
}

fn look() {
    loop {
        let now: HashSet<String> = interfaces().into_iter().collect();
        if let Some(tx) = PLUGGED.get() {
            tx.send_if_modified(|known| std::mem::replace(known, now.clone()) != *known);
        }
        std::thread::sleep(LOOK_EVERY);
    }
}

/// The adb interfaces present now, by their device paths.
pub fn interfaces() -> Vec<String> {
    crate::win::device_interfaces(&ADB_INTERFACE)
}

/// Asks the person here whether a phone's app may have the grants.
async fn ask(q: vyoma_core::adb::Question) -> Option<bool> {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let token = NEXT.fetch_add(1, Ordering::Relaxed).to_string();
    let (tx, rx) = oneshot::channel();
    WAITING.lock().unwrap().get_or_insert_default().insert(token.clone(), tx);
    let button = |label: &str, yes: &str| Button { label: label.into(), args: toast::args(&[("do", TAG), ("t", &token), ("yes", yes)]), with_reply: false };
    let t = Toast {
        tag: TAG.into(),
        title: q.title.clone(),
        body: q.body.clone(),
        launch: toast::args(&[("do", "open")]),
        buttons: vec![button("Allow", "1"), button("Not now", "0")],
        ..Default::default()
    };
    let answer = if toast::show(&t) == Shown::Yes {
        tokio::time::timeout(vyoma_core::adb::ASK_WAIT, rx).await.ok().and_then(Result::ok)
    } else {
        WAITING.lock().unwrap().get_or_insert_default().remove(&token);
        let (title, body) = (q.title, q.body);
        tokio::task::spawn_blocking(move || crate::win::ask(&title, &body)).await.ok()
    };
    WAITING.lock().unwrap().get_or_insert_default().remove(&token);
    toast::remove(TAG, "");
    answer
}

/// A toast's Allow or Not now.
pub fn answered(token: &str, yes: bool) {
    if let Some(tx) = WAITING.lock().unwrap().get_or_insert_default().remove(token) {
        let _ = tx.send(yes);
    }
}

/// The USB cable check for This PC.
pub fn check() -> serde_json::Value {
    let adb = vyoma_core::adb::find_adb().is_some();
    let detail = match (adb, plugged()) {
        (true, 0) => "Plug the phone in with USB debugging on, and they link over the cable",
        (true, _) => "A phone is plugged in",
        (false, _) => "Linking over the cable needs adb, Google's Android platform tools. Get adb downloads them from Google into Vyoma's folder",
    };
    serde_json::json!({ "name": CHECK, "state": if adb { "ok" } else { "warn" }, "detail": detail, "action": if adb { "" } else { "Get adb" } })
}

/// Get adb: Google's platform tools into Vyoma's folder, then watching again.
pub async fn fetch_adb() -> Result<()> {
    if vyoma_core::adb::find_adb().is_some() {
        rewatch();
        return Ok(());
    }
    let dir = vyoma_core::identity::dir();
    std::fs::create_dir_all(&dir)?;
    let zip = dir.join("platform-tools.zip");
    // Windows' own curl and tar (bsdtar, which reads zips), in System32 since Windows 10 1803.
    let fetched = tokio::time::timeout(FETCH_WAIT, quiet("curl.exe").args(["-fsSL", "--proto", "=https", "-o"]).arg(&zip).arg(TOOLS_URL).status())
        .await
        .context("the download took too long")?
        .context("Windows has no curl")?;
    if !fetched.success() {
        let _ = std::fs::remove_file(&zip);
        bail!("Couldn't download Google's platform tools. Check the internet connection");
    }
    let tools = vyoma_core::adb::own_tools();
    let _ = std::fs::remove_dir_all(&tools);
    let unpacked = unpack(&zip, &dir).await;
    let _ = std::fs::remove_file(&zip);
    unpacked?;
    if !tools.join("adb.exe").is_file() {
        bail!("The download wasn't Google's platform tools");
    }
    tracing::info!("adb is in place");
    rewatch();
    Ok(())
}

async fn unpack(zip: &Path, into: &Path) -> Result<()> {
    let done = quiet("tar.exe").arg("-xf").arg(zip).arg("-C").arg(into).status().await.context("Windows has no tar")?;
    if !done.success() {
        bail!("Couldn't unpack Google's platform tools");
    }
    Ok(())
}

/// A program without a console window.
pub(crate) fn quiet(program: &str) -> tokio::process::Command {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut c = tokio::process::Command::new(program);
    c.creation_flags(CREATE_NO_WINDOW).kill_on_drop(true);
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_no_phone_without_one() {
        // Wine (and CI) have no phone on USB.
        println!("adb interfaces: {}", interfaces().len());
        let c = check();
        assert_eq!(c["name"], CHECK);
        assert!(c["state"] == "ok" || c["action"] == "Get adb", "{c}");
    }

    #[tokio::test]
    async fn a_toasts_answer_reaches_its_question() {
        let (tx, rx) = oneshot::channel();
        WAITING.lock().unwrap().get_or_insert_default().insert("7".into(), tx);
        answered("8", false);
        answered("7", true);
        assert_eq!(rx.await.ok(), Some(true));
        answered("7", false); // a second press: nothing waits
    }
}
