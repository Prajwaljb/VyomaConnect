//! Handoff on the Mac: the web video in a browser here, continued on the phone. macOS has no
//! public way to see what a browser plays, so the browsers already running are asked for their
//! front window's tab through AppleScript (Safari and the Chromium family: Chrome, Brave, Edge,
//! Vivaldi, Arc, Chromium, Opera; Firefox has no scripting). macOS asks once per browser whether
//! Vyoma may control it. Where the browser also allows JavaScript from Apple Events (Safari: Develop
//! menu; Chrome: View > Developer), the page's video says whether it plays and exactly where, and
//! Continue pauses it here; otherwise a known video page (YouTube, Vimeo, Twitch, Dailymotion) is
//! offered at the address's `t=`. The offer is refreshed every 3 s while phones asked within 30 s.
//! The scripts are Vyoma's own text with a browser name from the list below, never a page's text.
//! Built and tested on CI (the parsing and choosing); not yet run on a Mac.

use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use vyoma_engine::media::url_time_ms;
use vyoma_engine::platform::Handoff;

const REFRESH: Duration = Duration::from_secs(3);
const ASKED_LATELY: Duration = Duration::from_secs(30);
/// Safari and the browsers that speak Chrome's AppleScript, by their app (and process) names.
const SAFARI: &str = "Safari";
const CHROMIUMS: [&str; 7] = [
    "Google Chrome",
    "Brave Browser",
    "Microsoft Edge",
    "Vivaldi",
    "Arc",
    "Chromium",
    "Opera",
];
/// The page's playing video (or else one partway through): "1 <ms>" playing, "0 <ms>" paused, or "".
const VIDEO_JS: &str = "(function(){var v=Array.prototype.slice.call(document.querySelectorAll('video'));\
var p=v.filter(function(x){return !x.paused&&!x.ended})[0]||v.filter(function(x){return x.currentTime>0})[0];\
return p?(p.paused?'0 ':'1 ')+Math.round(p.currentTime*1000):''})()";
const PAUSE_JS: &str =
    "(function(){document.querySelectorAll('video').forEach(function(x){x.pause()});return ''})()";

/// What a browser's front tab showed.
#[derive(Debug, PartialEq)]
struct Tab {
    browser: &'static str,
    url: String,
    title: String,
    /// From the page itself: (playing, position in ms); None when the browser keeps JavaScript off.
    video: Option<(bool, i64)>,
}

/// The browser's AppleScript: the front tab's address, title and the page's answer to [js], one a line.
fn script(browser: &str, js: &str) -> String {
    let (tab, title, run) = if browser == SAFARI {
        (
            "current tab of front window",
            "name",
            format!("do JavaScript \"{js}\" in t"),
        )
    } else {
        (
            "active tab of front window",
            "title",
            format!("execute t javascript \"{js}\""),
        )
    };
    format!(
        "tell application \"{browser}\"\n\
         if (count of windows) is 0 then return \"\"\n\
         set t to {tab}\n\
         set u to URL of t\n\
         set n to {title} of t\n\
         set j to \"\"\n\
         try\n\
         set j to ({run}) as text\n\
         end try\n\
         return u & linefeed & n & linefeed & j\n\
         end tell"
    )
}

