//! The PC's players and sound on the phone (`kdeconnect.mpris`, `kdeconnect.systemvolume`), read
//! and written once for every PC side: a player's state as the phone is told it, what a phone's
//! request asks of a player (in order, only the words the protocol has), and the outputs' list
//! and changes. Which players there are and how they're driven is each side's (MPRIS on Linux,
//! Windows' media controls, AppleScript for Music and Spotify on the Mac).

use serde_json::{Value, json};
use vyoma_core::link::packet;

pub const MPRIS: &str = "kdeconnect.mpris";
pub const MPRIS_REQUEST: &str = "kdeconnect.mpris.request";
pub const VOLUME: &str = "kdeconnect.systemvolume";
pub const VOLUME_REQUEST: &str = "kdeconnect.systemvolume.request";

/// 100%: the top of the phone's volume slider, PulseAudio's scale as kdeconnectd sends it.
pub const NORMAL: i64 = 65536;
/// A position further than this from where playing on would put it is a seek, worth telling.
const SEEK_MS: i64 = 2500;

/// A player's state, as the phone is told it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
    pub playing: bool,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// The cover: a web link the phone fetches, or a `file://` name it asks for as a payload.
    pub art: String,
    pub url: String,
    pub length_ms: i64,
    pub pos_ms: i64,
    /// The player's volume, 0-100.
    pub volume: i64,
    pub can_play: bool,
    pub can_pause: bool,
    pub can_next: bool,
    pub can_previous: bool,
    /// Seeking also needs a length: the phone has no bar to seek on without one.
    pub can_seek: bool,
    /// Only when the player says.
    pub shuffle: Option<bool>,
    pub repeat: Option<Repeat>,
}

/// MPRIS' LoopStatus.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Repeat {
    None,
    Track,
    Playlist,
}

impl Repeat {
    pub fn word(self) -> &'static str {
        match self {
            Repeat::None => "None",
            Repeat::Track => "Track",
            Repeat::Playlist => "Playlist",
        }
    }

    pub fn from_word(w: &str) -> Option<Repeat> {
        [Repeat::None, Repeat::Track, Repeat::Playlist].into_iter().find(|r| r.word() == w)
    }
}

pub fn track_packet(player: &str, t: &Track) -> Value {
    let mut body = json!({
        "player": player,
        "isPlaying": t.playing,
        "pos": t.pos_ms,
        "length": t.length_ms,
        "canPlay": t.can_play,
        "canPause": t.can_pause,
        "canGoNext": t.can_next,
        "canGoPrevious": t.can_previous,
        "canSeek": t.can_seek && t.length_ms > 0,
        "title": t.title,
        "artist": t.artist,
        "album": t.album,
        "albumArtUrl": t.art,
        "url": t.url,
        "volume": t.volume,
        // For old phones that only read this.
        "nowPlaying": if t.artist.is_empty() { t.title.clone() } else { format!("{} - {}", t.artist, t.title) },
    });
    if let Some(s) = t.shuffle {
        body["shuffle"] = json!(s);
    }
    if let Some(r) = t.repeat {
        body["loopStatus"] = json!(r.word());
    }
    packet(MPRIS, body)
}

/// The players' names; [art] when covers can come as payloads (a `file://` art name).
pub fn list_packet<S: AsRef<str>>(names: &[S], art: bool) -> Value {
    let names: Vec<&str> = names.iter().map(AsRef::as_ref).collect();
    packet(MPRIS, json!({ "playerList": names, "supportAlbumArtPayload": art }))
}

/// Worth telling the phone after [elapsed_ms]: anything but the position moving on as it plays,
/// which the phone counts itself; a seek is told.
pub fn changed(old: &Track, new: &Track, elapsed_ms: i64) -> bool {
    let expected = old.pos_ms + if old.playing { elapsed_ms } else { 0 };
    Track { pos_ms: 0, ..old.clone() } != Track { pos_ms: 0, ..new.clone() } || (new.pos_ms - expected).abs() > SEEK_MS
}

/// What a player request asks it to do, in order.
#[derive(Clone, Debug, PartialEq)]
pub enum Do {
    PlayPause,
    Play,
    Pause,
    Stop,
    Next,
    Previous,
    /// Milliseconds from the start.
    SetPosition(i64),
    /// Microseconds from here, as MPRIS' Seek.
    Seek(i64),
    Shuffle(bool),
    Repeat(Repeat),
    /// 0-100.
    Volume(i64),
}

