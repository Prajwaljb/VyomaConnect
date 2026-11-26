//! Vyoma in the notification area: its icon (click: open Vyoma's window; right click: Open, Quit),
//! its notifications (balloons, where Windows has no toasts for Vyoma: see [crate::toast]), and
//! the session ending (shut down, restart, sign out), so phones hear the agent is leaving.
//!
//! One hidden window on its own thread owns all of it; the async side talks to it through
//! [Tray::notify] and hears back on the channel given to [start].
//! Built and tested on CI and under Wine only; not yet run on a real Windows PC.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use tokio::sync::mpsc;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CW_USEDEFAULT, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DispatchMessageW, GetCursorPos, GetMessageW, HICON,
    IDI_APPLICATION, LoadIconW, MF_SEPARATOR, MF_STRING, MSG, PostMessageW, PostQuitMessage, RegisterClassExW, RegisterWindowMessageW,
    SetForegroundWindow, TPM_BOTTOMALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WM_APP, WM_CLOSE, WM_CONTEXTMENU,
    WM_DESTROY, WM_ENDSESSION, WM_LBUTTONUP, WM_NULL, WM_QUERYENDSESSION, WM_RBUTTONUP, WNDCLASSEXW, PBT_APMSUSPEND, PBT_APMRESUMEAUTOMATIC, WM_POWERBROADCAST,
};

use crate::win::wide;

/// What the person did with the icon, or the session ending.
#[derive(Debug, PartialEq)]
pub enum Said {
    Open,
    Quit,
    /// Windows is shutting down, restarting or signing out.
    Ending,
    /// The session was locked (true) or unlocked.
    Locked(bool),
    /// The PC is going to sleep (Modern Standby too), and is awake again.
    Sleeping,
    Awake,
}

/// The hidden window's class (Vyoma's window shares its title, "Vyoma").
pub const CLASS: &str = "VyomaAgent";
/// A notification queued for the window's thread.
const SHOW_NOTE: u32 = WM_APP + 1;
/// The icon's mouse messages.
const ICON_EVENT: u32 = WM_APP + 2;
const OPEN: usize = 1;
const QUIT: usize = 2;
/// Windows waits for WM_ENDSESSION's (and sleep's) answer; this long lets the agent tell phones it's leaving.
const GOODBYE: Duration = Duration::from_millis(600);
/// The session's lock changes (WTSRegisterSessionNotification): WM_WTSSESSION_CHANGE's lock and unlock.
const WM_WTSSESSION_CHANGE: u32 = 0x02B1;
const WTS_SESSION_LOCK: usize = 7;
const WTS_SESSION_UNLOCK: usize = 8;
/// The lock as last known: 0 unknown, 1 unlocked, 2 locked.
static LOCK: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Whether the session is locked, once it's known (asked as the agent starts, then followed).
pub fn locked() -> Option<bool> {
    match LOCK.load(std::sync::atomic::Ordering::Relaxed) {
        1 => Some(false),
        2 => Some(true),
        _ => None,
    }
}

fn set_lock(locked: bool) {
    LOCK.store(if locked { 2 } else { 1 }, std::sync::atomic::Ordering::Relaxed);
}

/// The session's lock now, from Windows' session information (Windows 10 and later report it
/// the right way round; 7 and Server 2008 R2 had it inverted). None where Windows won't say (Wine).
fn session_locked() -> Option<bool> {
    use windows_sys::Win32::System::RemoteDesktop::{
        WTS_CURRENT_SERVER_HANDLE, WTS_CURRENT_SESSION, WTS_SESSIONSTATE_LOCK, WTS_SESSIONSTATE_UNLOCK, WTSFreeMemory, WTSINFOEXW, WTSQuerySessionInformationW,
        WTSSessionInfoEx,
    };
    let (mut buf, mut len) = (std::ptr::null_mut::<u16>(), 0u32);
    // SAFETY: Windows allocates the buffer, read as its WTSINFOEXW and freed with WTSFreeMemory.
    unsafe {
        if WTSQuerySessionInformationW(WTS_CURRENT_SERVER_HANDLE, WTS_CURRENT_SESSION, WTSSessionInfoEx, &mut buf, &mut len) == 0 || buf.is_null() {
            return None;
        }
        let info = &*(buf as *const WTSINFOEXW);
        let flags = if info.Level == 1 && len as usize >= std::mem::size_of::<WTSINFOEXW>() { Some(info.Data.WTSInfoExLevel1.SessionFlags as u32) } else { None };
        WTSFreeMemory(buf.cast());
        match flags {
            Some(WTS_SESSIONSTATE_LOCK) => Some(true),
            Some(WTS_SESSIONSTATE_UNLOCK) => Some(false),
            _ => None,
        }
    }
}

