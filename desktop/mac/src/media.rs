//! The Mac's players and sound for the phone, as the Linux app's MPRIS and volume plugins:
//! `kdeconnect.mpris(.request)` for Music and Spotify, `kdeconnect.systemvolume(.request)` for
//! the output volume. macOS has no public "what's playing" for every app, so it's those two
//! through AppleScript (macOS asks the person once whether Vyoma may control each), read every
//! few seconds while a phone is listening. Only a fixed script per action runs: what the phone
//! sends picks one, and numbers go as arguments.
//! Built on CI only; not yet tried on a real Mac.

// Only macOS does anything with most of this; the tests run everywhere.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc;
use vyoma_core::link::packet;

pub const MPRIS: &str = "kdeconnect.mpris";
pub const MPRIS_REQUEST: &str = "kdeconnect.mpris.request";
pub const VOLUME: &str = "kdeconnect.systemvolume";
pub const VOLUME_REQUEST: &str = "kdeconnect.systemvolume.request";

/// How often the players and the volume are read while a phone listens.
pub const EVERY: Duration = Duration::from_secs(3);
/// 100%, as the Linux app (PulseAudio's scale) and kdeconnectd send it.
const NORMAL: i64 = 65536;
/// The one output the phone is shown: macOS's own volume follows the output in use.
const SINK: &str = "mac";
/// Field and record separators in what the scripts print (no title has them).
const US: char = '\u{1f}';

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Player {
    Music,
    Spotify,
}

impl Player {
    pub const ALL: [Player; 2] = [Player::Music, Player::Spotify];

    pub fn name(self) -> &'static str {
        match self {
            Player::Music => "Music",
            Player::Spotify => "Spotify",
        }
    }

    pub fn named(name: &str) -> Option<Player> {
        Self::ALL.into_iter().find(|p| p.name() == name)
    }

    /// Spotify's times are milliseconds, Music's seconds; Spotify has a cover link.
    fn read_script(self) -> String {
        let (scale, art) = match self {
            Player::Music => ("1000", "\"\""),
            Player::Spotify => ("1", "(artwork url of t)"),
        };
        let us = "(character id 31)";
        format!(
            "if application \"{app}\" is running then
tell application \"{app}\"
set s to player state as text
if s is \"stopped\" then return s
set t to current track
return s & {us} & (name of t) & {us} & (artist of t) & {us} & (album of t) & {us} & (((duration of t) * {scale}) as integer) & {us} & ((player position * 1000) as integer) & {us} & sound volume & {us} & {art}
end tell
end if
return \"\"",
            app = self.name()
        )
    }

    /// The script for one of the phone's actions; [None] for anything else.
    fn action_script(self, action: &str) -> Option<String> {
        let verb = match (action, self) {
            ("PlayPause", _) => "playpause",
            ("Play", _) => "play",
            ("Pause", _) | ("Stop", Player::Spotify) => "pause",
            ("Stop", Player::Music) => "stop",
            ("Next", _) => "next track",
            ("Previous", _) => "previous track",
            _ => return None,
        };
        Some(format!("tell application \"{}\" to {verb}", self.name()))
    }
}

/// A player's state, as the phone is told it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
    pub playing: bool,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub length_ms: i64,
    pub pos_ms: i64,
    pub volume: i64,
    pub art: String,
}

/// What [Player::read_script] printed: None when the app isn't open.
fn parse(out: &str) -> Option<Track> {
    let out = out.trim_end_matches(['\n', '\r']);
    if out.is_empty() {
        return None;
    }
    let f: Vec<&str> = out.split(US).collect();
    let n = |i: usize| f.get(i).and_then(|v| v.trim().parse::<f64>().ok()).unwrap_or(0.0) as i64;
    let s = |i: usize| f.get(i).map(|v| v.trim().to_owned()).filter(|v| v != "missing value").unwrap_or_default();
    Some(Track {
        playing: f[0].trim() == "playing",
        title: s(1),
        artist: s(2),
        album: s(3),
        length_ms: n(4),
        pos_ms: n(5),
        volume: n(6).clamp(0, 100),
        // Only a web link: the phone fetches it.
        art: Some(s(7)).filter(|a| a.starts_with("https://")).unwrap_or_default(),
    })
}

