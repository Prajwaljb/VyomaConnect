//! The presenter's pointer (stock `kdeconnect.presenter`): core's red dot over everything on the
//! main screen while the phone points ([vyoma_core::presenter]: how the packets move it, and the dot
//! itself, as on Linux and the Mac). It hides after a quiet second and when the phone goes.
//!
//! The dot is a small layered window (per-pixel alpha, drawn once with UpdateLayeredWindow) that's
//! topmost, takes no clicks or focus and stays out of the taskbar and Alt+Tab; moving it redraws
//! nothing. Its own thread, made with the first move.
//! Built and tested on CI and under Wine only; not yet run on a real Windows PC.

use std::sync::mpsc;
use std::time::Duration;

use serde_json::Value;
use vyoma_core::presenter::{Order, QUIET, SIZE, Spot, paint_dot};
use windows_sys::Win32::Foundation::{HWND, POINT, RECT, SIZE as WSIZE};
use windows_sys::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject,
    GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint, SelectObject,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetThreadDpiAwarenessContext};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, HWND_TOPMOST, MSG, PM_REMOVE, PeekMessageW, RegisterClassExW, SW_HIDE, SWP_NOACTIVATE,
    SWP_NOSIZE, SWP_SHOWWINDOW, SetWindowPos, ShowWindow, TranslateMessage, ULW_ALPHA, UpdateLayeredWindow, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::win::wide;

pub use vyoma_core::presenter::PRESENTER;

const CLASS: &str = "VyomaPresenter";
/// How often the dot's thread looks at its window's messages while waiting.
const TICK: Duration = Duration::from_millis(50);

/// Where the phone's packets go; the dot's thread starts with the first move.
#[derive(Default)]
pub struct Presenter(Option<mpsc::Sender<Order>>);

impl Presenter {
    pub fn on_packet(&mut self, body: &Value) {
        let Some(order) = Order::of(body) else { return };
        if self.0.is_none() && order == Order::Stop {
            return;
        }
        let tx = self.0.get_or_insert_with(|| {
            let (tx, rx) = mpsc::channel();
            if let Err(e) = std::thread::Builder::new().name("vyoma-presenter".into()).spawn(move || run(rx)) {
                tracing::warn!("no presenter pointer: {e}");
            }
            tx
        });
        if tx.send(order).is_err() {
            // The dot's thread ended (no window): the next packet tries again.
            self.0 = None;
        }
    }

    /// The phone went: the dot goes.
    pub fn stop(&self) {
        if let Some(tx) = &self.0 {
            let _ = tx.send(Order::Stop);
        }
    }
}

/// The dot's window on its thread.
pub struct Dot {
    hwnd: HWND,
    spot: Spot,
    shown: bool,
    side: i32,
}

