//! This PC's controls on the phone (`vyoma.pc`): each display's brightness and each app's volume,
//! as the Linux app reports them: `{displays:[{id,label,internal,value}], apps:[{id,name,binary,
//! title,volume,muted}]}`, values as fractions.
//!
//! A laptop's own screen through WMI (`WmiMonitorBrightness`, what Windows' own slider uses);
//! other displays over DDC/CI (the monitor's own brightness, for those that take it over the
//! cable). Apps are the audio sessions on the default speaker, as Windows' volume mixer lists them
//! (one per program, by its process id), named as the mixer names them.
//! Built and tested on CI and under Wine only; not yet run on a real Windows PC.

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use windows::Win32::Devices::Display::{DestroyPhysicalMonitors, GetMonitorBrightness, GetNumberOfPhysicalMonitorsFromHMONITOR, GetPhysicalMonitorsFromHMONITOR, PHYSICAL_MONITOR, SetMonitorBrightness};
use windows::Win32::Foundation::{CloseHandle, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, HDC, HMONITOR};
use windows::Win32::Media::Audio::{AudioSessionStateExpired, IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator, eMultimedia, eRender};
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::System::Com::{CLSCTX_ALL, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE, CoSetProxyBlanket};
use windows::Win32::System::Rpc::{RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::System::Wmi::{IWbemClassObject, IWbemLocator, IWbemServices, WBEM_FLAG_FORWARD_ONLY, WBEM_FLAG_RETURN_IMMEDIATELY, WBEM_GENERIC_FLAG_TYPE, WBEM_INFINITE, WbemLocator};
use windows::core::{BOOL, BSTR, HSTRING, Interface, PCWSTR, PWSTR, w};

pub const REQUEST: &str = "vyoma.pc.request";
pub const SET: &str = "vyoma.pc.set";
pub const STATE: &str = "vyoma.pc";

/// The laptop's own screen.
const INTERNAL: &str = "internal";
/// Other displays, by their place in Windows' list: `ddc:0`, `ddc:1`...
const DDC: &str = "ddc:";

fn com() {
    // SAFETY: the calling thread joins the multithreaded apartment (again is fine).
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
}

/// Everything the phone's PC controls show (blocking: run off the async threads). A part this PC
/// can't report is just empty.
pub fn state() -> Value {
    com();
    let mut displays = Vec::new();
    if let Some(value) = internal_brightness() {
        displays.push(json!({ "id": INTERNAL, "label": "Built-in display", "internal": true, "value": value }));
    }
    displays.extend(external_displays());
    json!({ "displays": displays, "apps": apps().unwrap_or_default() })
}

/// One change from a `vyoma.pc.set` body: `{display, brightness}` or `{app, volume?, muted?}`.
pub fn set(body: &Value) -> Result<()> {
    com();
    if let (Some(display), Some(level)) = (body["display"].as_str(), body["brightness"].as_f64()) {
        let level = level.clamp(0.0, 1.0);
        if display == INTERNAL {
            return set_internal(level);
        }
        let index = display.strip_prefix(DDC).and_then(|n| n.parse().ok()).context("no such display")?;
        return set_external(index, level);
    }
    if let Some(pid) = body["app"].as_u64() {
        return set_app(pid as u32, body["volume"].as_f64(), body["muted"].as_bool());
    }
    bail!("nothing to change")
}

/// A burst of sets for the same slider (the phone sends while dragging): only the newest counts.
pub fn superseded(batch: &[Value], i: usize) -> bool {
    let target = |b: &Value| (b["display"].clone(), b["app"].clone(), b["volume"].is_null(), b["muted"].is_null());
    batch[i]["type"] == SET && batch[i + 1..].iter().any(|q| q["type"] == SET && target(&q["body"]) == target(&batch[i]["body"]))
}

// ---- The laptop's screen, through WMI

fn wmi() -> Result<IWbemServices> {
    // SAFETY: WMI's locator and the ROOT\WMI namespace, with the call-level security WMI asks of
    // its clients.
    unsafe {
        let locator: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER)?;
        let services = locator.ConnectServer(&BSTR::from(r"ROOT\WMI"), &BSTR::new(), &BSTR::new(), &BSTR::new(), 0, &BSTR::new(), None)?;
        CoSetProxyBlanket(&services, RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE, PCWSTR::null(), RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE, None, EOAC_NONE)?;
        Ok(services)
    }
}

