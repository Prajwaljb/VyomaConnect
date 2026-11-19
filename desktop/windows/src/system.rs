//! This PC as a whole: starting with Windows (the user's Run key: no administrator needed), one
//! agent at a time, sleep / restart / shut down / sign out as the Start menu does them (open apps
//! asked to close, never forced), and waking the screen.
//! Built and tested on CI and under Wine only; not yet run on a real Windows PC.

use std::path::Path;

use anyhow::{Result, bail};
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, LUID};
use windows_sys::Win32::Security::{AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY};
use windows_sys::Win32::System::Power::{ES_DISPLAY_REQUIRED, SetSuspendState, SetThreadExecutionState};
use windows_sys::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW};
use windows_sys::Win32::System::Shutdown::{EWX_LOGOFF, EWX_POWEROFF, EWX_REBOOT, EWX_SHUTDOWN, ExitWindowsEx, SHTDN_REASON_FLAG_PLANNED, SHTDN_REASON_MAJOR_OTHER};
use windows_sys::Win32::System::Threading::{CreateMutexW, GetCurrentProcess, OpenProcessToken};

use crate::win::wide;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "Vyoma";
/// Per sign-in session: two people signed in each have their own agent.
const AGENT_MUTEX: &str = r"Local\VyomaAgent";
const WINDOW_MUTEX: &str = r"Local\VyomaWindow";

struct Key(HKEY);

impl Key {
    fn open(access: u32) -> Result<Self> {
        let mut k: HKEY = std::ptr::null_mut();
        // SAFETY: opening an existing key under HKCU by a NUL-terminated path.
        let err = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), 0, access, &mut k) };
        if err != 0 {
            bail!("couldn't open the Run key ({err})");
        }
        Ok(Key(k))
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: opened in Key::open.
        unsafe { RegCloseKey(self.0) };
    }
}

/// The command line Windows runs at sign-in: this program, quoted (paths have spaces).
fn run_line(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

/// `--install`: starts with Windows from now on, as this copy of the program, and is in Explorer's Send to.
pub fn install() -> Result<()> {
    let exe = std::env::current_exe()?;
    let line = wide(&run_line(&exe));
    let k = Key::open(KEY_SET_VALUE)?;
    // SAFETY: a REG_SZ value of [line]'s bytes, NUL included.
    let err = unsafe { RegSetValueExW(k.0, wide(RUN_VALUE).as_ptr(), 0, REG_SZ, line.as_ptr().cast(), (line.len() * 2) as u32) };
    if err != 0 {
        bail!("couldn't write the Run value ({err})");
    }
    // Explorer's Send to entry, pointing at this copy too.
    if let Err(e) = crate::sendto::add() {
        tracing::info!("no Send to entry: {e:#}");
    }
    Ok(())
}

/// `--uninstall`: no longer starts with Windows (nor in Send to).
pub fn uninstall() -> Result<()> {
    crate::sendto::remove();
    let k = Key::open(KEY_SET_VALUE)?;
    // SAFETY: deleting a value by a NUL-terminated name; 2 (not found) is fine.
    let err = unsafe { RegDeleteValueW(k.0, wide(RUN_VALUE).as_ptr()) };
    if err != 0 && err != 2 {
        bail!("couldn't remove the Run value ({err})");
    }
    Ok(())
}

/// Whether Windows starts this copy at sign-in.
pub fn installed() -> bool {
    let Ok(k) = Key::open(KEY_QUERY_VALUE) else { return false };
    let mut buf = [0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    // SAFETY: reading into a buffer of [size] bytes.
    let err = unsafe { RegQueryValueExW(k.0, wide(RUN_VALUE).as_ptr(), std::ptr::null(), std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut size) };
    if err != 0 {
        return false;
    }
    let len = ((size / 2) as usize).min(buf.len());
    let value = String::from_utf16_lossy(&buf[..len]);
    std::env::current_exe().is_ok_and(|exe| value.trim_end_matches('\0') == run_line(&exe))
}

/// Held while this agent runs; a second agent in the same session gets none.
pub struct OnlyAgent(HANDLE);

// SAFETY: a mutex handle, only closed on drop.
unsafe impl Send for OnlyAgent {}

impl Drop for OnlyAgent {
    fn drop(&mut self) {
        // SAFETY: made in only_agent.
        unsafe { CloseHandle(self.0) };
    }
}

pub fn only_agent() -> Option<OnlyAgent> {
    // SAFETY: a named mutex; ERROR_ALREADY_EXISTS says another agent has it.
    unsafe {
        let h = CreateMutexW(std::ptr::null(), 1, wide(AGENT_MUTEX).as_ptr());
        if h.is_null() {
            return None;
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            CloseHandle(h);
            return None;
        }
        Some(OnlyAgent(h))
    }
}

/// Whether an agent holds this session's lock (without taking it).
pub fn agent_running() -> bool {
    mutex_held(AGENT_MUTEX)
}

/// Whether Vyoma's window is open in this session (its own lock, gui/src/one_window.rs).
pub fn window_running() -> bool {
    mutex_held(WINDOW_MUTEX)
}

fn mutex_held(name: &str) -> bool {
    use windows_sys::Win32::System::Threading::{OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
    // SAFETY: opening an existing named mutex just to see that it's there, then closing it.
    unsafe {
        let h = OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, 0, wide(name).as_ptr());
        if h.is_null() {
            return false;
        }
        CloseHandle(h);
        true
    }
}

/// Asks Vyoma's window to close, as its close button would (the installer replaces it next).
pub fn close_windows() {
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{EnumWindows, GetClassNameW, GetWindowTextW, PostMessageW, WM_CLOSE};
    unsafe extern "system" fn close_ours(hwnd: HWND, _: LPARAM) -> i32 {
        let mut buf = [0u16; 64];
        // SAFETY: reading a window's title and class into fixed buffers; posting it WM_CLOSE.
        unsafe {
            let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
            if n <= 0 || String::from_utf16_lossy(&buf[..n as usize]) != "Vyoma" {
                return 1;
            }
            let n = GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
            if n > 0 && String::from_utf16_lossy(&buf[..n as usize]) == crate::tray::CLASS {
                return 1;
            }
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
        }
        1
    }
    // SAFETY: walks the top-level windows with a callback that only reads and posts.
    unsafe { EnumWindows(Some(close_ours), 0) };
}

/// Lets this process shut down, restart and sleep the PC (every user has the right; it's off
/// until a program turns it on).
fn allow_shutdown() -> Result<()> {
    // SAFETY: the process token, opened to adjust one privilege and closed.
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut token) == 0 {
            bail!("no process token: {}", std::io::Error::last_os_error());
        }
        let mut luid = LUID { LowPart: 0, HighPart: 0 };
        let ok = LookupPrivilegeValueW(std::ptr::null(), wide("SeShutdownPrivilege").as_ptr(), &mut luid) != 0 && {
            let p = TOKEN_PRIVILEGES { PrivilegeCount: 1, Privileges: [LUID_AND_ATTRIBUTES { Luid: luid, Attributes: SE_PRIVILEGE_ENABLED }] };
            AdjustTokenPrivileges(token, 0, &p, 0, std::ptr::null_mut(), std::ptr::null_mut()) != 0 && GetLastError() == 0
        };
        CloseHandle(token);
        if !ok {
            bail!("Windows didn't allow it: {}", std::io::Error::last_os_error());
        }
    }
    Ok(())
}

