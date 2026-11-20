//! This PC's players and sound for the phone, as the Linux app's MPRIS and volume plugins:
//! `kdeconnect.mpris(.request)` for every app that shows in Windows' own media controls (the
//! volume flyout's player: Spotify, browsers, Media Player, VLC...), through the system media
//! transport controls (`GlobalSystemMediaTransportControlsSessionManager`), and
//! `kdeconnect.systemvolume(.request)` for the default speaker's volume through Core Audio.
//!
//! Read every [EVERY] while a phone listens. Covers come as payloads (Windows hands over the
//! picture, not a link), under a made-up `file://` name that changes with the track, so the phone
//! asks for each new one once. Players have no volume of their own here: theirs is the speaker's.
//! Built and tested on CI and under Wine only; not yet run on a real Windows PC.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::mpsc;
use windows::ApplicationModel::AppInfo;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session, GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};
use windows::Media::MediaPlaybackAutoRepeatMode;
use windows::Storage::Streams::DataReader;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{IMMDeviceEnumerator, MMDeviceEnumerator, eMultimedia, eRender};
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToBSTR};
use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, STGM_READ};
use windows::core::HSTRING;

pub use vyoma_engine::media::{MPRIS, MPRIS_REQUEST, Track, VOLUME, VOLUME_REQUEST};
use vyoma_engine::media::{self, Do, Repeat, Sink, changed, track_packet};

/// How often the players and the volume are read while a phone listens (cheap calls, no polling
/// of the apps themselves).
pub const EVERY: Duration = Duration::from_secs(1);
/// The one output the phone is shown: Windows' default speaker, whichever that is.
const SINK: &str = "default";
/// Covers past this aren't sent (the phone refuses more than a few MiB anyway).
const MAX_ART: usize = 4 << 20;
/// Windows' clock (100 ns since 1601) at the Unix epoch.
const EPOCH_TICKS: i64 = 116_444_736_000_000_000;

/// One app in Windows' media controls: the phone's name for it, and Windows' (its app id).
#[derive(Clone, Debug, PartialEq)]
pub struct Player {
    pub name: String,
    pub app: String,
    pub track: Track,
}

/// The speaker: percent, muted, and its name as Windows shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sound {
    pub percent: i64,
    pub muted: bool,
    pub name: String,
}

/// What goes to the phones: (to whom, or every phone that takes it; packet), or a cover.
pub enum Out {
    Packet(Option<String>, Value),
    Art { id: String, packet: Value, bytes: Vec<u8> },
}

/// A name for an app id: Windows' own display name where it knows one, else the program's name
/// ("Spotify.exe" is Spotify). Firefox's id is a hash of where it's installed (16 hex digits).
fn app_name(app: &str) -> String {
    if let Ok(name) = AppInfo::GetFromAppUserModelId(&HSTRING::from(app)).and_then(|i| i.DisplayInfo()).and_then(|d| d.DisplayName()) {
        let name = name.to_string_lossy();
        if !name.trim().is_empty() {
            return name.trim().to_owned();
        }
    }
    guess_name(app)
}

fn guess_name(app: &str) -> String {
    if app.len() == 16 && app.chars().all(|c| c.is_ascii_hexdigit()) {
        return "Firefox".into();
    }
    // A packaged app's id ends in its own name after the "!": Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic.
    let base = app.rsplit(['!', '\\', '/']).next().unwrap_or(app);
    let base = base.strip_suffix(".exe").or_else(|| base.strip_suffix(".EXE")).unwrap_or(base);
    let base = base.rsplit('.').next().unwrap_or(base);
    match base.to_ascii_lowercase().as_str() {
        "" => "Player".into(),
        "chrome" => "Chrome".into(),
        "msedge" => "Edge".into(),
        "zunemusic" => "Media Player".into(),
        "vlc" => "VLC".into(),
        _ => {
            let mut c = base.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
        }
    }
}

/// Names for the phone, told apart as the Linux app does: " [2]" on a second one.
fn names(apps: &[(String, String)]) -> Vec<String> {
    let mut seen: HashMap<&str, usize> = HashMap::new();
    apps.iter()
        .map(|(_, name)| {
            let n = seen.entry(name.as_str()).or_default();
            *n += 1;
            if *n == 1 { name.clone() } else { format!("{name} [{n}]") }
        })
        .collect()
}

fn ms(ticks: i64) -> i64 {
    ticks / 10_000
}