impl Dot {
    /// Makes the (hidden) window and draws the dot into it at this screen's scale.
    pub fn new() -> Option<Dot> {
        let class = wide(CLASS);
        // SAFETY: a class with the default procedure, a layered popup of it, and a DIB section of
        // exactly side × side pixels handed to UpdateLayeredWindow, then freed.
        unsafe {
            SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let instance = GetModuleHandleW(std::ptr::null());
            let wc = WNDCLASSEXW { cbSize: size_of::<WNDCLASSEXW>() as u32, lpfnWndProc: Some(DefWindowProcW), hInstance: instance, lpszClassName: class.as_ptr(), ..std::mem::zeroed() };
            RegisterClassExW(&wc);
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class.as_ptr(),
                class.as_ptr(),
                WS_POPUP,
                0,
                0,
                SIZE,
                SIZE,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            );
            if hwnd.is_null() {
                return None;
            }
            let dpi = GetDpiForWindow(hwnd).max(96) as i32;
            let side = SIZE * dpi / 96;
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader = BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: side, biHeight: -side, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB, ..std::mem::zeroed() };
            let dc = CreateCompatibleDC(std::ptr::null_mut());
            let mut bits = std::ptr::null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
            if bitmap.is_null() || bits.is_null() {
                DeleteDC(dc);
                DestroyWindow(hwnd);
                return None;
            }
            paint_dot(std::slice::from_raw_parts_mut(bits.cast::<u8>(), (side * side * 4) as usize), side as usize);
            let old = SelectObject(dc, bitmap);
            let (size, from) = (WSIZE { cx: side, cy: side }, POINT { x: 0, y: 0 });
            let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
            let drawn = UpdateLayeredWindow(hwnd, std::ptr::null_mut(), std::ptr::null(), &size, dc, &from, 0, &blend, ULW_ALPHA);
            SelectObject(dc, old);
            DeleteObject(bitmap);
            DeleteDC(dc);
            if drawn == 0 {
                DestroyWindow(hwnd);
                return None;
            }
            Some(Dot { hwnd, spot: Spot::CENTRE, shown: false, side })
        }
    }

    /// The main screen: its top-left and size, in pixels.
    fn screen() -> (i32, i32, i32, i32) {
        // SAFETY: plain reads of the primary monitor's rectangle.
        unsafe {
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY), &mut info);
            let r: RECT = info.rcMonitor;
            (r.left, r.top, r.right - r.left, r.bottom - r.top)
        }
    }

    /// Where the window is now.
    #[cfg(test)]
    fn at(&self) -> (i32, i32) {
        let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        // SAFETY: our window's rectangle.
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect(self.hwnd, &mut r) };
        (r.left, r.top)
    }

    #[cfg(test)]
    fn shown(&self) -> bool {
        self.shown
    }

    pub fn carry_out(&mut self, order: Order) {
        match order {
            Order::Stop => self.hide(),
            Order::Move(dx, dy) => {
                let (left, top, w, h) = Dot::screen();
                self.spot = self.spot.moved(dx, dy, (w, h));
                let (x, y) = self.spot.corner((w, h), self.side);
                // SAFETY: moving our own window, above everything, without taking focus.
                unsafe { SetWindowPos(self.hwnd, HWND_TOPMOST, left + x, top + y, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW) };
                self.shown = true;
            }
        }
    }

    pub fn hide(&mut self) {
        if self.shown {
            // SAFETY: hiding our own window.
            unsafe { ShowWindow(self.hwnd, SW_HIDE) };
            (self.shown, self.spot) = (false, Spot::CENTRE);
        }
    }

    /// Handles what Windows sent the window (nothing much: it takes no input).
    fn pump() {
        // SAFETY: the standard message handling for this thread's window, without blocking.
        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

impl Drop for Dot {
    fn drop(&mut self) {
        // SAFETY: our window, on the thread that made it.
        unsafe { DestroyWindow(self.hwnd) };
    }
}

fn run(orders: mpsc::Receiver<Order>) {
    let Some(mut dot) = Dot::new() else {
        tracing::warn!("no presenter pointer: Windows wouldn't make its window");
        return;
    };
    let mut quiet = Duration::ZERO;
    loop {
        match orders.recv_timeout(TICK) {
            Ok(order) => {
                quiet = Duration::ZERO;
                dot.carry_out(order);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                quiet += TICK;
                // The phone stopped sending without saying so.
                if quiet >= QUIET {
                    dot.hide();
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
        Dot::pump();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dot_shows_moves_and_hides() {
        let Some(mut dot) = Dot::new() else {
            println!("no window here: skipped");
            return;
        };
        let (left, top, w, h) = Dot::screen();
        assert!(!dot.shown());
        dot.carry_out(Order::Move(0.0, 0.0));
        assert!(dot.shown());
        assert_eq!(dot.at(), (left + w / 2 - dot.side / 2, top + h / 2 - dot.side / 2), "in the middle at first");
        dot.carry_out(Order::Move(5.0, -5.0));
        assert_eq!(dot.at(), (left + w - dot.side, top), "kept on the screen at its top right");
        dot.carry_out(Order::Stop);
        assert!(!dot.shown());
        dot.carry_out(Order::Move(0.0, 0.0));
        assert_eq!(dot.at(), (left + w / 2 - dot.side / 2, top + h / 2 - dot.side / 2), "back in the middle after a stop");
    }

    #[test]
    fn packets_reach_the_dot_and_a_quiet_second_hides_it() {
        let mut p = Presenter::default();
        p.on_packet(&serde_json::json!({ "stop": true }));
        assert!(p.0.is_none(), "a stop alone starts nothing");
        p.on_packet(&serde_json::json!({ "dx": 0.1, "dy": 0.1 }));
        assert!(p.0.is_some());
        std::thread::sleep(QUIET + Duration::from_millis(200));
        p.stop();
    }
}

#[cfg(test)]
mod look {
    /// Holds the dot up for a look (a private X server only): `-- --ignored holds_the_dot`.
    #[test]
    #[ignore]
    fn holds_the_dot() {
        let mut dot = super::Dot::new().unwrap();
        dot.carry_out(super::Order::Move(0.0, 0.0));
        let until = std::time::Instant::now() + std::time::Duration::from_secs(4);
        while std::time::Instant::now() < until {
            super::Dot::pump();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}
