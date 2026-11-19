//! One window at a time on Windows (Linux's is `instance.rs` on the session bus): the first
//! holds a named mutex for its session; a second start brings that window to the front and ends.
//! The page a second start asks for (`--page`) isn't passed on yet.

use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowTextW, IsIconic, IsWindowVisible, SW_RESTORE, SetForegroundWindow, ShowWindow,
};

/// Per sign-in session, as the agent's own lock.
const MUTEX: &str = r"Local\VyomaWindow";
/// The window's title (`MainWindow` in app.slint); the agent's hidden window shares it, by its class.
const TITLE: &str = "Vyoma";
const AGENT_CLASS: &str = "VyomaAgent";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// True when this is the only window; else the open one comes to the front.
pub fn claim() -> bool {
    // SAFETY: a named mutex, kept for the life of the process (never closed).
    let first = unsafe {
        let h = CreateMutexW(std::ptr::null(), 1, wide(MUTEX).as_ptr());
        !h.is_null() && GetLastError() != ERROR_ALREADY_EXISTS
    };
    if !first {
        // SAFETY: walks the top-level windows with a callback that only reads them.
        unsafe { EnumWindows(Some(raise_ours), 0) };
    }
    first
}

unsafe extern "system" fn raise_ours(hwnd: HWND, _: LPARAM) -> i32 {
    let mut buf = [0u16; 64];
    // SAFETY: reading a window's title and class into fixed buffers.
    unsafe {
        let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        if n <= 0 || String::from_utf16_lossy(&buf[..n as usize]) != TITLE || IsWindowVisible(hwnd) == 0 {
            return 1;
        }
        let n = GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        if n > 0 && String::from_utf16_lossy(&buf[..n as usize]) == AGENT_CLASS {
            return 1;
        }
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        SetForegroundWindow(hwnd);
    }
    0
}