fn first(services: &IWbemServices, query: &str) -> Result<Option<IWbemClassObject>> {
    // SAFETY: one row of a forward-only query.
    unsafe {
        let rows = services.ExecQuery(&BSTR::from("WQL"), &BSTR::from(query), WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY, None)?;
        let mut row = [None];
        let mut got = 0u32;
        rows.Next(WBEM_INFINITE, &mut row, &mut got).ok()?;
        Ok(if got == 0 { None } else { row[0].take() })
    }
}

fn get(object: &IWbemClassObject, name: PCWSTR) -> Result<VARIANT> {
    let mut v = VARIANT::default();
    // SAFETY: reading one property into a VARIANT we own.
    unsafe { object.Get(name, 0, &mut v, None, None)? };
    Ok(v)
}

/// The built-in screen's brightness; None where there's none (a desktop PC, or Wine).
fn internal_brightness() -> Option<f64> {
    let services = wmi().ok()?;
    let row = first(&services, "SELECT CurrentBrightness FROM WmiMonitorBrightness WHERE Active=TRUE").ok()??;
    let level = i32::try_from(&get(&row, w!("CurrentBrightness")).ok()?).ok()?;
    Some((level.clamp(0, 100) as f64) / 100.0)
}

fn set_internal(level: f64) -> Result<()> {
    let services = wmi()?;
    let target = first(&services, "SELECT * FROM WmiMonitorBrightnessMethods WHERE Active=TRUE")?.context("no built-in display")?;
    let path = BSTR::try_from(&get(&target, w!("__PATH"))?)?;
    // SAFETY: WmiSetBrightness's parameters made from its class's own signature, then the call.
    unsafe {
        let mut class = None;
        services.GetObject(&BSTR::from("WmiMonitorBrightnessMethods"), WBEM_GENERIC_FLAG_TYPE(0), None, Some(&mut class), None)?;
        let (mut input, mut output) = (None, None);
        class.context("no brightness class")?.GetMethod(w!("WmiSetBrightness"), 0, &mut input, &mut output)?;
        let input = input.context("no brightness method")?.SpawnInstance(0)?;
        input.Put(w!("Timeout"), 0, &VARIANT::from(0i32), 0)?;
        input.Put(w!("Brightness"), 0, &VARIANT::from((level * 100.0).round() as u8), 0)?;
        services.ExecMethod(&path, &BSTR::from("WmiSetBrightness"), WBEM_GENERIC_FLAG_TYPE(0), None, &input, None, None)?;
    }
    Ok(())
}

// ---- Other displays, over DDC/CI

unsafe extern "system" fn add_monitor(monitor: HMONITOR, _: HDC, _: *mut RECT, list: LPARAM) -> BOOL {
    // SAFETY: [list] is the Vec passed to EnumDisplayMonitors below, alive for the call.
    unsafe { (*(list.0 as *mut Vec<HMONITOR>)).push(monitor) };
    BOOL(1)
}

/// Each physical display with what DDC/CI says of its brightness (min, current, max): those that
/// don't answer (laptop panels, monitors with DDC/CI off) are left out. [f] sees them in Windows'
/// order, numbered as the phone knows them; the handles are let go after.
fn with_monitors<T>(mut f: impl FnMut(usize, &PHYSICAL_MONITOR, (u32, u32, u32)) -> Option<T>) -> Option<T> {
    let mut monitors: Vec<HMONITOR> = Vec::new();
    // SAFETY: the callback only pushes into [monitors]; each display's physical monitors are
    // destroyed after they're used.
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(add_monitor), LPARAM(&mut monitors as *mut _ as isize));
        let mut index = 0;
        for m in monitors {
            let mut count = 0u32;
            if GetNumberOfPhysicalMonitorsFromHMONITOR(m, &mut count).is_err() || count == 0 {
                continue;
            }
            let mut physical = vec![PHYSICAL_MONITOR::default(); count as usize];
            if GetPhysicalMonitorsFromHMONITOR(m, &mut physical).is_err() {
                continue;
            }
            let mut found = None;
            for p in &physical {
                let (mut min, mut cur, mut max) = (0u32, 0u32, 0u32);
                if GetMonitorBrightness(p.hPhysicalMonitor, &mut min, &mut cur, &mut max) == 0 || max <= min {
                    continue;
                }
                if found.is_none() {
                    found = f(index, p, (min, cur, max));
                }
                index += 1;
            }
            let _ = DestroyPhysicalMonitors(&physical);
            if found.is_some() {
                return found;
            }
        }
        None
    }
}