/// The packet for [player] in state [t].
pub fn track_packet(player: &str, t: &Track) -> Value {
    packet(
        MPRIS,
        json!({
            "player": player,
            "isPlaying": t.playing,
            "pos": t.pos_ms,
            "length": t.length_ms,
            "canPlay": true,
            "canPause": true,
            "canGoNext": true,
            "canGoPrevious": true,
            "canSeek": t.length_ms > 0,
            "title": t.title,
            "artist": t.artist,
            "album": t.album,
            "albumArtUrl": t.art,
            "url": "",
            "volume": t.volume,
            "nowPlaying": if t.artist.is_empty() { t.title.clone() } else { format!("{} - {}", t.artist, t.title) },
        }),
    )
}

pub fn list_packet(players: &[(Player, Track)]) -> Value {
    let names: Vec<&str> = players.iter().map(|(p, _)| p.name()).collect();
    packet(MPRIS, json!({ "playerList": names, "supportAlbumArtPayload": false }))
}

/// Worth telling the phone: anything but the position moving on as it plays, which it counts
/// itself; a seek (a jump of more than [EVERY] and a bit) is told.
pub fn changed(old: &Track, new: &Track) -> bool {
    let expected = old.pos_ms + if old.playing { EVERY.as_millis() as i64 } else { 0 };
    Track { pos_ms: 0, ..old.clone() } != Track { pos_ms: 0, ..new.clone() } || (new.pos_ms - expected).abs() > 2500
}

/// The output volume: percent and muted. None when the output has no volume (some displays).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sound {
    pub percent: i64,
    pub muted: bool,
}

const SOUND_READ: &str = "set v to get volume settings
return ((output volume of v) as text) & \",\" & ((output muted of v) as text)";

fn parse_sound(out: &str) -> Option<Sound> {
    let (v, m) = out.trim().split_once(',')?;
    Some(Sound { percent: v.trim().parse::<i64>().ok()?.clamp(0, 100), muted: m.trim() == "true" })
}

pub fn sinks_packet(s: Option<Sound>) -> Value {
    let list: Vec<Value> = s.iter().map(|s| json!({ "name": SINK, "description": "This Mac", "volume": s.percent * NORMAL / 100, "maxVolume": NORMAL, "muted": s.muted, "enabled": true })).collect();
    packet(VOLUME, json!({ "sinkList": list }))
}

pub fn sound_packet(s: Sound) -> Value {
    packet(VOLUME, json!({ "name": SINK, "volume": s.percent * NORMAL / 100, "muted": s.muted, "enabled": true }))
}

/// The scripts a volume request runs, with their argument.
fn sound_scripts(body: &Value) -> Vec<(&'static str, Option<String>)> {
    if body["name"].as_str() != Some(SINK) {
        return Vec::new();
    }
    let mut out = Vec::new();
    if let Some(v) = body["volume"].as_i64() {
        out.push(("on run argv\nset volume output volume ((item 1 of argv) as integer)\nend run", Some((v.clamp(0, NORMAL) * 100 / NORMAL).to_string())));
    }
    match body["muted"].as_bool() {
        Some(true) => out.push(("set volume with output muted", None)),
        Some(false) => out.push(("set volume without output muted", None)),
        None => {}
    }
    out
}

/// The scripts a player request runs (an action, volume, a seek), with their argument.
fn player_scripts(p: Player, body: &Value) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    if let Some(s) = body["action"].as_str().and_then(|a| p.action_script(a)) {
        out.push((s, None));
    }
    if let Some(v) = body["setVolume"].as_i64() {
        out.push((format!("on run argv\ntell application \"{}\" to set sound volume to ((item 1 of argv) as integer)\nend run", p.name()), Some(v.clamp(0, 100).to_string())));
    }
    if let Some(ms) = body["SetPosition"].as_i64() {
        out.push((format!("on run argv\ntell application \"{}\" to set player position to ((item 1 of argv) as integer) / 1000\nend run", p.name()), Some(ms.max(0).to_string())));
    }
    out
}

#[cfg(target_os = "macos")]
use mac::{apply_player, apply_sound, players, sound};

/// Elsewhere (this crate's tests on Linux) there are no players.
#[cfg(not(target_os = "macos"))]
mod elsewhere {
    use super::*;
    pub async fn players() -> Vec<(Player, Track)> {
        Vec::new()
    }
    pub async fn sound() -> Option<Sound> {
        None
    }
    pub async fn apply_player(_: Player, _: &Value) {}
    pub async fn apply_sound(_: &Value) {}
}
#[cfg(not(target_os = "macos"))]
use elsewhere::{apply_player, apply_sound, players, sound};

