//! Handoff on Windows: the web video playing in a browser, and where in it, so the phone can carry
//! on from the same moment (as the Linux app does from MPRIS).
//!
//! Windows' media controls name what plays and where it is, but never the page. So the browser
//! window whose title holds the video's title (its active tab) is found, and the address read from
//! its address bar through UI Automation: the first edit box holding a web address, outside the
//! page itself. A video in a tab that isn't the one showing can't be handed over; the phone isn't
//! offered it. The position is the browser's own where it reports one (Chromium-based browsers),
//! else the address's `t=`, else unknown.
//! Built and tested on CI and under Wine only; not yet run on a real Windows PC.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use vyoma_engine::Handoff;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationValuePattern, UIA_DocumentControlTypeId, UIA_EditControlTypeId,
    UIA_ValuePatternId,
};
use windows_sys::Win32::Foundation::{CloseHandle, LPARAM};
use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
use windows_sys::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible};

use crate::media;

/// Browsers whose windows are looked in, by program name.
const BROWSERS: [&str; 14] =
    ["chrome", "msedge", "firefox", "brave", "vivaldi", "opera", "chromium", "librewolf", "waterfox", "floorp", "zen", "thorium", "arc", "yandex"];
/// Firefox's family names its media session with a hash of where it's installed.
const FIREFOX_LIKE: [&str; 5] = ["firefox", "librewolf", "waterfox", "floorp", "zen"];
/// Elements of a browser window looked at for its address bar, at most (the bar is near the top).
const MOST_ELEMENTS: usize = 400;
/// The offer is kept fresh this often while phones ask for it.
const REFRESH: Duration = Duration::from_secs(3);
/// Phones stop asking (their Home closed): the refreshing stops after this.
const ASKED_LATELY: Duration = Duration::from_secs(30);

/// What can be handed over, found.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub url: String,
    pub title: String,
    pub position_ms: Option<i64>,
    pub exact: bool,
    /// The media session's app, to pause it.
    pub app: String,
}

impl From<Found> for Handoff {
    fn from(f: Found) -> Handoff {
        Handoff { url: f.url, title: f.title, position_ms: f.position_ms, exact: f.exact, player: f.app }
    }
}

/// The title the phone is offered, kept fresh on a thread while phones ask (the stats ask every
/// few seconds and mustn't wait on UI Automation).
#[derive(Default)]
pub struct Offer {
    state: Mutex<OfferState>,
}

#[derive(Default)]
struct OfferState {
    title: Option<String>,
    asked: Option<Instant>,
    running: bool,
}

impl Offer {
    pub fn title(self: &Arc<Self>) -> Option<String> {
        let mut state = self.state.lock().unwrap();
        state.asked = Some(Instant::now());
        if !state.running {
            state.running = true;
            let me = self.clone();
            if std::thread::Builder::new().name("vyoma-handoff".into()).spawn(move || me.refresh()).is_err() {
                state.running = false;
            }
        }
        state.title.clone()
    }

    fn refresh(&self) {
        // SAFETY: this thread joins the multithreaded apartment for the COM calls.
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        loop {
            let title = find().map(|f| f.title);
            let mut state = self.state.lock().unwrap();
            state.title = title;
            if state.asked.is_none_or(|a| a.elapsed() > ASKED_LATELY) {
                state.running = false;
                state.title = None;
                return;
            }
            drop(state);
            std::thread::sleep(REFRESH);
        }
    }
}

/// The video to continue: a playing one first, then paused ones, each only from a browser window
/// that shows it.
pub fn find() -> Option<Found> {
    // SAFETY: as in [Offer::refresh]; again on a thread that already joined is fine.
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let mut sessions = media::sessions_now();
    sessions.retain(|s| !s.title.trim().is_empty());
    if sessions.is_empty() {
        return None;
    }
    sessions.sort_by_key(|s| !s.playing);
    let windows = browser_windows();
    if windows.is_empty() {
        return None;
    }
    // SAFETY: creating UI Automation's in-process client.
    let uia: IUIAutomation = unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }.ok()?;
    for s in &sessions {
        let wanted = s.title.to_lowercase();
        for w in windows.iter().filter(|w| w.title.to_lowercase().contains(&wanted) && belongs(&s.app, &w.program)) {
            if let Some(url) = address(&uia, w.hwnd) {
                let (position_ms, exact) = if s.reported {
                    (Some(s.pos_ms), true)
                } else {
                    let t = vyoma_engine::media::url_time_ms(&url);
                    ((t > 0).then_some(t), false)
                };
                return Some(Found { url, title: s.title.clone(), position_ms, exact, app: s.app.clone() });
            }
        }
    }
    None
}