/// A display's name as DDC/CI's description gives it; Windows' stand-in name becomes a number.
fn monitor_label(description: &str, index: usize) -> String {
    let d = description.trim();
    if d.is_empty() || d.eq_ignore_ascii_case("Generic PnP Monitor") || d.eq_ignore_ascii_case("Generic Non-PnP Monitor") {
        format!("Display {}", index + 1)
    } else {
        d.to_owned()
    }
}

fn external_displays() -> Vec<Value> {
    let mut out = Vec::new();
    with_monitors::<()>(|index, p, (min, cur, max)| {
        // A packed struct: its name copied out before it's read.
        let name = p.szPhysicalMonitorDescription;
        let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
        let description = String::from_utf16_lossy(&name[..len]);
        let value = (cur.clamp(min, max) - min) as f64 / (max - min) as f64;
        out.push(json!({ "id": format!("{DDC}{index}"), "label": monitor_label(&description, index), "internal": false, "value": value }));
        None
    });
    out
}

fn set_external(index: usize, level: f64) -> Result<()> {
    let done = with_monitors(|i, p, (min, _, max)| {
        (i == index).then(|| {
            let value = min + ((max - min) as f64 * level).round() as u32;
            // SAFETY: a physical monitor handle, alive inside with_monitors.
            unsafe { SetMonitorBrightness(p.hPhysicalMonitor, value) != 0 }
        })
    });
    match done {
        Some(true) => Ok(()),
        Some(false) => bail!("the display didn't take that brightness"),
        None => bail!("that display is gone"),
    }
}

// ---- Each app's volume, through Core Audio's sessions

/// The live sessions on the default speaker, except Windows' own sounds, with their process id.
fn sessions() -> Result<Vec<(u32, IAudioSessionControl2, ISimpleAudioVolume)>> {
    // SAFETY: COM calls on this thread (joined in [com]); everything here is owned.
    unsafe {
        let devices: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = devices.GetDefaultAudioEndpoint(eRender, eMultimedia)?;
        let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None)?;
        let list = manager.GetSessionEnumerator()?;
        let mut out = Vec::new();
        for i in 0..list.GetCount()? {
            let Ok(control) = list.GetSession(i) else { continue };
            if control.GetState().is_ok_and(|s| s == AudioSessionStateExpired) {
                continue;
            }
            let Ok(control) = control.cast::<IAudioSessionControl2>() else { continue };
            // S_OK means it is the system sounds session.
            if control.IsSystemSoundsSession().0 == 0 {
                continue;
            }
            let (Ok(pid), Ok(volume)) = (control.GetProcessId(), control.cast::<ISimpleAudioVolume>()) else { continue };
            if pid != 0 {
                out.push((pid, control, volume));
            }
        }
        Ok(out)
    }
}

fn exe_path(pid: u32) -> Option<String> {
    // SAFETY: the process opened only to read its image name, closed after.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let named = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(process);
        named.ok()?;
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }
}

/// A program's own name for itself (its version resource's FileDescription: "Google Chrome").
fn description(path: &str) -> Option<String> {
    let path = HSTRING::from(path);
    // SAFETY: the version resource read into our buffer; VerQueryValueW's pointers point into it,
    // read within the lengths it gives.
    unsafe {
        let size = GetFileVersionInfoSizeW(&path, None);
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        GetFileVersionInfoW(&path, None, size, data.as_mut_ptr().cast()).ok()?;
        let (mut p, mut len) = (std::ptr::null_mut(), 0u32);
        if !VerQueryValueW(data.as_ptr().cast(), w!("\\VarFileInfo\\Translation"), &mut p, &mut len).as_bool() || len < 4 {
            return None;
        }
        let pair = std::slice::from_raw_parts(p as *const u16, 2);
        let key = HSTRING::from(format!("\\StringFileInfo\\{:04x}{:04x}\\FileDescription", pair[0], pair[1]));
        if !VerQueryValueW(data.as_ptr().cast(), &key, &mut p, &mut len).as_bool() || len == 0 {
            return None;
        }
        let text = String::from_utf16_lossy(std::slice::from_raw_parts(p as *const u16, len as usize));
        Some(text.trim_end_matches('\0').trim().to_owned()).filter(|t| !t.is_empty())
    }
}