fn now_ticks() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as i64 / 100).unwrap_or(0) + EPOCH_TICKS
}

/// Where a playing track is now: apps tell Windows the position only now and then (browsers
/// rarely), with the time they did, so the time since is added.
fn position(pos_ms: i64, length_ms: i64, playing: bool, updated_ticks: i64, now_ticks: i64) -> i64 {
    let mut pos = pos_ms;
    if playing && updated_ticks > 0 && now_ticks > updated_ticks {
        pos += ms(now_ticks - updated_ticks);
    }
    if length_ms > 0 { pos.clamp(0, length_ms) } else { pos.max(0) }
}

/// The cover's name for the phone: changes with the picture (FNV-1a of its bytes).
fn art_name(bytes: &[u8]) -> String {
    let hash = bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3));
    format!("file:///vyoma-art/{hash:016x}.img")
}

fn list_packet(players: &[Player]) -> Value {
    let names: Vec<&str> = players.iter().map(|p| p.name.as_str()).collect();
    media::list_packet(&names, true)
}

/// The speaker as the phone's one output.
fn sink(s: &Sound) -> Sink {
    Sink::from_percent(SINK, if s.name.is_empty() { "This PC" } else { &s.name }, s.percent, s.muted)
}

pub fn sinks_packet(s: Option<&Sound>) -> Value {
    media::sinks_packet(&s.map(sink).into_iter().collect::<Vec<_>>())
}

pub fn sound_packet(s: &Sound) -> Value {
    sink(s).packet()
}

struct Cover {
    track: String,
    url: String,
    bytes: Vec<u8>,
}

/// Windows' media controls, once asked for (None where there are none: before Windows 10 1809, Wine).
struct Controls {
    manager: Option<Manager>,
    /// Each player's cover by its name, with the track it's for and its name for the phone: read
    /// once per track.
    covers: HashMap<String, Cover>,
}

impl Controls {
    fn new() -> Controls {
        let manager = Manager::RequestAsync().and_then(|a| a.join());
        if let Err(e) = &manager {
            tracing::info!("no media controls on this PC: {e}");
        }
        Controls { manager: manager.ok(), covers: HashMap::new() }
    }

    fn sessions(&self) -> Vec<Session> {
        let Some(m) = &self.manager else { return Vec::new() };
        m.GetSessions().map(|v| v.into_iter().collect()).unwrap_or_default()
    }

    /// Every player and what it's doing; [volume] is the speaker's.
    fn players(&mut self, volume: i64) -> Vec<Player> {
        let sessions = self.sessions();
        let apps: Vec<(String, String)> = sessions.iter().map(|s| {
            let app = s.SourceAppUserModelId().map(|a| a.to_string_lossy()).unwrap_or_default();
            let name = app_name(&app);
            (app, name)
        }).collect();
        let now = now_ticks();
        let mut out = Vec::new();
        let mut covers = HashMap::new();
        for ((session, (app, _)), name) in sessions.iter().zip(&apps).zip(names(&apps)) {
            let Some(mut track) = read(session, now) else { continue };
            track.volume = volume;
            // The cover again only for a new track (its words changed).
            let key = format!("{}\u{1f}{}\u{1f}{}", track.title, track.artist, track.album);
            let known = self.covers.remove(&name).filter(|c| c.track == key);
            if let Some(c) = known.or_else(|| cover(session).map(|bytes| Cover { track: key, url: art_name(&bytes), bytes })) {
                track.art = c.url.clone();
                covers.insert(name.clone(), c);
            }
            out.push(Player { name, app: app.clone(), track });
        }
        self.covers = covers;
        out
    }

    /// The cover named [url] of player [name], if it's still the one shown.
    fn cover_named(&self, name: &str, url: &str) -> Option<Vec<u8>> {
        self.covers.get(name).filter(|c| c.url == url).map(|c| c.bytes.clone())
    }