fn running(app: &str) -> bool {
    Command::new("pgrep")
        .args(["-xq", app])
        .stdin(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn osascript(script: &str) -> Option<String> {
    let out = Command::new("osascript")
        .args(["-e", script])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn parse(browser: &'static str, out: &str) -> Option<Tab> {
    let mut lines = out.trim_end_matches(['\n', '\r']).splitn(3, '\n');
    let url = lines.next()?.trim().to_owned();
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return None;
    }
    let title = lines.next().unwrap_or_default().trim().to_owned();
    let video = lines.next().and_then(|j| {
        let (playing, ms) = j.trim().split_once(' ')?;
        Some((playing == "1", ms.parse().ok()?))
    });
    Some(Tab {
        browser,
        url,
        title,
        video,
    })
}

/// A page that is a video by its address alone (for browsers that keep JavaScript off).
fn video_page(url: &str) -> bool {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.trim_start_matches("www.").trim_start_matches("m.");
    match host {
        "youtube.com" | "music.youtube.com" => {
            path.starts_with("watch?") || path.starts_with("shorts/") || path.starts_with("live/")
        }
        "youtu.be" => !path.is_empty(),
        "vimeo.com" => path
            .split(['/', '?'])
            .next()
            .is_some_and(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())),
        "twitch.tv" => path.starts_with("videos/") || (!path.is_empty() && !path.contains('/')),
        "dailymotion.com" => path.starts_with("video/"),
        _ => false,
    }
}

/// The one to offer: a playing video first, then one paused partway, then a video page.
fn choose(tabs: Vec<Tab>) -> Option<Tab> {
    let rank = |t: &Tab| match t.video {
        Some((true, _)) => 0,
        Some((false, ms)) if ms > 0 => 1,
        None if video_page(&t.url) => 2,
        _ => 3,
    };
    tabs.into_iter().filter(|t| rank(t) < 3).min_by_key(rank)
}

fn tabs() -> Vec<Tab> {
    std::iter::once(SAFARI)
        .chain(CHROMIUMS)
        .filter(|b| running(b))
        .filter_map(|b| osascript(&script(b, VIDEO_JS)).and_then(|out| parse(b, &out)))
        .collect()
}

/// The video to continue, if a browser shows one.
pub fn find() -> Option<Handoff> {
    let t = choose(tabs())?;
    let (position_ms, exact) = match t.video {
        Some((_, ms)) => (Some(ms), true),
        None => {
            let ms = url_time_ms(&t.url);
            ((ms > 0).then_some(ms), false)
        }
    };
    let title = if t.title.is_empty() {
        t.url.clone()
    } else {
        t.title
    };
    Some(Handoff {
        url: t.url,
        title,
        position_ms,
        exact,
        player: t.browser.to_owned(),
    })
}

/// Continue pauses the video here (only where the browser allows JavaScript from Apple Events).
pub fn pause(browser: &str) {
    if let Some(b) = std::iter::once(SAFARI)
        .chain(CHROMIUMS)
        .find(|b| *b == browser)
    {
        let _ = osascript(&script(b, PAUSE_JS));
    }
}

/// The title offered on the phone's Home, refreshed on a thread while phones keep asking.
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
            if std::thread::Builder::new()
                .name("vyoma-handoff".into())
                .spawn(move || me.refresh())
                .is_err()
            {
                state.running = false;
            }
        }
        state.title.clone()
    }

    fn refresh(&self) {
        loop {
            let title = find().map(|h| h.title);
            let mut state = self.state.lock().unwrap();
            state.title = title;
            if state.asked.is_none_or(|a| a.elapsed() > ASKED_LATELY) {
                (state.running, state.title) = (false, None);
                return;
            }
            drop(state);
            std::thread::sleep(REFRESH);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_name_each_browsers_terms() {
        let s = script("Safari", VIDEO_JS);
        assert!(s.starts_with("tell application \"Safari\""));
        assert!(s.contains("current tab of front window") && s.contains("do JavaScript"));
        let c = script("Brave Browser", PAUSE_JS);
        assert!(c.contains("active tab of front window") && c.contains("execute t javascript"));
        // The page's code sits in an AppleScript string: no double quotes of its own.
        assert!(!VIDEO_JS.contains('"') && !PAUSE_JS.contains('"'));
    }

    #[test]
    fn reads_what_a_browser_said() {
        let t = parse(
            "Safari",
            "https://www.youtube.com/watch?v=abc\nCats - YouTube\n1 95000\n",
        )
        .unwrap();
        assert_eq!(t.video, Some((true, 95000)));
        assert_eq!(t.title, "Cats - YouTube");
        // JavaScript off: no third line's answer.
        let off = parse("Google Chrome", "https://vimeo.com/123\nA film\n\n").unwrap();
        assert_eq!(off.video, None);
        // No window, or a page that isn't on the web.
        assert!(parse("Arc", "\n").is_none());
        assert!(parse("Arc", "file:///Users/x/a.mp4\nx\n").is_none());
    }

    #[test]
    fn knows_video_pages() {
        for url in [
            "https://www.youtube.com/watch?v=a&t=30",
            "https://youtu.be/abc",
            "https://m.youtube.com/shorts/x",
            "https://vimeo.com/76979871",
            "https://www.twitch.tv/somebody",
            "https://www.dailymotion.com/video/x8",
        ] {
            assert!(video_page(url), "{url}");
        }
        for url in [
            "https://www.youtube.com/",
            "https://vimeo.com/about",
            "https://example.com/watch?v=a",
            "https://www.twitch.tv/directory/game",
        ] {
            assert!(!video_page(url), "{url}");
        }
    }

    #[test]
    fn a_playing_video_comes_first() {
        let tab = |browser, url: &str, video| Tab {
            browser,
            url: url.into(),
            title: String::new(),
            video,
        };
        let picked = choose(vec![
            tab("Safari", "https://youtu.be/a", None),
            tab("Arc", "https://news.example/", Some((false, 0))),
            tab(
                "Google Chrome",
                "https://example.com/clip",
                Some((true, 4000)),
            ),
        ]);
        assert_eq!(picked.unwrap().browser, "Google Chrome");
        let paused = choose(vec![
            tab("Safari", "https://youtu.be/a", None),
            tab("Arc", "https://x.example/", Some((false, 7000))),
        ]);
        assert_eq!(paused.unwrap().browser, "Arc");
        assert!(choose(vec![tab("Safari", "https://example.com/", None)]).is_none());
    }
}