/// Whether the media session of app id [app] can be browser [program]'s: Chromium's carry the
/// browser's name ("Chrome", "MSEdge", "Brave..."), Firefox's a 16-digit hash.
fn belongs(app: &str, program: &str) -> bool {
    let app = app.to_lowercase();
    let hashed = app.len() == 16 && app.chars().all(|c| c.is_ascii_hexdigit());
    app.contains(program)
        || (FIREFOX_LIKE.contains(&program) && hashed)
        || (program == "msedge" && app.contains("edge"))
        || (program == "arc" && app.contains("browsercompany"))
}

struct Window {
    hwnd: isize,
    title: String,
    program: String,
}

/// Visible top-level windows of the browsers, with their titles.
fn browser_windows() -> Vec<Window> {
    unsafe extern "system" fn each(hwnd: windows_sys::Win32::Foundation::HWND, out: LPARAM) -> windows_sys::core::BOOL {
        // SAFETY: [out] is the Vec passed to EnumWindows below, alive for the whole call.
        let out = unsafe { &mut *(out as *mut Vec<Window>) };
        // SAFETY: a window handle Windows just gave us, and buffers we own.
        unsafe {
            if IsWindowVisible(hwnd) == 0 {
                return 1;
            }
            let mut text = [0u16; 512];
            let n = GetWindowTextW(hwnd, text.as_mut_ptr(), text.len() as i32);
            if n <= 0 {
                return 1;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if let Some(program) = program_of(pid).filter(|p| BROWSERS.contains(&p.as_str())) {
                out.push(Window { hwnd: hwnd as isize, title: String::from_utf16_lossy(&text[..n as usize]), program });
            }
        }
        1
    }
    let mut out: Vec<Window> = Vec::new();
    // SAFETY: the callback only touches [out], which outlives the call.
    unsafe { EnumWindows(Some(each), &mut out as *mut Vec<Window> as LPARAM) };
    out
}

/// A process's program name, lowercase, without ".exe" ("chrome").
fn program_of(pid: u32) -> Option<String> {
    // SAFETY: a limited query handle, closed below; the buffer is ours.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }
        let mut path = [0u16; 1024];
        let mut len = path.len() as u32;
        let ok = QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut len) != 0;
        CloseHandle(process);
        ok.then(|| program_name(&String::from_utf16_lossy(&path[..len as usize])))
    }
}

fn program_name(path: &str) -> String {
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path).to_lowercase();
    file.strip_suffix(".exe").unwrap_or(&file).to_owned()
}

/// The address in window [hwnd]'s address bar: breadth first through the browser's own controls
/// (pages are skipped: their text boxes aren't the bar), the first edit box holding a web address.
fn address(uia: &IUIAutomation, hwnd: isize) -> Option<String> {
    // SAFETY: UI Automation calls on elements it hands back; the window may close meanwhile,
    // which only makes the calls fail.
    unsafe {
        let root = uia.ElementFromHandle(HWND(hwnd as _)).ok()?;
        let walker = uia.ControlViewWalker().ok()?;
        let mut queue: VecDeque<IUIAutomationElement> = VecDeque::from([root]);
        let mut seen = 0;
        while let Some(e) = queue.pop_front() {
            seen += 1;
            if seen > MOST_ELEMENTS {
                break;
            }
            let kind = e.CurrentControlType().unwrap_or_default();
            if kind == UIA_DocumentControlTypeId {
                continue;
            }
            if kind == UIA_EditControlTypeId
                && let Ok(value) = e.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                && let Some(url) = value.CurrentValue().ok().and_then(|v| web_url(&v.to_string()))
            {
                return Some(url);
            }
            let mut child = walker.GetFirstChildElement(&e).ok();
            while let Some(c) = child {
                child = walker.GetNextSiblingElement(&c).ok();
                queue.push_back(c);
            }
        }
        None
    }
}