    fn apply(&self, app: &str, todo: &[Do]) {
        let Some(session) = self.sessions().into_iter().find(|s| s.SourceAppUserModelId().is_ok_and(|a| a.to_string_lossy() == app)) else { return };
        for d in todo {
            let done = match d {
                Do::PlayPause => session.TryTogglePlayPauseAsync().and_then(|a| a.join()),
                Do::Play => session.TryPlayAsync().and_then(|a| a.join()),
                Do::Pause => session.TryPauseAsync().and_then(|a| a.join()),
                Do::Stop => session.TryStopAsync().and_then(|a| a.join()),
                Do::Next => session.TrySkipNextAsync().and_then(|a| a.join()),
                Do::Previous => session.TrySkipPreviousAsync().and_then(|a| a.join()),
                Do::SetPosition(ms) => {
                    let start = session.GetTimelineProperties().and_then(|t| t.StartTime()).map(|t| t.Duration).unwrap_or(0);
                    session.TryChangePlaybackPositionAsync(start + ms * 10_000).and_then(|a| a.join())
                }
                Do::Seek(us) => {
                    let pos = session.GetTimelineProperties().and_then(|t| t.Position()).map(|t| t.Duration).unwrap_or(0);
                    session.TryChangePlaybackPositionAsync((pos + us * 10).max(0)).and_then(|a| a.join())
                }
                Do::Shuffle(on) => session.TryChangeShuffleActiveAsync(*on).and_then(|a| a.join()),
                Do::Repeat(r) => session.TryChangeAutoRepeatModeAsync(repeat_mode(*r)).and_then(|a| a.join()),
                Do::Volume(v) => {
                    set_sound(Some(*v), None);
                    Ok(true)
                }
            };
            if !done.unwrap_or(false) {
                tracing::info!("the player didn't take {d:?}");
            }
        }
    }
}

fn repeat_mode(r: Repeat) -> MediaPlaybackAutoRepeatMode {
    match r {
        Repeat::None => MediaPlaybackAutoRepeatMode::None,
        Repeat::Track => MediaPlaybackAutoRepeatMode::Track,
        Repeat::Playlist => MediaPlaybackAutoRepeatMode::List,
    }
}

/// One session's state; None while it has nothing loaded.
fn read(s: &Session, now: i64) -> Option<Track> {
    let info = s.GetPlaybackInfo().ok()?;
    let status = info.PlaybackStatus().ok()?;
    if status == Status::Closed {
        return None;
    }
    let props = s.TryGetMediaPropertiesAsync().and_then(|a| a.join()).ok();
    let text = |f: Option<windows::core::Result<HSTRING>>| f.and_then(|r| r.ok()).map(|h| h.to_string_lossy().trim().to_owned()).unwrap_or_default();
    let controls = info.Controls().ok();
    let can = |f: fn(&windows::Media::Control::GlobalSystemMediaTransportControlsSessionPlaybackControls) -> windows::core::Result<bool>| controls.as_ref().and_then(|c| f(c).ok()).unwrap_or(false);
    let timeline = s.GetTimelineProperties().ok();
    let span = |f: fn(&windows::Media::Control::GlobalSystemMediaTransportControlsSessionTimelineProperties) -> windows::core::Result<windows::Foundation::TimeSpan>| timeline.as_ref().and_then(|t| f(t).ok()).map_or(0, |t| t.Duration);
    let (start, end, pos) = (span(|t| t.StartTime()), span(|t| t.EndTime()), span(|t| t.Position()));
    let updated = timeline.as_ref().and_then(|t| t.LastUpdatedTime().ok()).map_or(0, |d| d.UniversalTime);
    let playing = status == Status::Playing;
    let length_ms = ms((end - start).max(0));
    let artist = text(props.as_ref().map(|p| p.Artist()));
    Some(Track {
        playing,
        title: text(props.as_ref().map(|p| p.Title())),
        // Videos in browsers often have no artist but an album artist (the channel).
        artist: if artist.is_empty() { text(props.as_ref().map(|p| p.AlbumArtist())) } else { artist },
        album: text(props.as_ref().map(|p| p.AlbumTitle())),
        length_ms,
        pos_ms: position(ms(pos - start), length_ms, playing, updated, now),
        can_play: can(|c| c.IsPlayEnabled()) || can(|c| c.IsPlayPauseToggleEnabled()),
        can_pause: can(|c| c.IsPauseEnabled()) || can(|c| c.IsPlayPauseToggleEnabled()),
        can_next: can(|c| c.IsNextEnabled()),
        can_previous: can(|c| c.IsPreviousEnabled()),
        can_seek: can(|c| c.IsPlaybackPositionEnabled()),
        shuffle: info.IsShuffleActive().ok().and_then(|r| r.Value().ok()),
        repeat: info.AutoRepeatMode().ok().and_then(|r| r.Value().ok()).map(|m| match m {
            MediaPlaybackAutoRepeatMode::Track => Repeat::Track,
            MediaPlaybackAutoRepeatMode::List => Repeat::Playlist,
            _ => Repeat::None,
        }),
        ..Default::default()
    })
}