/// Follows the players and the volume while [wanted] (a phone that takes them is linked) and
/// answers [requests] (device id, packet). [out]: (to whom, or every device that takes it; packet).
pub async fn run(wanted: Arc<AtomicBool>, mut requests: mpsc::UnboundedReceiver<(String, Value)>, out: mpsc::UnboundedSender<(Option<String>, Value)>) {
    let mut known: Vec<(Player, Track)> = Vec::new();
    let mut volume: Option<Sound> = None;
    let mut tick = tokio::time::interval(EVERY);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        let asked = tokio::select! {
            _ = tick.tick() => {
                if !wanted.load(Ordering::Relaxed) {
                    continue;
                }
                None
            }
            req = requests.recv() => match req {
                Some(r) => Some(r),
                None => return,
            },
        };
        // Changes first, so what's read next shows them.
        if let Some((_, p)) = &asked {
            let body = &p["body"];
            match p["type"].as_str() {
                Some(MPRIS_REQUEST) => {
                    if let Some(player) = body["player"].as_str().and_then(Player::named) {
                        apply_player(player, body).await;
                    }
                }
                Some(VOLUME_REQUEST) => apply_sound(body).await,
                _ => {}
            }
        }
        let (now, sound_now) = (players().await, sound().await);
        for p in tell(&known, &now) {
            let _ = out.send((None, p));
        }
        if sound_now != volume {
            let p = if sound_now.is_some() != volume.is_some() { sinks_packet(sound_now) } else { sound_now.map(sound_packet).unwrap_or_else(|| sinks_packet(None)) };
            let _ = out.send((None, p));
        }
        (known, volume) = (now, sound_now);
        // Then what was asked for.
        let Some((id, p)) = asked else { continue };
        let body = &p["body"];
        if p["type"] == MPRIS_REQUEST {
            if body["requestPlayerList"].as_bool() == Some(true) {
                let _ = out.send((Some(id.clone()), list_packet(&known)));
            }
            if (body["requestNowPlaying"].as_bool() == Some(true) || body["requestVolume"].as_bool() == Some(true))
                && let Some((p, t)) = known.iter().find(|(p, _)| Some(p.name()) == body["player"].as_str())
            {
                let _ = out.send((Some(id.clone()), track_packet(p.name(), t)));
            }
        } else if p["type"] == VOLUME_REQUEST && body["requestSinks"].as_bool() == Some(true) {
            let _ = out.send((Some(id), sinks_packet(volume)));
        }
    }
}

/// What the phones are told after [old] became [new]: the list if players came or went (and each
/// one's state), else the state of each that changed.
fn tell(old: &[(Player, Track)], new: &[(Player, Track)]) -> Vec<Value> {
    let names = |l: &[(Player, Track)]| l.iter().map(|(p, _)| *p).collect::<Vec<_>>();
    if names(old) != names(new) {
        return std::iter::once(list_packet(new)).chain(new.iter().map(|(p, t)| track_packet(p.name(), t))).collect();
    }
    old.iter().zip(new).filter(|((_, a), (_, b))| changed(a, b)).map(|(_, (p, t))| track_packet(p.name(), t)).collect()
}

