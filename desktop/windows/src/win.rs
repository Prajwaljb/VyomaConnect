//! Windows' own calls the agent makes, each wrapped so the rest reads like the Mac's `mac.rs`:
//! the battery, the screen lock, the clipboard, Downloads, opening links, a yes/no question, the
//! computer's name, and finding a phone (a sound until it's stopped).
//! Built and tested on CI and under Wine only; not yet run on a real Windows PC.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Result, bail};
use windows_sys::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber, IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW,
    SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock};
use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows_sys::Win32::UI::Shell::{FOLDERID_Downloads, KF_FLAG_DEFAULT, SHGetKnownFolderPath, ShellExecuteW};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    IDYES, MB_ICONINFORMATION, MB_ICONQUESTION, MB_OK, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO, MessageBoxW, SW_SHOWNORMAL,
};

/// CF_UNICODETEXT.
const UNICODE_TEXT: u32 = 13;
/// A copy marked this way (KeePass, Bitwarden, 1Password...) is a password: it stays on this PC.
const PRIVATE_FORMATS: [&str; 2] = ["ExcludeClipboardContentFromMonitorProcessing", "Clipboard Viewer Ignore"];

/// [s] as Windows wants a string: UTF-16, NUL at the end.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// A NUL-terminated UTF-16 string Windows handed over.
///
/// # Safety
/// [p] is null or points at a NUL-terminated UTF-16 string.
pub unsafe fn from_wide(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: the caller's promise; the length stops at the NUL.
    unsafe {
        let len = (0..).take_while(|&i| *p.add(i) != 0).count();
        String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
    }
}

/// Where Vyoma keeps its identity, pairings and settings: %LOCALAPPDATA%\Vyoma (this PC's own:
/// a roaming profile mustn't carry the keys to another PC). `VYOMA_DIR` overrides it, for tests.
pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("VYOMA_DIR") {
        return PathBuf::from(d);
    }
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| home().join("AppData").join("Local"));
    base.join("Vyoma")
}

pub fn home() -> PathBuf {
    std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\"))
}

/// This PC as Settings > System > About names it.
pub fn name() -> String {
    std::env::var("COMPUTERNAME").ok().filter(|n| !n.is_empty()).unwrap_or_else(|| "Windows PC".into())
}

pub use vyoma_engine::stock::Battery;

fn power_status() -> Option<SYSTEM_POWER_STATUS> {
    // SAFETY: fills the struct we pass.
    unsafe {
        let mut s: SYSTEM_POWER_STATUS = std::mem::zeroed();
        (GetSystemPowerStatus(&mut s) != 0).then_some(s)
    }
}

/// The battery, if this PC has one: none on a desktop (flag 128) or when Windows can't tell.
pub fn battery() -> Option<Battery> {
    let s = power_status()?;
    if s.BatteryFlag == 128 || s.BatteryFlag == 255 || s.BatteryLifePercent > 100 {
        return None;
    }
    Some(Battery { charge: i64::from(s.BatteryLifePercent), charging: s.ACLineStatus == 1 })
}