/// An address bar's text as a web address: browsers hide "https://" ("youtube.com/watch?v=..."),
/// and what someone is typing there isn't one.
fn web_url(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() || text.chars().any(char::is_whitespace) {
        return None;
    }
    let lower = text.to_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") {
        return Some(text.to_owned());
    }
    if text.contains("://") {
        return None;
    }
    let host = text.split(['/', '?', '#']).next().unwrap_or_default();
    let name = host.split(':').next().unwrap_or_default();
    let looks_like_host = name.contains('.') && !name.starts_with('.') && !name.ends_with('.') && name.chars().all(|c| c.is_alphanumeric() || c == '.' || c == '-');
    looks_like_host.then(|| format!("https://{text}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_bars_read_as_web_addresses() {
        assert_eq!(web_url("youtube.com/watch?v=abc&t=30s").as_deref(), Some("https://youtube.com/watch?v=abc&t=30s"));
        assert_eq!(web_url(" https://www.youtube.com/watch?v=abc ").as_deref(), Some("https://www.youtube.com/watch?v=abc"));
        assert_eq!(web_url("http://192.168.1.4:8096/web").as_deref(), Some("http://192.168.1.4:8096/web"));
        assert_eq!(web_url("vimeo.com:443/1").as_deref(), Some("https://vimeo.com:443/1"));
        assert_eq!(web_url("cat videos"), None, "typing a search");
        assert_eq!(web_url("about:blank"), None);
        assert_eq!(web_url("file:///C:/movie.mp4"), None);
        assert_eq!(web_url("chrome://settings"), None);
        assert_eq!(web_url("localhost"), None);
        assert_eq!(web_url(""), None);
    }

    #[test]
    fn sessions_belong_to_their_browser() {
        assert!(belongs("Chrome", "chrome"));
        assert!(belongs("Chrome.UZVAQ6WUGBXC5BOOQ5WPNOB3JE", "chrome"));
        assert!(belongs("MSEdge", "msedge"));
        assert!(belongs("Microsoft.MicrosoftEdge.Stable_8wekyb3d8bbwe!App", "msedge"));
        assert!(belongs("308046B0AF4A39CB", "firefox"));
        assert!(belongs("Brave.ABCDEF", "brave"));
        assert!(belongs("TheBrowserCompany.Arc_ttt1ap7aakyb4!Arc", "arc"));
        assert!(!belongs("Spotify.exe", "chrome"), "a song with the tab's title isn't the tab");
        assert!(!belongs("308046B0AF4A39CB", "chrome"));
    }

    #[test]
    fn program_names() {
        assert_eq!(program_name(r"C:\Program Files\Google\Chrome\Application\chrome.exe"), "chrome");
        assert_eq!(program_name(r"C:\Program Files\Mozilla Firefox\FIREFOX.EXE"), "firefox");
    }

    /// The address bar is found through UI Automation in a real window: a page's own text box
    /// (inside a document) never counts, the bar outside it does.
    #[test]
    fn reads_a_windows_address_bar() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE};
        let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
        // SAFETY: windows of stock classes made and destroyed on this thread.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let Ok(uia) = CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER) else {
                return eprintln!("no UI Automation here");
            };
            // Wine's reads a window but walks no further: this runs on Windows (CI's runners).
            if uia.ControlViewWalker().is_err() {
                return eprintln!("UI Automation here can't walk a window's controls; skipped");
            }
            let top = CreateWindowExW(0, wide("STATIC").as_ptr(), wide("Big Buck Bunny - YouTube - Browser").as_ptr(), WS_OVERLAPPEDWINDOW | WS_VISIBLE, 0, 0, 400, 300, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null());
            assert!(!top.is_null());
            let search = CreateWindowExW(0, wide("EDIT").as_ptr(), wide("cat videos").as_ptr(), WS_CHILD | WS_VISIBLE, 0, 0, 200, 20, top, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null());
            let bar = CreateWindowExW(0, wide("EDIT").as_ptr(), wide("youtube.com/watch?v=aqz-KE-bpKQ&t=95s").as_ptr(), WS_CHILD | WS_VISIBLE, 0, 30, 200, 20, top, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null());
            assert!(!search.is_null() && !bar.is_null());
            let found = address(&uia, top as isize);
            DestroyWindow(top);
            assert_eq!(found.as_deref(), Some("https://youtube.com/watch?v=aqz-KE-bpKQ&t=95s"), "the text box being typed in is skipped");
            assert_eq!(vyoma_engine::media::url_time_ms(found.as_deref().unwrap()), 95_000);
        }
    }

    /// Under Wine there are no media sessions: nothing to offer, and no panic on the way.
    #[test]
    fn nothing_playing_offers_nothing() {
        assert_eq!(find(), None);
    }
}