/// The session's cover as Windows has it (a PNG or JPEG), if any.
fn cover(s: &Session) -> Option<Vec<u8>> {
    let props = s.TryGetMediaPropertiesAsync().and_then(|a| a.join()).ok()?;
    let stream = props.Thumbnail().ok()?.OpenReadAsync().ok()?.join().ok()?;
    let size = stream.Size().ok()? as usize;
    if size == 0 || size > MAX_ART {
        return None;
    }
    let reader = DataReader::CreateDataReader(&stream).ok()?;
    let got = reader.LoadAsync(size as u32).ok()?.join().ok()? as usize;
    let mut bytes = vec![0u8; got];
    reader.ReadBytes(&mut bytes).ok()?;
    Some(bytes)
}

fn speaker() -> windows::core::Result<(IAudioEndpointVolume, String)> {
    // SAFETY: COM calls on the media thread, which joined the multithreaded apartment ([run]);
    // the property's value is cleared after it's read.
    unsafe {
        let devices: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = devices.GetDefaultAudioEndpoint(eRender, eMultimedia)?;
        let volume: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None)?;
        let name = device
            .OpenPropertyStore(STGM_READ)
            .and_then(|store| store.GetValue(&PKEY_Device_FriendlyName))
            .map(|mut v| {
                let s = PropVariantToBSTR(&v).map(|b| b.to_string()).unwrap_or_default();
                let _ = PropVariantClear(&mut v);
                s
            })
            .unwrap_or_default();
        Ok((volume, name))
    }
}

/// The default speaker a step up or down, as its volume keys do (the phone's volume keys while
/// its sound plays here, from the mirror). Blocking: run off the async threads.
pub fn step_volume(up: bool) {
    // SAFETY: COM on this thread, then the default speaker's own volume step.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let Ok((volume, _)) = speaker() else { return };
        let done = if up { volume.VolumeStepUp(std::ptr::null()) } else { volume.VolumeStepDown(std::ptr::null()) };
        if let Err(e) = done {
            tracing::info!("couldn't change the volume: {e}");
        }
    }
}

/// The default speaker; None when there's none (no sound device, or Windows' audio stopped).
fn sound() -> Option<Sound> {
    let (volume, name) = speaker().ok()?;
    // SAFETY: as in [speaker].
    unsafe {
        let level = volume.GetMasterVolumeLevelScalar().ok()?;
        let muted = volume.GetMute().ok()?.as_bool();
        Some(Sound { percent: ((level * 100.0).round() as i64).clamp(0, 100), muted, name })
    }
}

fn set_sound(percent: Option<i64>, muted: Option<bool>) {
    let Ok((volume, _)) = speaker() else {
        tracing::info!("no speaker to change");
        return;
    };
    // SAFETY: as in [speaker].
    unsafe {
        if let Some(p) = percent
            && let Err(e) = volume.SetMasterVolumeLevelScalar(p as f32 / 100.0, std::ptr::null())
        {
            tracing::info!("couldn't set the volume: {e}");
        }
        if let Some(m) = muted
            && let Err(e) = volume.SetMute(m, std::ptr::null())
        {
            tracing::info!("couldn't mute: {e}");
        }
    }
}

/// For walk-away rules and calls, on any thread: what's playing paused; the apps it paused.
pub fn pause_playing() -> Vec<String> {
    // SAFETY: this thread joins the multithreaded apartment (again is fine) for the COM calls.
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let controls = Controls::new();
    let mut paused = Vec::new();
    for s in controls.sessions() {
        let playing = s.GetPlaybackInfo().and_then(|i| i.PlaybackStatus()).is_ok_and(|st| st == Status::Playing);
        if playing && s.TryPauseAsync().and_then(|a| a.join()).unwrap_or(false) {
            paused.push(s.SourceAppUserModelId().map(|a| a.to_string_lossy()).unwrap_or_default());
        }
    }
    paused
}

/// Plays again what [pause_playing] paused (apps by their id), if they're still there.
pub fn resume(apps: &[String]) {
    // SAFETY: as in [pause_playing].
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let controls = Controls::new();
    for app in apps {
        controls.apply(app, &[Do::Play]);
    }
}