/// What the mixer calls an app: the name its session gives (not a resource reference), else its
/// program's description, else its file's name.
pub fn app_label(session: &str, description: Option<&str>, binary: &str) -> String {
    let session = session.trim();
    if !session.is_empty() && !session.starts_with('@') {
        return session.to_owned();
    }
    if let Some(d) = description.map(str::trim).filter(|d| !d.is_empty()) {
        return d.to_owned();
    }
    let stem = binary.rsplit_once('.').map_or(binary, |(s, _)| s);
    let mut chars = stem.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_else(|| "App".into())
}

fn apps() -> Result<Vec<Value>> {
    let mut out: Vec<Value> = Vec::new();
    for (pid, control, volume) in sessions()? {
        if out.iter().any(|a| a["id"] == pid) {
            continue;
        }
        // SAFETY: the session's name (freed after it's copied) and its volume.
        let (session, level, muted) = unsafe {
            let name = control.GetDisplayName().ok().map(|p| {
                let s = p.to_string().unwrap_or_default();
                CoTaskMemFree(Some(p.0.cast()));
                s
            });
            (name.unwrap_or_default(), volume.GetMasterVolume().unwrap_or(1.0), volume.GetMute().map(|m| m.as_bool()).unwrap_or(false))
        };
        let path = exe_path(pid).unwrap_or_default();
        let binary = path.rsplit(['\\', '/']).next().unwrap_or_default().to_owned();
        let name = app_label(&session, description(&path).as_deref(), &binary);
        out.push(json!({ "id": pid, "name": name, "binary": binary, "title": null, "volume": level as f64, "muted": muted }));
    }
    Ok(out)
}

fn set_app(pid: u32, level: Option<f64>, muted: Option<bool>) -> Result<()> {
    let mine: Vec<ISimpleAudioVolume> = sessions()?.into_iter().filter(|(p, _, _)| *p == pid).map(|(_, _, v)| v).collect();
    if mine.is_empty() {
        bail!("that app isn't playing sound any more");
    }
    for volume in mine {
        // SAFETY: the session's own volume control.
        unsafe {
            if let Some(l) = level {
                volume.SetMasterVolume(l.clamp(0.0, 1.0) as f32, std::ptr::null())?;
            }
            if let Some(m) = muted {
                volume.SetMute(m, std::ptr::null())?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apps_are_named_as_the_mixer_names_them() {
        assert_eq!(app_label("Spotify", Some("Spotify"), "Spotify.exe"), "Spotify");
        assert_eq!(app_label("@%SystemRoot%\\System32\\AudioSrv.Dll,-202", Some("Google Chrome"), "chrome.exe"), "Google Chrome");
        assert_eq!(app_label("", None, "vlc.exe"), "Vlc");
        assert_eq!(app_label("", Some("  "), ""), "App");
    }

    #[test]
    fn displays_without_a_name_get_a_number() {
        assert_eq!(monitor_label("Generic PnP Monitor", 0), "Display 1");
        assert_eq!(monitor_label("DELL U2720Q", 1), "DELL U2720Q");
    }

    #[test]
    fn a_dragged_slider_counts_once() {
        let set = |display: &str, b: f64| json!({ "type": SET, "body": { "display": display, "brightness": b } });
        let batch = [set("ddc:0", 0.1), set("ddc:1", 0.5), set("ddc:0", 0.3)];
        assert!(superseded(&batch, 0));
        assert!(!superseded(&batch, 1));
        assert!(!superseded(&batch, 2));
    }

    #[test]
    fn reads_what_there_is() {
        // Under Wine: no brightness, maybe no sessions; never changed here (Wine's sound can reach
        // the real speaker).
        let s = state();
        assert!(s["displays"].is_array() && s["apps"].is_array());
        println!("{} displays, {} apps", s["displays"].as_array().unwrap().len(), s["apps"].as_array().unwrap().len());
    }

    #[test]
    fn refuses_what_it_cant_change() {
        assert!(set(&json!({ "display": "ddc:99", "brightness": 0.5 })).is_err());
        assert!(set(&json!({ "nothing": true })).is_err());
    }
}