#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use std::process::Stdio;
    use tokio::process::Command;

    async fn osascript(script: &str, arg: Option<&str>) -> Option<String> {
        let mut c = Command::new("osascript");
        c.arg("-e").arg(script);
        if let Some(a) = arg {
            c.arg(a);
        }
        let out = tokio::time::timeout(Duration::from_secs(5), c.stdin(Stdio::null()).stderr(Stdio::null()).kill_on_drop(true).output()).await.ok()?.ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Whether [app] is open, without starting it (a script naming it would).
    async fn open(app: &str) -> bool {
        Command::new("pgrep").args(["-xq", app]).stdin(Stdio::null()).status().await.is_ok_and(|s| s.success())
    }

    /// Every player that's open, and what it's doing.
    pub async fn players() -> Vec<(Player, Track)> {
        let mut out = Vec::new();
        for p in Player::ALL {
            if open(p.name()).await
                && let Some(t) = osascript(&p.read_script(), None).await.as_deref().and_then(parse)
            {
                out.push((p, t));
            }
        }
        out
    }

    pub async fn sound() -> Option<Sound> {
        osascript(SOUND_READ, None).await.as_deref().and_then(parse_sound)
    }

    pub async fn apply_player(p: Player, body: &Value) {
        for (script, arg) in player_scripts(p, body) {
            if osascript(&script, arg.as_deref()).await.is_none() {
                tracing::info!("{} didn't take that", p.name());
            }
        }
    }

    pub async fn apply_sound(body: &Value) {
        for (script, arg) in sound_scripts(body) {
            if osascript(script, arg.as_deref()).await.is_none() {
                tracing::info!("couldn't change the volume");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_the_scripts_print() {
        assert_eq!(parse(""), None, "not open");
        assert_eq!(parse("stopped\n"), Some(Track::default()));
        let t = parse("playing\u{1f}Song\u{1f}Band\u{1f}Album\u{1f}215000\u{1f}61234\u{1f}80\u{1f}https://i.scdn.co/image/x\n").unwrap();
        assert_eq!((t.playing, t.title.as_str(), t.length_ms, t.pos_ms, t.volume), (true, "Song", 215000, 61234, 80));
        assert_eq!(t.art, "https://i.scdn.co/image/x");
        let t = parse("paused\u{1f}Song\u{1f}missing value\u{1f}\u{1f}1000\u{1f}0\u{1f}50\u{1f}file:///x").unwrap();
        assert_eq!((t.playing, t.artist.as_str(), t.art.as_str()), (false, "", ""), "no artist, no local art");
        assert_eq!(parse_sound("45,false\n"), Some(Sound { percent: 45, muted: false }));
        assert_eq!(parse_sound("missing value,false"), None, "an output with no volume");
    }

    #[test]
    fn only_fixed_scripts_run() {
        let s = player_scripts(Player::Music, &json!({ "action": "PlayPause" }));
        assert_eq!(s, [("tell application \"Music\" to playpause".to_owned(), None)]);
        assert!(player_scripts(Player::Music, &json!({ "action": "do shell script \"x\"" })).is_empty());
        assert_eq!(player_scripts(Player::Spotify, &json!({ "action": "Stop" }))[0].0, "tell application \"Spotify\" to pause", "Spotify has no stop");
        let s = player_scripts(Player::Music, &json!({ "setVolume": 300, "SetPosition": 61000 }));
        assert_eq!((s[0].1.as_deref(), s[1].1.as_deref()), (Some("100"), Some("61000")), "numbers go as arguments");
        assert_eq!(Player::named("Spotify"), Some(Player::Spotify));
        assert_eq!(Player::named("Terminal"), None);
    }

    #[test]
    fn the_volume_is_the_linux_apps_scale() {
        let s = Sound { percent: 50, muted: false };
        assert_eq!(sinks_packet(Some(s))["body"]["sinkList"][0]["volume"], NORMAL / 2);
        assert_eq!(sinks_packet(None)["body"]["sinkList"], json!([]));
        let scripts = sound_scripts(&json!({ "name": SINK, "volume": NORMAL / 4, "muted": true }));
        assert_eq!(scripts[0].1.as_deref(), Some("25"));
        assert_eq!(scripts[1].0, "set volume with output muted");
        assert!(sound_scripts(&json!({ "name": "other", "volume": 1 })).is_empty());
    }

    #[test]
    fn players_coming_and_going_resend_the_list() {
        let a = Track { title: "A".into(), ..Default::default() };
        let one = [(Player::Music, a.clone())];
        let two = [(Player::Music, a.clone()), (Player::Spotify, Track::default())];
        let sent = tell(&one, &two);
        assert_eq!(sent[0]["body"]["playerList"], json!(["Music", "Spotify"]));
        assert_eq!(sent.len(), 3, "the list, then each player");
        assert!(tell(&one, &one).is_empty());
        assert_eq!(tell(&one, &[(Player::Music, Track { playing: true, ..a })])[0]["body"]["isPlaying"], true);
    }

    #[test]
    fn the_phone_hears_changes_not_ticks() {
        let a = Track { playing: true, title: "A".into(), length_ms: 200_000, pos_ms: 10_000, ..Default::default() };
        assert!(!changed(&a, &Track { pos_ms: 13_000, ..a.clone() }), "playing on");
        assert!(changed(&a, &Track { pos_ms: 90_000, ..a.clone() }), "a seek");
        assert!(changed(&a, &Track { playing: false, ..a.clone() }));
        assert!(changed(&a, &Track { title: "B".into(), ..a.clone() }));
    }
}