/// On the charger (or a desktop's mains); None if Windows can't tell.
pub fn on_ac() -> Option<bool> {
    match power_status()?.ACLineStatus {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}

/// Locks the session, as Windows+L does.
/// Why the phone's Unlock does nothing here (Windows lets no app unlock it).
pub const CANT_UNLOCK: &str = "Windows doesn't let apps unlock it. Unlock it at the PC.";

pub fn lock() -> Result<()> {
    // SAFETY: no arguments; it only asks Windows to lock.
    if unsafe { windows_sys::Win32::System::Shutdown::LockWorkStation() } == 0 {
        bail!("Windows didn't lock: {}", std::io::Error::last_os_error());
    }
    Ok(())
}

// ---- clipboard ----------------------------------------------------------------------------------

/// Windows' count of clipboard changes: a new number, something was copied.
pub fn clipboard_changes() -> u32 {
    // SAFETY: no arguments.
    unsafe { GetClipboardSequenceNumber() }
}

/// Holds the clipboard open while it lives. Another program may have it for a moment: a few tries.
struct Open;

impl Open {
    fn new() -> Option<Self> {
        for _ in 0..10 {
            // SAFETY: no owner window; closed on drop.
            if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
                return Some(Open);
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        None
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        // SAFETY: opened in new().
        unsafe { CloseClipboard() };
    }
}

fn format(name: &str) -> u32 {
    // SAFETY: a NUL-terminated name; registering an existing one returns its number.
    unsafe { RegisterClipboardFormatW(wide(name).as_ptr()) }
}

/// The text on the clipboard; None for no text, or a password manager's copy.
pub fn clipboard() -> Option<String> {
    // SAFETY: checks with the clipboard open; the data is read under GlobalLock and copied out.
    unsafe {
        if IsClipboardFormatAvailable(UNICODE_TEXT) == 0 {
            return None;
        }
        let _open = Open::new()?;
        if PRIVATE_FORMATS.iter().any(|f| IsClipboardFormatAvailable(format(f)) != 0) {
            return None;
        }
        let h = GetClipboardData(UNICODE_TEXT);
        if h.is_null() {
            return None;
        }
        let p = GlobalLock(h as HGLOBAL) as *const u16;
        if p.is_null() {
            return None;
        }
        // Bounded by the block's size, should the text lack its NUL.
        let units = GlobalSize(h as HGLOBAL) / 2;
        let len = (0..units).take_while(|&i| *p.add(i) != 0).count();
        let text = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
        GlobalUnlock(h as HGLOBAL);
        Some(text)
    }
}

pub fn set_clipboard(text: &str) -> Result<()> {
    let units: Vec<u16> = wide(text);
    // SAFETY: the block is allocated, filled under GlobalLock, and owned by the clipboard once set.
    unsafe {
        let Some(_open) = Open::new() else { bail!("another program is holding the clipboard") };
        EmptyClipboard();
        let h = GlobalAlloc(GMEM_MOVEABLE, units.len() * 2);
        if h.is_null() {
            bail!("no memory for the clipboard");
        }
        let p = GlobalLock(h) as *mut u16;
        if p.is_null() {
            GlobalFree(h);
            bail!("no memory for the clipboard");
        }
        std::ptr::copy_nonoverlapping(units.as_ptr(), p, units.len());
        GlobalUnlock(h);
        if SetClipboardData(UNICODE_TEXT, h as HANDLE).is_null() {
            GlobalFree(h);
            bail!("Windows didn't take the text: {}", std::io::Error::last_os_error());
        }
    }
    Ok(())
}

// ---- places, links, questions -------------------------------------------------------------------

/// The Downloads folder, wherever the person moved it.
pub fn downloads() -> PathBuf {
    // SAFETY: the shell allocates the path; it's copied and freed with CoTaskMemFree.
    unsafe {
        let mut p: *mut u16 = std::ptr::null_mut();
        if SHGetKnownFolderPath(&FOLDERID_Downloads, KF_FLAG_DEFAULT as u32, std::ptr::null_mut(), &mut p) == 0 && !p.is_null() {
            let len = (0..).take_while(|&i| *p.add(i) != 0).count();
            let path = OsString::from_wide(std::slice::from_raw_parts(p, len));
            windows_sys::Win32::System::Com::CoTaskMemFree(p.cast());
            return PathBuf::from(path);
        }
        if !p.is_null() {
            windows_sys::Win32::System::Com::CoTaskMemFree(p.cast());
        }
    }
    home().join("Downloads")
}

/// Whether a link from the phone may be opened here: web pages only. Windows hands any other
/// scheme to whatever registered it (ms-msdt:, search-ms:, file:...), which a phone mustn't reach.
pub fn openable(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://")) && !url.chars().any(char::is_control)
}

/// Opens [target] (a web link the phone shared, or a folder or program of ours) as a double click would.
pub fn open(target: &str) {
    let (verb, what) = (wide("open"), wide(target));
    // SAFETY: NUL-terminated strings; ShellExecuteW copies what it needs.
    unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), what.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL) };
}

/// Asks yes or no in a dialog on top; blocks until answered (run it off the async threads).
pub fn ask(title: &str, text: &str) -> bool {
    let (t, m) = (wide(title), wide(text));
    // SAFETY: NUL-terminated strings, no owner window.
    unsafe { MessageBoxW(std::ptr::null_mut(), m.as_ptr(), t.as_ptr(), MB_YESNO | MB_ICONQUESTION | MB_SETFOREGROUND | MB_TOPMOST) == IDYES }
}

/// Says something in a dialog with OK; blocks until it's closed.
pub fn tell(title: &str, text: &str) {
    let (t, m) = (wide(title), wide(text));
    // SAFETY: as in ask.
    unsafe { MessageBoxW(std::ptr::null_mut(), m.as_ptr(), t.as_ptr(), MB_OK | MB_ICONINFORMATION | MB_SETFOREGROUND | MB_TOPMOST) };
}

// ---- find my PC ---------------------------------------------------------------------------------

/// The phone's "find my PC": Windows' alert sound over and over until Found it, or a minute.
#[derive(Default)]
pub struct Ringer(Option<tokio::task::JoinHandle<()>>);

const RING_FOR: Duration = Duration::from_secs(60);
const RING_EVERY: Duration = Duration::from_millis(1200);