/// Mutes the speaker; true if that changed anything.
pub fn mute_if_on() -> bool {
    // SAFETY: as in [pause_playing].
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if sound().is_some_and(|s| !s.muted) {
        set_sound(None, Some(true));
        return true;
    }
    false
}

pub fn unmute() {
    // SAFETY: as in [pause_playing].
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    set_sound(None, Some(false));
}

/// What the phones are told after [old] became [new]: the list if players came or went (and each
/// one's state), else the state of each that changed.
fn tell(old: &[Player], new: &[Player], elapsed_ms: i64) -> Vec<Value> {
    let names = |l: &[Player]| l.iter().map(|p| p.name.clone()).collect::<Vec<_>>();
    if names(old) != names(new) {
        return std::iter::once(list_packet(new)).chain(new.iter().map(|p| track_packet(&p.name, &p.track))).collect();
    }
    old.iter().zip(new).filter(|(a, b)| changed(&a.track, &b.track, elapsed_ms)).map(|(_, p)| track_packet(&p.name, &p.track)).collect()
}

/// Follows the players and the volume while [wanted] (a phone that takes them is linked) and
/// answers [requests] (device id, packet). Windows' calls block, so all of it runs on a thread of
/// its own.
pub fn run(wanted: Arc<AtomicBool>, requests: std::sync::mpsc::Receiver<(String, Value)>, out: mpsc::UnboundedSender<Out>) {
    use std::sync::mpsc::RecvTimeoutError;
    // SAFETY: this thread joins the multithreaded apartment for good (Core Audio and WinRT need one).
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let mut controls: Option<Controls> = None;
    let mut known: Vec<Player> = Vec::new();
    let mut volume: Option<Sound> = None;
    let mut last = std::time::Instant::now();
    loop {
        // A request, or the next look after [EVERY].
        let asked = match requests.recv_timeout(EVERY) {
            Ok(r) => Some(r),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        if asked.is_none() && !wanted.load(Ordering::Relaxed) {
            continue;
        }
        let c = controls.get_or_insert_with(Controls::new);
        // Changes first, so what's read next shows them.
        if let Some((_, p)) = &asked {
            let body = &p["body"];
            match p["type"].as_str() {
                Some(MPRIS_REQUEST) => {
                    let r = media::request(body);
                    if let Some(player) = known.iter().find(|k| r.player.as_deref() == Some(k.name.as_str())) {
                        c.apply(&player.app, &r.does);
                    }
                }
                Some(VOLUME_REQUEST) => {
                    let r = media::sink_request(body);
                    if r.name.as_deref() == Some(SINK) && (r.volume.is_some() || r.muted.is_some()) {
                        set_sound(r.percent(), r.muted);
                    }
                }
                _ => {}
            }
        }
        let sound_now = sound();
        let now = c.players(sound_now.as_ref().map_or(0, |s| s.percent));
        let elapsed = last.elapsed().as_millis() as i64;
        last = std::time::Instant::now();
        for p in tell(&known, &now, elapsed) {
            let _ = out.send(Out::Packet(None, p));
        }
        if sound_now != volume {
            // A new speaker (headphones in) is a new list; else only the level.
            let p = match (&volume, &sound_now) {
                (Some(a), Some(b)) if a.name == b.name => sound_packet(b),
                _ => sinks_packet(sound_now.as_ref()),
            };
            let _ = out.send(Out::Packet(None, p));
        }
        (known, volume) = (now, sound_now);
        // Then what was asked for.
        let Some((id, p)) = asked else { continue };
        let body = &p["body"];
        if p["type"] == MPRIS_REQUEST {
            let r = media::request(body);
            if r.list {
                let _ = out.send(Out::Packet(Some(id.clone()), list_packet(&known)));
            }
            let Some(player) = known.iter().find(|k| r.player.as_deref() == Some(k.name.as_str())) else { continue };
            if r.now {
                let _ = out.send(Out::Packet(Some(id.clone()), track_packet(&player.name, &player.track)));
            }
            if let Some(url) = &r.art {
                match c.cover_named(&player.name, url) {
                    Some(bytes) => {
                        let _ = out.send(Out::Art { id, packet: media::art_packet(&player.name, url), bytes });
                    }
                    None => tracing::info!("{id}: no such cover for {}", player.name),
                }
            }
        } else if p["type"] == VOLUME_REQUEST && media::sink_request(body).list {
            let _ = out.send(Out::Packet(Some(id), sinks_packet(volume.as_ref())));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn players_get_names_people_know() {
        assert_eq!(guess_name("Spotify.exe"), "Spotify");
        assert_eq!(guess_name("chrome"), "Chrome");
        assert_eq!(guess_name("MSEdge"), "Edge");
        assert_eq!(guess_name("308046B0AF4A39CB"), "Firefox");
        assert_eq!(guess_name("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"), "Media Player");
        assert_eq!(guess_name(r"C:\Program Files\foobar2000\foobar2000.exe"), "Foobar2000");
        assert_eq!(guess_name(""), "Player");
        let apps = [("a".to_owned(), "Chrome".to_owned()), ("b".into(), "Spotify".into()), ("c".into(), "Chrome".into())];
        assert_eq!(names(&apps), ["Chrome", "Spotify", "Chrome [2]"]);
    }

    #[test]
    fn a_playing_track_moves_on_between_reports() {
        let t = EPOCH_TICKS + 1_000_000_000;
        assert_eq!(position(10_000, 200_000, true, t, t + 30_000_000), 13_000, "3 s later");
        assert_eq!(position(10_000, 200_000, false, t, t + 30_000_000), 10_000, "paused");
        assert_eq!(position(199_000, 200_000, true, t, t + 50_000_000), 200_000, "not past the end");
        assert_eq!(position(5_000, 0, true, 0, t), 5_000, "no time given");
        assert!(now_ticks() > EPOCH_TICKS);
    }

    #[test]
    fn repeats_as_windows_has_them() {
        assert_eq!(repeat_mode(Repeat::Playlist), MediaPlaybackAutoRepeatMode::List);
        assert_eq!(repeat_mode(Repeat::Track), MediaPlaybackAutoRepeatMode::Track);
    }

    #[test]
    fn packets_as_the_linux_app_sends_them() {
        let s = Sound { percent: 50, muted: false, name: "Speakers".into() };
        let sinks = sinks_packet(Some(&s));
        assert_eq!((sinks["body"]["sinkList"][0]["volume"].as_i64(), sinks["body"]["sinkList"][0]["description"].as_str()), (Some(media::NORMAL / 2), Some("Speakers")));
        assert_eq!(sinks_packet(Some(&Sound { name: String::new(), ..s }))["body"]["sinkList"][0]["description"], "This PC");
        assert_eq!(sinks_packet(None)["body"]["sinkList"], json!([]));
        let t = Track { title: "Song".into(), artist: "Band".into(), shuffle: Some(true), repeat: Some(Repeat::Track), can_seek: true, length_ms: 1000, ..Default::default() };
        let p = track_packet("Spotify", &t);
        assert_eq!((p["body"]["nowPlaying"].as_str(), p["body"]["shuffle"].as_bool(), p["body"]["loopStatus"].as_str()), (Some("Band - Song"), Some(true), Some("Track")));
        assert_eq!(p["body"]["canSeek"], true);
        assert!(track_packet("x", &Track::default())["body"].get("shuffle").is_none(), "only what Windows says");
        assert_eq!(list_packet(&[])["body"]["supportAlbumArtPayload"], true);
        let a = art_name(b"one");
        assert!(a.starts_with("file:///") && a != art_name(b"two"), "the phone asks for file:// covers, each once");
    }

    #[test]
    fn the_phone_hears_changes_not_ticks() {
        let a = Track { playing: true, title: "A".into(), length_ms: 200_000, pos_ms: 10_000, ..Default::default() };
        assert!(!changed(&a, &Track { pos_ms: 11_000, ..a.clone() }, 1000), "playing on");
        assert!(changed(&a, &Track { pos_ms: 90_000, ..a.clone() }, 1000), "a seek");
        assert!(changed(&a, &Track { playing: false, ..a.clone() }, 1000));
        let p = |name: &str| Player { name: name.into(), app: name.into(), track: a.clone() };
        let sent = tell(&[p("A")], &[p("A"), p("B")], 1000);
        assert_eq!((sent.len(), sent[0]["body"]["playerList"].clone()), (3, json!(["A", "B"])));
        assert!(tell(&[p("A")], &[p("A")], 0).is_empty());
    }

    /// Whatever this PC has (Wine has no media controls): reading never fails, only finds nothing.
    #[test]
    fn reads_what_there_is() {
        let mut c = Controls::new();
        let players = c.players(0);
        eprintln!("players: {}, speaker: {:?}", players.len(), sound().map(|s| (s.percent, s.muted)));
    }
}