struct State {
    said: mpsc::UnboundedSender<Said>,
    notes: Mutex<VecDeque<(String, String)>>,
    /// Explorer's "the taskbar is back" (after it restarted): the icon goes in again.
    taskbar_created: u32,
}

static STATE: OnceLock<State> = OnceLock::new();
static TRAY: OnceLock<Tray> = OnceLock::new();

/// Shows a notification: a toast under Vyoma's name where Windows has them ([crate::toast]),
/// else a balloon from Vyoma's icon; before there is one (tests, `--send-any`), the log has it.
/// Pressing it opens Vyoma. Nothing if the person turned Vyoma's notifications off.
pub fn notify(title: &str, body: &str) {
    use crate::toast::{self, Shown, Toast};
    let t = Toast { title: title.to_owned(), body: body.to_owned(), launch: toast::args(&[("do", "open")]), ..Default::default() };
    match toast::show(&t) {
        Shown::Yes => {}
        Shown::Off => tracing::info!("(notifications off) {title}"),
        Shown::Unavailable => match TRAY.get() {
            Some(t) => t.notify(title, body),
            None => tracing::info!("(notification) {title}: {body}"),
        },
    }
}

/// A balloon from Vyoma's icon, for where toasts aren't there; its words never go to the log
/// (they can be a caller's name).
pub fn balloon(title: &str, body: &str) {
    match TRAY.get() {
        Some(t) => t.notify(title, body),
        None => tracing::info!("(no notification area for a notification)"),
    }
}

#[derive(Clone, Copy)]
pub struct Tray {
    hwnd: usize,
}

impl Tray {
    /// Shows a notification under Vyoma's icon. Several close together are shown in turn.
    pub fn notify(&self, title: &str, body: &str) {
        let Some(state) = STATE.get() else { return };
        state.notes.lock().unwrap().push_back((title.to_owned(), body.to_owned()));
        // SAFETY: posting to our own window; it pops the queue on its thread.
        unsafe { PostMessageW(self.hwnd as HWND, SHOW_NOTE, 0, 0) };
    }

    /// Takes the icon away and ends the window's thread.
    pub fn close(&self) {
        // SAFETY: as in notify.
        unsafe { PostMessageW(self.hwnd as HWND, WM_CLOSE, 0, 0) };
    }
}

/// Puts the icon in and starts its thread; None if Windows wouldn't make the window.
pub fn start(said: mpsc::UnboundedSender<Said>) -> Option<Tray> {
    // SAFETY: registering a window message by a NUL-terminated name.
    let taskbar_created = unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) };
    STATE.set(State { said, notes: Mutex::new(VecDeque::new()), taskbar_created }).ok()?;
    let (tx, rx) = std::sync::mpsc::channel::<Option<usize>>();
    std::thread::Builder::new()
        .name("vyoma-tray".into())
        .spawn(move || {
            let hwnd = make_window();
            let _ = tx.send(hwnd.map(|h| h as usize));
            if let Some(hwnd) = hwnd {
                add_icon(hwnd);
                pump();
            }
        })
        .ok()?;
    let tray = Tray { hwnd: rx.recv().ok().flatten()? };
    let _ = TRAY.set(tray);
    Some(tray)
}

fn make_window() -> Option<HWND> {
    let class = wide(CLASS);
    // SAFETY: a class with our window procedure, then a hidden window of it (never shown: it only
    // receives messages; not message-only, which wouldn't hear the session end or the taskbar).
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassExW(&wc);
        let title = wide("Vyoma");
        let hwnd = CreateWindowExW(0, class.as_ptr(), title.as_ptr(), 0, CW_USEDEFAULT, CW_USEDEFAULT, 0, 0, std::ptr::null_mut(), std::ptr::null_mut(), instance, std::ptr::null());
        if !hwnd.is_null() {
            // Lock and unlock come to this window from now on; what it is now, asked once.
            windows_sys::Win32::System::RemoteDesktop::WTSRegisterSessionNotification(hwnd, windows_sys::Win32::System::RemoteDesktop::NOTIFY_FOR_THIS_SESSION);
            if let Some(locked) = session_locked() {
                set_lock(locked);
            }
            // Sleep and wake come here too; registered, Modern Standby's as well.
            windows_sys::Win32::System::Power::RegisterSuspendResumeNotification(hwnd, windows_sys::Win32::UI::WindowsAndMessaging::DEVICE_NOTIFY_WINDOW_HANDLE);
        }
        (!hwnd.is_null()).then_some(hwnd)
    }
}