impl Ringer {
    pub fn start(&mut self) {
        self.stop();
        self.0 = Some(tokio::spawn(async {
            let ring = async {
                loop {
                    // SAFETY: plays a system sound asynchronously; no pointers.
                    unsafe { windows_sys::Win32::System::Diagnostics::Debug::MessageBeep(MB_ICONQUESTION) };
                    tokio::time::sleep(RING_EVERY).await;
                }
            };
            let _ = tokio::time::timeout(RING_FOR, ring).await;
        }));
    }

    pub fn stop(&mut self) {
        if let Some(task) = self.0.take() {
            task.abort();
        }
    }
}

/// A registry hive: this user's, or the whole machine's (writing there needs an administrator).
#[derive(Clone, Copy)]
pub enum Hive {
    User,
    Machine,
}

impl Hive {
    fn key(self) -> windows_sys::Win32::System::Registry::HKEY {
        use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
        match self {
            Hive::User => HKEY_CURRENT_USER,
            Hive::Machine => HKEY_LOCAL_MACHINE,
        }
    }
}

/// Sets a text value under HKEY_CURRENT_USER, making the key if needed ([name] None: its default).
pub fn set_user_value(key: &str, name: Option<&str>, value: &str) -> anyhow::Result<()> {
    set_value(Hive::User, key, name, value)
}

/// Sets a text value in [hive], making the key if needed ([name] None: its default).
pub fn set_value(hive: Hive, key: &str, name: Option<&str>, value: &str) -> anyhow::Result<()> {
    use windows_sys::Win32::System::Registry::{REG_SZ, RegSetKeyValueW};
    let value = wide(value);
    let name = name.map(wide);
    // SAFETY: NUL-terminated key, name and REG_SZ data of exactly value's bytes.
    let err = unsafe {
        RegSetKeyValueW(hive.key(), wide(key).as_ptr(), name.as_ref().map_or(std::ptr::null(), |n| n.as_ptr()), REG_SZ, value.as_ptr().cast(), (value.len() * 2) as u32)
    };
    if err != 0 {
        anyhow::bail!("couldn't write {key} ({err})");
    }
    Ok(())
}

/// Reads a text value under HKEY_CURRENT_USER ([name] None: its default).
pub fn user_value(key: &str, name: Option<&str>) -> Option<String> {
    value(Hive::User, key, name)
}