/// The phone asked first, so no "are you sure" here.
pub fn power(action: &str) -> Result<()> {
    let reason = SHTDN_REASON_MAJOR_OTHER | SHTDN_REASON_FLAG_PLANNED;
    // SAFETY: plain calls asking Windows to sleep or end the session.
    let ok = unsafe {
        match action {
            "sleep" => {
                allow_shutdown()?;
                SetSuspendState(false, false, false)
            }
            "restart" => {
                allow_shutdown()?;
                ExitWindowsEx(EWX_REBOOT, reason) != 0
            }
            "shutdown" => {
                allow_shutdown()?;
                ExitWindowsEx(EWX_SHUTDOWN | EWX_POWEROFF, reason) != 0
            }
            "logout" => ExitWindowsEx(EWX_LOGOFF, 0) != 0,
            other => bail!("unknown power action {other}"),
        }
    };
    if !ok {
        bail!("Windows didn't do it: {}", std::io::Error::last_os_error());
    }
    Ok(())
}

/// Lights the screen as a touch of the mouse would: a move there and back (Windows wakes the
/// display for input, not for a program's request alone).
pub fn wake() -> Result<()> {
    // SAFETY: a one-off execution state for this thread.
    unsafe { SetThreadExecutionState(ES_DISPLAY_REQUIRED) };
    let mut p = crate::input::Pointer::default();
    p.handle(&[crate::input::Act::Move(1.0, 0.0), crate::input::Act::Move(-1.0, 0.0)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_agent_per_session() {
        let first = only_agent().expect("the first gets it");
        assert!(only_agent().is_none(), "a second doesn't");
        assert!(agent_running());
        drop(first);
        assert!(only_agent().is_some(), "free again once the first stops");
    }

    #[test]
    fn the_run_line_is_quoted() {
        assert_eq!(run_line(Path::new(r"C:\Program Files\Vyoma\vyoma-windows.exe")), r#""C:\Program Files\Vyoma\vyoma-windows.exe""#);
    }

    #[test]
    fn unknown_power_words_do_nothing() {
        assert!(power("dance").is_err());
    }
}