/// A `kdeconnect.mpris.request`'s body, read.
#[derive(Debug, Default, PartialEq)]
pub struct Request {
    /// The players' list is asked for.
    pub list: bool,
    pub player: Option<String>,
    pub does: Vec<Do>,
    /// The player's state is asked for (after what it does).
    pub now: bool,
    /// A cover asked for as a payload, by the name the player was told.
    pub art: Option<String>,
}

pub fn request(body: &Value) -> Request {
    let mut does = Vec::new();
    match body["action"].as_str() {
        Some("PlayPause") => does.push(Do::PlayPause),
        Some("Play") => does.push(Do::Play),
        Some("Pause") => does.push(Do::Pause),
        Some("Stop") => does.push(Do::Stop),
        Some("Next") => does.push(Do::Next),
        Some("Previous") => does.push(Do::Previous),
        _ => {}
    }
    if let Some(v) = body["setVolume"].as_i64() {
        does.push(Do::Volume(v.clamp(0, 100)));
    }
    if let Some(us) = body["Seek"].as_i64() {
        does.push(Do::Seek(us));
    }
    if let Some(ms) = body["SetPosition"].as_i64() {
        does.push(Do::SetPosition(ms.max(0)));
    }
    if let Some(r) = body["setLoopStatus"].as_str().and_then(Repeat::from_word) {
        does.push(Do::Repeat(r));
    }
    if let Some(s) = body["setShuffle"].as_bool() {
        does.push(Do::Shuffle(s));
    }
    let yes = |k: &str| body[k].as_bool() == Some(true);
    Request {
        list: yes("requestPlayerList"),
        player: body["player"].as_str().map(str::to_owned),
        does,
        now: yes("requestNowPlaying") || yes("requestVolume"),
        art: body["albumArtUrl"].as_str().filter(|u| !u.is_empty()).map(str::to_owned),
    }
}

/// The packet saying [art] is coming as this packet's payload.
pub fn art_packet(player: &str, art: &str) -> Value {
    packet(MPRIS, json!({ "player": player, "transferringAlbumArt": true, "albumArtUrl": art }))
}

/// One of the PC's outputs on the phone's volume page.
#[derive(Clone, Debug, PartialEq)]
pub struct Sink {
    pub name: String,
    pub description: String,
    /// On [NORMAL]'s scale.
    pub volume: i64,
    pub muted: bool,
    /// The output in use (the default).
    pub enabled: bool,
}

impl Sink {
    /// A sink from a percentage, for PCs that have their own scale.
    pub fn from_percent(name: &str, description: &str, percent: i64, muted: bool) -> Sink {
        Sink { name: name.to_owned(), description: description.to_owned(), volume: percent.clamp(0, 100) * NORMAL / 100, muted, enabled: true }
    }

    /// Its change alone, once the phone has the list.
    pub fn packet(&self) -> Value {
        packet(VOLUME, json!({ "name": self.name, "volume": self.volume, "muted": self.muted, "enabled": self.enabled }))
    }
}

pub fn sinks_packet(sinks: &[Sink]) -> Value {
    let list: Vec<Value> = sinks.iter().map(|s| json!({ "name": s.name, "description": s.description, "volume": s.volume, "maxVolume": NORMAL, "muted": s.muted, "enabled": s.enabled })).collect();
    packet(VOLUME, json!({ "sinkList": list }))
}

/// What the phones are told after [old] became [new]: the whole list if outputs came or went, else
/// one packet per output that changed.
pub fn sink_changes(old: &[Sink], new: &[Sink]) -> Vec<Value> {
    let names = |s: &[Sink]| s.iter().map(|s| s.name.clone()).collect::<Vec<_>>();
    if names(old) != names(new) {
        return vec![sinks_packet(new)];
    }
    old.iter().zip(new).filter(|(a, b)| a != b).map(|(_, s)| s.packet()).collect()
}

/// A `kdeconnect.systemvolume.request`'s body, read.
#[derive(Debug, Default, PartialEq)]
pub struct SinkRequest {
    /// The list is asked for.
    pub list: bool,
    /// The output to change, and how.
    pub name: Option<String>,
    /// On [NORMAL]'s scale, not yet limited: each PC has its own most.
    pub volume: Option<i64>,
    pub muted: Option<bool>,
    /// Made the output in use.
    pub enable: bool,
}

impl SinkRequest {
    /// The volume asked for as a percentage, 0-100.
    pub fn percent(&self) -> Option<i64> {
        self.volume.map(|v| v.clamp(0, NORMAL) * 100 / NORMAL)
    }
}

