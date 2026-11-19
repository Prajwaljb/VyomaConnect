//! One window at a time on Windows (Linux's is `instance.rs` on the session bus): the first
//! holds a named mutex for its session; a second start brings that window to the front, hands it
//! the page it was asked for (`--page`, also how the agent opens the command editor for a phone)
//! and ends. The page goes as WM_COPYDATA to a hidden message-only window the first one keeps on a
//! thread of its own; only page names pass (letters, digits, `-`, `_`, `.`), so another program can
//! at most turn the page.

use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, WPARAM};
use slint::ComponentHandle;
use windows_sys::Win32::System::DataExchange::COPYDATASTRUCT;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, EnumWindows, FindWindowExW, GetClassNameW, GetMessageW, GetWindowTextW, HWND_MESSAGE,
    IsIconic, IsWindowVisible, MSG, RegisterClassExW, SMTO_ABORTIFHUNG, SW_RESTORE, SendMessageTimeoutW, SetForegroundWindow, ShowWindow, WM_COPYDATA, WNDCLASSEXW,
};

use crate::MainWindow;

/// Per sign-in session, as the agent's own lock.
const MUTEX: &str = r"Local\VyomaWindow";
/// The window's title (`MainWindow` in app.slint); the agent's hidden window shares it, by its class.
const TITLE: &str = "Vyoma";
const AGENT_CLASS: &str = "VyomaAgent";
/// The hidden window that takes pages.
const PAGES_CLASS: &str = "VyomaWindowPages";
/// WM_COPYDATA's tag for a page ("VYPG").
const PAGE_TAG: usize = 0x5659_5047;
const PAGE_MAX: usize = 128;

type OnPage = Box<dyn Fn(String) + Send + Sync>;
static ON_PAGE: OnceLock<OnPage> = OnceLock::new();

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// True when this is the only window (it then takes pages from later starts); else the open one
/// comes to the front on [page].
pub fn claim(ui: &MainWindow, page: &str) -> bool {
    // SAFETY: a named mutex, kept for the life of the process (never closed).
    let first = unsafe {
        let h = CreateMutexW(std::ptr::null(), 1, wide(MUTEX).as_ptr());
        !h.is_null() && GetLastError() != ERROR_ALREADY_EXISTS
    };
    if first {
        let ui = std::sync::Mutex::new(ui.as_weak());
        listen(Box::new(move |page| {
            let _ = ui.lock().unwrap().upgrade_in_event_loop(move |win| {
                crate::open_page(&win, &page);
                let _ = win.show();
                win.window().set_minimized(false);
            });
        }));
    } else {
        send(page);
        // SAFETY: walks the top-level windows with a callback that only reads them. (This process
        // was just started by the person, so Windows lets it bring the window forward.)
        unsafe { EnumWindows(Some(raise_ours), 0) };
    }
    first
}

/// Whether [page] is a page name: what `--page` takes, nothing else.
pub fn is_page(page: &str) -> bool {
    !page.is_empty() && page.len() <= PAGE_MAX && page.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Takes pages from later starts on a thread of its own, each to [on_page].
fn listen(on_page: OnPage) {
    if ON_PAGE.set(on_page).is_err() {
        return;
    }
    let _ = std::thread::Builder::new().name("vyoma-pages".into()).spawn(|| {
        let class = wide(PAGES_CLASS);
        // SAFETY: a class with our procedure, one message-only window of it, and the standard
        // message loop on this thread.
        unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            let wc = WNDCLASSEXW { cbSize: size_of::<WNDCLASSEXW>() as u32, lpfnWndProc: Some(pages_proc), hInstance: instance, lpszClassName: class.as_ptr(), ..std::mem::zeroed() };
            RegisterClassExW(&wc);
            let hwnd = CreateWindowExW(0, class.as_ptr(), class.as_ptr(), 0, 0, 0, 0, 0, HWND_MESSAGE, std::ptr::null_mut(), instance, std::ptr::null());
            if hwnd.is_null() {
                return;
            }
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                DispatchMessageW(&msg);
            }
        }
    });
}

unsafe extern "system" fn pages_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg != WM_COPYDATA || lparam == 0 {
        // SAFETY: Windows' own handling for everything else.
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    // SAFETY: WM_COPYDATA's lParam is a COPYDATASTRUCT whose data Windows copied into this process
    // and keeps valid while this runs; its length is checked before it's read.
    let page = unsafe {
        let data = &*(lparam as *const COPYDATASTRUCT);
        let units = data.cbData as usize / 2;
        if data.dwData != PAGE_TAG || data.lpData.is_null() || units > PAGE_MAX {
            return 0;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(data.lpData as *const u16, units))
    };
    if !is_page(&page) {
        return 0;
    }
    if let Some(on_page) = ON_PAGE.get() {
        on_page(page);
    }
    1
}

/// Hands [page] to the open window, if it asked for one.
fn send(page: &str) -> bool {
    if !is_page(page) {
        return false;
    }
    let units: Vec<u16> = page.encode_utf16().collect();
    let data = COPYDATASTRUCT { dwData: PAGE_TAG, cbData: (units.len() * 2) as u32, lpData: units.as_ptr() as *mut _ };
    let class = wide(PAGES_CLASS);
    // SAFETY: finding the open window's message-only window by class, then handing it our data,
    // which stays alive until SendMessageTimeoutW returns (it waits at most 2 s for a hung window).
    unsafe {
        let hwnd = FindWindowExW(HWND_MESSAGE, std::ptr::null_mut(), class.as_ptr(), std::ptr::null());
        if hwnd.is_null() {
            return false;
        }
        let mut result = 0usize;
        SendMessageTimeoutW(hwnd, WM_COPYDATA, 0, &data as *const _ as LPARAM, SMTO_ABORTIFHUNG, 2000, &mut result) != 0 && result == 1
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_page_names_pass() {
        for page in ["commands", "this-pc", "activity", "f460b0e1_2acb_4049_944d_f544ef113514"] {
            assert!(is_page(page), "{page}");
        }
        for page in ["", "a b", "C:\\x", "../x", "page\0", &"x".repeat(200)] {
            assert!(!is_page(page), "{page:?}");
        }
    }

    #[test]
    fn a_second_start_hands_its_page_to_the_open_window() {
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let tx = std::sync::Mutex::new(tx);
        listen(Box::new(move |page| {
            let _ = tx.lock().unwrap().send(page);
        }));
        let started = std::time::Instant::now();
        while !send("commands") {
            assert!(started.elapsed() < std::time::Duration::from_secs(5), "the open window never took the page");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap(), "commands");
        assert!(!send("not a page"), "refused before it's sent");
    }
}