fn pump() {
    // SAFETY: the standard message loop for this thread's window.
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// Vyoma's icon from the program's resources (number 1), else Windows' plain application icon.
fn icon() -> HICON {
    // SAFETY: loading an icon resource by number; null falls back to the system's.
    unsafe {
        let mine = LoadIconW(GetModuleHandleW(std::ptr::null()), std::ptr::without_provenance(1));
        if mine.is_null() { LoadIconW(std::ptr::null_mut(), IDI_APPLICATION) } else { mine }
    }
}

/// Copies [s] into one of the notification's fixed fields, cut to fit with its NUL.
fn put<const N: usize>(field: &mut [u16; N], s: &str) {
    let units: Vec<u16> = s.encode_utf16().take(N - 1).collect();
    field[..units.len()].copy_from_slice(&units);
    field[units.len()] = 0;
}

fn data(hwnd: HWND) -> NOTIFYICONDATAW {
    // SAFETY: a plain C struct; zero is its empty value.
    let mut d: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    d.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    d.hWnd = hwnd;
    d.uID = 1;
    d
}

fn add_icon(hwnd: HWND) {
    let mut d = data(hwnd);
    d.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    d.uCallbackMessage = ICON_EVENT;
    d.hIcon = icon();
    put(&mut d.szTip, "Vyoma");
    // SAFETY: a filled-in NOTIFYICONDATAW for our window.
    unsafe { Shell_NotifyIconW(NIM_ADD, &d) };
}

fn show_notes(hwnd: HWND) {
    let Some(state) = STATE.get() else { return };
    while let Some((title, body)) = state.notes.lock().unwrap().pop_front() {
        let mut d = data(hwnd);
        d.uFlags = NIF_INFO;
        d.dwInfoFlags = NIIF_INFO;
        put(&mut d.szInfoTitle, &title);
        // An empty text shows nothing at all: the title stands in.
        put(&mut d.szInfo, if body.is_empty() { &title } else { &body });
        // SAFETY: as in add_icon.
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &d) };
    }
}

fn menu(hwnd: HWND) -> usize {
    // SAFETY: a popup menu shown at the pointer and destroyed after; SetForegroundWindow first so
    // it closes when clicked away from (Windows' documented quirk), WM_NULL after.
    unsafe {
        let m = CreatePopupMenu();
        let (open, quit) = (wide("Open Vyoma"), wide("Quit Vyoma"));
        AppendMenuW(m, MF_STRING, OPEN, open.as_ptr());
        AppendMenuW(m, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(m, MF_STRING, QUIT, quit.as_ptr());
        let mut at = POINT { x: 0, y: 0 };
        GetCursorPos(&mut at);
        SetForegroundWindow(hwnd);
        let picked = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, at.x, at.y, 0, hwnd, std::ptr::null());
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(m);
        picked as usize
    }
}

fn say(s: Said) {
    if let Some(state) = STATE.get() {
        let _ = state.said.send(s);
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        SHOW_NOTE => show_notes(hwnd),
        ICON_EVENT => match lparam as u32 {
            WM_LBUTTONUP => say(Said::Open),
            WM_RBUTTONUP | WM_CONTEXTMENU => match menu(hwnd) {
                OPEN => say(Said::Open),
                QUIT => say(Said::Quit),
                _ => {}
            },
            _ => {}
        },
        WM_WTSSESSION_CHANGE if matches!(wparam, WTS_SESSION_LOCK | WTS_SESSION_UNLOCK) => {
            let locked = wparam == WTS_SESSION_LOCK;
            set_lock(locked);
            say(Said::Locked(locked));
        }
        WM_POWERBROADCAST => {
            match wparam as u32 {
                PBT_APMSUSPEND => {
                    say(Said::Sleeping);
                    // Windows waits a moment for this answer: long enough to tell the phones.
                    std::thread::sleep(GOODBYE);
                }
                PBT_APMRESUMEAUTOMATIC => say(Said::Awake),
                _ => {}
            }
            return 1;
        }
        WM_QUERYENDSESSION => return 1,
        WM_ENDSESSION => {
            if wparam != 0 {
                say(Said::Ending);
                std::thread::sleep(GOODBYE);
            }
            return 0;
        }
        WM_CLOSE => {
            let d = data(hwnd);
            // SAFETY: removing our own icon, then ending the loop.
            unsafe {
                Shell_NotifyIconW(NIM_DELETE, &d);
                windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd);
            }
            return 0;
        }
        // SAFETY: ends this thread's message loop.
        WM_DESTROY => unsafe { PostQuitMessage(0) },
        m if STATE.get().is_some_and(|s| s.taskbar_created != 0 && s.taskbar_created == m) => add_icon(hwnd),
        // SAFETY: everything else as Windows does by default.
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    0
}