/// Reads a text value in [hive] ([name] None: its default).
pub fn value(hive: Hive, key: &str, name: Option<&str>) -> Option<String> {
    use windows_sys::Win32::System::Registry::{RRF_RT_REG_SZ, RegGetValueW};
    let name = name.map(wide);
    let mut buf = vec![0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    // SAFETY: a buffer of [size] bytes; Windows writes a NUL-terminated string into it.
    let err = unsafe {
        RegGetValueW(hive.key(), wide(key).as_ptr(), name.as_ref().map_or(std::ptr::null(), |n| n.as_ptr()), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut size)
    };
    // SAFETY: as above, NUL-terminated on success.
    (err == 0).then(|| unsafe { from_wide(buf.as_ptr()) })
}

/// Deletes a key under HKEY_CURRENT_USER with everything in it; fine if it isn't there.
pub fn delete_user_key(key: &str) {
    delete_key(Hive::User, key);
}

/// Deletes a key in [hive] with everything in it; fine if it isn't there.
pub fn delete_key(hive: Hive, key: &str) {
    use windows_sys::Win32::System::Registry::RegDeleteTreeW;
    // SAFETY: a NUL-terminated path under the hive.
    unsafe { RegDeleteTreeW(hive.key(), wide(key).as_ptr()) };
}

/// Runs this program again with [arg] as an administrator (Windows asks), waiting up to two
/// minutes; Err with [failed] when it was refused or didn't finish well.
pub fn as_admin(arg: &str, failed: &str) -> anyhow::Result<()> {
    use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
    use windows::Win32::UI::Shell::{SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
    use windows::core::{HSTRING, PCWSTR};
    let exe = HSTRING::from(std::env::current_exe()?.as_os_str());
    let (verb, args) = (HSTRING::from("runas"), HSTRING::from(arg));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(exe.as_ptr()),
        lpParameters: PCWSTR(args.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    // SAFETY: the strings outlive the call; the process handle is waited on and closed.
    unsafe {
        if let Err(e) = ShellExecuteExW(&mut info) {
            if e.code() == ERROR_CANCELLED.to_hresult() {
                anyhow::bail!("Windows asked for an administrator and was told no");
            }
            return Err(anyhow::Error::from(e).context("couldn't ask Windows for an administrator"));
        }
        if info.hProcess.is_invalid() {
            anyhow::bail!("{failed}");
        }
        let waited = WaitForSingleObject(info.hProcess, 120_000);
        let mut code = 1u32;
        let _ = GetExitCodeProcess(info.hProcess, &mut code);
        let _ = CloseHandle(info.hProcess);
        if waited != WAIT_OBJECT_0 || code != 0 {
            anyhow::bail!("{failed}");
        }
    }
    Ok(())
}

/// The device interfaces of [class] present now, by their paths (cfgmgr32).
pub fn device_interfaces(class: &windows_sys::core::GUID) -> Vec<String> {
    use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
        CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CM_Get_Device_Interface_List_SizeW, CM_Get_Device_Interface_ListW, CR_BUFFER_SMALL, CR_SUCCESS,
    };
    for _ in 0..3 {
        let mut len = 0u32;
        // SAFETY: Windows writes the needed length (in characters), then a list of NUL-terminated
        // paths ending in an empty one into a buffer of that many characters.
        unsafe {
            if CM_Get_Device_Interface_List_SizeW(&mut len, class, std::ptr::null(), CM_GET_DEVICE_INTERFACE_LIST_PRESENT) != CR_SUCCESS || len <= 1 {
                return Vec::new();
            }
            let mut buf = vec![0u16; len as usize];
            match CM_Get_Device_Interface_ListW(class, std::ptr::null(), buf.as_mut_ptr(), len, CM_GET_DEVICE_INTERFACE_LIST_PRESENT) {
                CR_SUCCESS => return paths(&buf),
                // One arrived between the two calls: ask again.
                CR_BUFFER_SMALL => continue,
                _ => return Vec::new(),
            }
        }
    }
    Vec::new()
}

/// A double-NUL-terminated list of strings.
fn paths(list: &[u16]) -> Vec<String> {
    list.split(|&c| c == 0).take_while(|s| !s.is_empty()).map(String::from_utf16_lossy).collect()
}

/// This PC's own processor family, as Windows sees it (an x64 Vyoma runs on ARM64 Windows too,
/// but drivers and DLLs that Windows' own services load must be built for its real processor).
pub fn arm64() -> bool {
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, IsWow64Process2};
    const IMAGE_FILE_MACHINE_ARM64: u16 = 0xAA64;
    let (mut process, mut native) = (0u16, 0u16);
    // SAFETY: two out-params for this process.
    unsafe { IsWow64Process2(GetCurrentProcess(), &mut process, &mut native) != 0 && native == IMAGE_FILE_MACHINE_ARM64 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_windows_lists_of_paths() {
        let list: Vec<u16> = "\\\\?\\USB#VID_04E8&PID_6860#R5C#{f72fe0d4}\0\\\\?\\USB#VID_18D1#9A#{f72fe0d4}\0\0".encode_utf16().collect();
        assert_eq!(paths(&list).len(), 2);
        assert!(paths(&[0, 0]).is_empty());
    }

    #[test]
    fn registry_values_go_in_and_out() {
        let key = format!(r"Software\VyomaTest-{}", std::process::id());
        set_user_value(&format!(r"{key}\Sub"), Some("Name"), "C:\\Program Files\\Vyoma").unwrap();
        set_user_value(&format!(r"{key}\Sub"), None, "default").unwrap();
        assert_eq!(user_value(&format!(r"{key}\Sub"), Some("Name")).as_deref(), Some("C:\\Program Files\\Vyoma"));
        assert_eq!(user_value(&format!(r"{key}\Sub"), None).as_deref(), Some("default"));
        delete_user_key(&key);
        assert_eq!(user_value(&format!(r"{key}\Sub"), Some("Name")), None);
    }

    #[test]
    fn only_web_links_open() {
        assert!(openable("https://example.org/a?b=c"));
        assert!(openable("HTTP://example.org"));
        for no in ["ms-msdt:/id PCWDiagnostic", "search-ms:query=x", "file:///C:/Windows/System32/calc.exe", "C:\\Windows\\notepad.exe", "javascript:alert(1)", "https://x\n.y"] {
            assert!(!openable(no), "{no}");
        }
    }

    #[test]
    fn the_clipboard_takes_text_both_ways() {
        let text = "Vyoma \u{263a} test";
        set_clipboard(text).unwrap();
        assert_eq!(clipboard().as_deref(), Some(text));
    }

    #[test]
    fn strings_go_to_windows_and_back() {
        let w = wide("D\u{e9}j\u{e0}");
        assert_eq!(w.last(), Some(&0));
        // SAFETY: NUL-terminated above.
        assert_eq!(unsafe { from_wide(w.as_ptr()) }, "D\u{e9}j\u{e0}");
    }
}