pub fn sink_request(body: &Value) -> SinkRequest {
    SinkRequest {
        list: body["requestSinks"].as_bool() == Some(true),
        name: body["name"].as_str().map(str::to_owned),
        volume: body["volume"].as_i64(),
        muted: body["muted"].as_bool(),
        enable: body["enabled"].as_bool() == Some(true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_as_kdeconnectd_sends_them() {
        let t = Track { title: "Song".into(), artist: "Band".into(), shuffle: Some(true), repeat: Some(Repeat::Track), can_seek: true, length_ms: 1000, ..Default::default() };
        let b = &track_packet("Spotify", &t)["body"];
        assert_eq!((b["nowPlaying"].as_str(), b["shuffle"].as_bool(), b["loopStatus"].as_str(), b["canSeek"].as_bool()), (Some("Band - Song"), Some(true), Some("Track"), Some(true)));
        let b = &track_packet("x", &Track { can_seek: true, ..Default::default() })["body"];
        assert!(b.get("shuffle").is_none() && b.get("loopStatus").is_none(), "only what the player says");
        assert_eq!(b["canSeek"], false, "nothing to seek along");
        assert_eq!(list_packet(&["A", "B"], false)["body"], json!({ "playerList": ["A", "B"], "supportAlbumArtPayload": false }));
    }

    #[test]
    fn the_phone_hears_changes_not_ticks() {
        let a = Track { playing: true, title: "A".into(), length_ms: 200_000, pos_ms: 10_000, ..Default::default() };
        assert!(!changed(&a, &Track { pos_ms: 13_000, ..a.clone() }, 3000), "playing on");
        assert!(changed(&a, &Track { pos_ms: 90_000, ..a.clone() }, 3000), "a seek");
        assert!(changed(&a, &Track { playing: false, ..a.clone() }, 3000));
        assert!(!changed(&Track { playing: false, ..a.clone() }, &Track { playing: false, ..a.clone() }, 3000), "paused stays put");
    }

    #[test]
    fn requests_in_order() {
        let r = request(&json!({ "player": "P", "action": "PlayPause", "setVolume": 300, "SetPosition": -5, "setLoopStatus": "Playlist", "setShuffle": true, "requestNowPlaying": true }));
        assert_eq!(r.does, [Do::PlayPause, Do::Volume(100), Do::SetPosition(0), Do::Repeat(Repeat::Playlist), Do::Shuffle(true)]);
        assert_eq!((r.player.as_deref(), r.now, r.list, r.art), (Some("P"), true, false, None));
        assert!(request(&json!({ "action": "rm -rf", "setLoopStatus": "Forever" })).does.is_empty(), "only the protocol's words");
        let r = request(&json!({ "requestPlayerList": true, "albumArtUrl": "file:///a.png" }));
        assert_eq!((r.list, r.art.as_deref(), r.player), (true, Some("file:///a.png"), None));
        assert_eq!(art_packet("P", "u")["body"]["transferringAlbumArt"], true);
    }

    #[test]
    fn outputs_and_their_changes() {
        let a = Sink::from_percent("default", "Speakers", 50, false);
        let b = Sink { name: "hdmi".into(), description: "HDMI".into(), volume: 100, muted: true, enabled: false };
        let list = sinks_packet(&[a.clone(), b.clone()]);
        assert_eq!((list["body"]["sinkList"][0]["volume"].as_i64(), list["body"]["sinkList"][1]["maxVolume"].as_i64()), (Some(NORMAL / 2), Some(NORMAL)));
        let louder = Sink { volume: NORMAL, ..a.clone() };
        let c = sink_changes(&[a.clone(), b.clone()], &[louder, b.clone()]);
        assert_eq!((c.len(), c[0]["body"]["name"].as_str(), c[0]["body"]["volume"].as_i64()), (1, Some("default"), Some(NORMAL)));
        assert!(sink_changes(std::slice::from_ref(&a), std::slice::from_ref(&a)).is_empty());
        assert!(sink_changes(&[a.clone(), b], &[a])[0]["body"]["sinkList"].is_array(), "one went: the whole list");
        let r = sink_request(&json!({ "name": "default", "volume": NORMAL / 4, "muted": true }));
        assert_eq!((r.name.as_deref(), r.percent(), r.muted, r.enable, r.list), (Some("default"), Some(25), Some(true), false, false));
        assert!(sink_request(&json!({ "requestSinks": true })).list);
    }
}
