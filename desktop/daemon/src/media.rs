//! The PC's media players (MPRIS) for the phone to show and control, as kdeconnectd's mpriscontrol
//! plugin does: `kdeconnect.mpris.request` asks for the player list, a player's state, an action,
//! volume, a seek or a local file's album art; `kdeconnect.mpris` answers, and goes out unasked
//! whenever a player comes, goes or changes.
//!
//! Players are told apart by their `Identity` ("Firefox", "mpv"), with " [2]" on a second one of the
//! same name, like kdeconnectd.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use vyoma_core::link::packet;
use zbus::{Connection, MatchRule, MessageStream, OwnedVariant, Variant};

pub const MPRIS: &str = "kdeconnect.mpris";
pub const MPRIS_REQUEST: &str = "kdeconnect.mpris.request";

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const ROOT: &str = "org.mpris.MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
/// A frozen player never answers, and zbus would otherwise wait for ever.
const PLAYER_TIMEOUT: Duration = Duration::from_secs(3);
/// The most the phone takes for a cover.
const MAX_ART: u64 = 8 << 20;
const ACTIONS: &[&str] = &["PlayPause", "Play", "Pause", "Stop", "Next", "Previous"];

/// What the media side has for the devices.
pub enum Out {
    /// For every device that takes `kdeconnect.mpris`.
    All(Value),
    To(String, Value),
    /// A cover to offer the device as a payload with this packet.
    Art { id: String, packet: Value, path: PathBuf, size: u64 },
}

/// One player's state, as the phone is told it.
#[derive(Clone, Debug, Default, PartialEq)]
struct State {
    playing: bool,
    title: String,
    artist: String,
    album: String,
    art: String,
    url: String,
    track: String,
    length_ms: i64,
    pos_ms: i64,
    volume: i64,
    can_play: bool,
    can_pause: bool,
    can_next: bool,
    can_previous: bool,
    can_seek: bool,
    loop_status: Option<String>,
    shuffle: Option<bool>,
}

impl State {
    fn from_props(p: &HashMap<String, OwnedVariant>) -> Self {
        let meta: HashMap<String, OwnedVariant> = p.get("Metadata").and_then(|m| m.try_clone().ok()).and_then(|m| m.try_into().ok()).unwrap_or_default();
        let flag = |k: &str| p.get(k).and_then(|v| bool::try_from(&**v).ok()).unwrap_or(false);
        Self {
            playing: p.get("PlaybackStatus").and_then(text).as_deref() == Some("Playing"),
            title: meta.get("xesam:title").and_then(text).unwrap_or_default(),
            artist: meta.get("xesam:artist").map(strings).unwrap_or_default().join(", "),
            album: meta.get("xesam:album").and_then(text).unwrap_or_default(),
            art: meta.get("mpris:artUrl").and_then(text).unwrap_or_default(),
            url: meta.get("xesam:url").and_then(text).unwrap_or_default(),
            track: meta.get("mpris:trackid").and_then(path).unwrap_or_default(),
            length_ms: meta.get("mpris:length").and_then(num).map_or(0, |us| us as i64 / 1000),
            pos_ms: p.get("Position").and_then(num).map_or(0, |us| us as i64 / 1000),
            volume: p.get("Volume").and_then(num).map_or(100, |v| (v * 100.0).round().clamp(0.0, 100.0) as i64),
            can_play: flag("CanPlay"),
            can_pause: flag("CanPause"),
            can_next: flag("CanGoNext"),
            can_previous: flag("CanGoPrevious"),
            can_seek: flag("CanSeek"),
            loop_status: p.get("LoopStatus").and_then(text),
            shuffle: p.get("Shuffle").and_then(|v| bool::try_from(&**v).ok()),
        }
    }

    fn packet(&self, player: &str) -> Value {
        let mut body = json!({
            "player": player,
            "isPlaying": self.playing,
            "pos": self.pos_ms,
            "length": self.length_ms,
            "canPlay": self.can_play,
            "canPause": self.can_pause,
            "canGoNext": self.can_next,
            "canGoPrevious": self.can_previous,
            "canSeek": self.can_seek,
            "title": self.title,
            "artist": self.artist,
            "album": self.album,
            "albumArtUrl": self.art,
            "url": self.url,
            "volume": self.volume,
            // For old phones that only read this.
            "nowPlaying": if self.artist.is_empty() { self.title.clone() } else { format!("{} - {}", self.artist, self.title) },
        });
        if let Some(l) = &self.loop_status {
            body["loopStatus"] = json!(l);
        }
        if let Some(s) = self.shuffle {
            body["shuffle"] = json!(s);
        }
        packet(MPRIS, body)
    }
}

struct Player {
    /// Its well-known name (org.mpris.MediaPlayer2.firefox.instance_1_42), for calls.
    bus: String,
    /// What the phone calls it.
    name: String,
    state: State,
}

struct Media {
    conn: Connection,
    /// Keyed by the player's unique bus name (":1.42"), which is what signals carry.
    players: HashMap<String, Player>,
    out: mpsc::UnboundedSender<Out>,
}

/// Follows the players and answers [requests] (device id, packet) until the session bus goes.
pub async fn run(mut requests: mpsc::UnboundedReceiver<(String, Value)>, out: mpsc::UnboundedSender<Out>) -> Result<()> {
    let conn = zbus::connection::Builder::session().method_timeout(PLAYER_TIMEOUT).build().await.context("no session bus")?;
    let rule = |iface: &'static str, member: &'static str| MatchRule::builder().msg_type(zbus::message::Type::Signal).interface(iface).member(member).build();
    let mut changed = MessageStream::for_match_rule(rule(PROPERTIES, "PropertiesChanged")?, &conn, None).await?;
    let mut seeked = MessageStream::for_match_rule(rule(PLAYER, "Seeked")?, &conn, None).await?;
    let mut owners = MessageStream::for_match_rule(rule("org.freedesktop.DBus", "NameOwnerChanged")?, &conn, None).await?;

    let only: Option<Vec<String>> = std::env::var("VYOMA_PLAYERS").ok().map(|s| s.split(',').map(|s| format!("{PREFIX}{}", s.trim())).collect());
    let shown = move |name: &str| name.starts_with(PREFIX) && only.as_ref().is_none_or(|o| o.iter().any(|x| x == name));
    let mut media = Media { conn, players: HashMap::new(), out };
    let names: Vec<String> = media.conn.call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "ListNames", &()).await?.body().deserialize()?;
    for name in names.into_iter().filter(|n| shown(n)) {
        media.add(&name).await;
    }
    loop {
        tokio::select! {
            Some((id, p)) = requests.recv() => media.on_request(&id, &p["body"]).await,
            Some(Ok(msg)) = changed.next() => {
                let Some(sender) = msg.header().sender().map(|s| s.to_string()) else { continue };
                let Ok((iface, ..)) = msg.body().deserialize::<(String, HashMap<String, OwnedVariant>, Vec<String>)>() else { continue };
                if iface == PLAYER {
                    media.refresh(&sender).await;
                }
            }
            Some(Ok(msg)) = seeked.next() => {
                if let Some(sender) = msg.header().sender().map(|s| s.to_string()) {
                    media.refresh(&sender).await;
                }
            }
            Some(Ok(msg)) = owners.next() => {
                let Ok((name, old, new)) = msg.body().deserialize::<(String, String, String)>() else { continue };
                if !shown(&name) {
                    continue;
                }
                if !old.is_empty() && media.players.remove(&old).is_some() {
                    media.send(Out::All(media.list()));
                }
                if !new.is_empty() {
                    media.add(&name).await;
                }
            }
            else => break,
        }
    }
    Ok(())
}

impl Media {
    fn send(&self, o: Out) {
        let _ = self.out.send(o);
    }

    fn list(&self) -> Value {
        let mut names: Vec<&str> = self.players.values().map(|p| p.name.as_str()).collect();
        names.sort();
        packet(MPRIS, json!({ "playerList": names, "supportAlbumArtPayload": true }))
    }

    fn by_name(&self, name: &str) -> Option<&Player> {
        self.players.values().find(|p| p.name == name)
    }

    async fn add(&mut self, bus: &str) {
        let Ok(owner) = self.call::<String>("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "GetNameOwner", &bus).await else { return };
        let Ok(props) = self.props(bus).await else { return };
        let identity = self.call::<OwnedVariant>(bus, PATH, PROPERTIES, "Get", &(ROOT, "Identity")).await.ok().and_then(|v| text(&v));
        let base = identity.filter(|i| !i.trim().is_empty()).unwrap_or_else(|| bus.trim_start_matches(PREFIX).split('.').next().unwrap_or("Player").to_owned());
        let taken: Vec<&str> = self.players.values().map(|p| p.name.as_str()).collect();
        let name = unique(&base, &taken);
        tracing::info!("media player {name}");
        self.players.insert(owner, Player { bus: bus.to_owned(), name, state: State::from_props(&props) });
        self.send(Out::All(self.list()));
    }

    /// Reads a player again after it signalled; the devices hear only of a real change.
    async fn refresh(&mut self, owner: &str) {
        let Some(bus) = self.players.get(owner).map(|p| p.bus.clone()) else { return };
        let Ok(props) = self.props(&bus).await else { return };
        let state = State::from_props(&props);
        let Some(p) = self.players.get_mut(owner) else { return };
        // The position moves on its own; alone it's no news (the phone keeps the clock).
        let mut was = p.state.clone();
        was.pos_ms = state.pos_ms;
        if was != state || p.state.playing != state.playing {
            p.state = state;
            let packet = p.state.packet(&p.name);
            self.send(Out::All(packet));
        } else {
            p.state = state;
        }
    }

    async fn on_request(&mut self, id: &str, body: &Value) {
        if body["requestPlayerList"].as_bool() == Some(true) {
            self.send(Out::To(id.to_owned(), self.list()));
        }
        let Some(name) = body["player"].as_str() else { return };
        let Some(player) = self.by_name(name) else { return };
        let (bus, state) = (player.bus.clone(), player.state.clone());
        let bus = bus.as_str();

        if let Some(action) = body["action"].as_str().filter(|a| ACTIONS.contains(a)) {
            self.player_call(bus, action, &()).await;
        }
        if let Some(v) = body["setVolume"].as_i64() {
            self.set(bus, "Volume", Variant::from(v.clamp(0, 100) as f64 / 100.0)).await;
        }
        if let Some(us) = body["Seek"].as_i64() {
            self.player_call(bus, "Seek", &us).await;
        }
        if let Some(ms) = body["SetPosition"].as_i64() {
            let us = ms.max(0) * 1000;
            match zbus::ObjectPath::try_from(state.track.as_str()) {
                Ok(track) if !state.track.is_empty() => self.player_call(bus, "SetPosition", &(track, us)).await,
                // No track id to name: a seek by the difference does the same.
                _ => self.player_call(bus, "Seek", &(us - state.pos_ms * 1000)).await,
            }
        }
        if let Some(l) = body["setLoopStatus"].as_str().filter(|l| ["None", "Track", "Playlist"].contains(l)) {
            self.set(bus, "LoopStatus", Variant::from(l)).await;
        }
        if let Some(s) = body["setShuffle"].as_bool() {
            self.set(bus, "Shuffle", Variant::from(s)).await;
        }
        if body["requestNowPlaying"].as_bool() == Some(true) || body["requestVolume"].as_bool() == Some(true) {
            // Read afresh: the position isn't signalled.
            let now = self.props(bus).await.map(|p| State::from_props(&p)).unwrap_or_else(|_| state.clone());
            self.send(Out::To(id.to_owned(), now.packet(name)));
        }
        if let Some(url) = body["albumArtUrl"].as_str() {
            // Only the cover the player itself shows: never any file a device names.
            if url == state.art
                && let Some(path) = file_path(url)
                && let Ok(meta) = std::fs::metadata(&path)
                && meta.is_file()
                && meta.len() <= MAX_ART
            {
                let packet = packet(MPRIS, json!({ "player": name, "transferringAlbumArt": true, "albumArtUrl": url }));
                self.send(Out::Art { id: id.to_owned(), packet, path, size: meta.len() });
            } else {
                tracing::info!("{id}: album art for {name} refused: not its cover, or not a local file under {} MiB", MAX_ART >> 20);
            }
        }
    }

    async fn call<T: serde::de::DeserializeOwned + zbus::zvariant::Type>(&self, dest: &str, path: &str, iface: &str, method: &str, args: &(impl serde::Serialize + zbus::zvariant::DynamicType)) -> Result<T> {
        Ok(self.conn.call_method(Some(dest), path, Some(iface), method, args).await?.body().deserialize()?)
    }

    async fn props(&self, bus: &str) -> Result<HashMap<String, OwnedVariant>> {
        self.call(bus, PATH, PROPERTIES, "GetAll", &PLAYER).await
    }

    async fn player_call(&self, bus: &str, method: &str, args: &(impl serde::Serialize + zbus::zvariant::DynamicType)) {
        if let Err(e) = self.conn.call_method(Some(bus), PATH, Some(PLAYER), method, args).await {
            tracing::info!("{bus} {method}: {e}");
        }
    }

    async fn set(&self, bus: &str, property: &str, value: Variant<'_>) {
        if let Err(e) = self.conn.call_method(Some(bus), PATH, Some(PROPERTIES), "Set", &(PLAYER, property, value)).await {
            tracing::info!("{bus} {property}: {e}");
        }
    }
}

/// [base], or `base [2]`, `base [3]`… whichever isn't [taken].
fn unique(base: &str, taken: &[&str]) -> String {
    let mut name = base.to_owned();
    let mut n = 2;
    while taken.contains(&name.as_str()) {
        name = format!("{base} [{n}]");
        n += 1;
    }
    name
}

/// A local `file://` URL as a path, percent-decoded.
fn file_path(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    let path = PathBuf::from(<std::ffi::OsString as std::os::unix::ffi::OsStringExt>::from_vec(out));
    path.is_absolute().then_some(path)
}

fn text(v: &OwnedVariant) -> Option<String> {
    <&str>::try_from(&**v).ok().map(str::to_owned)
}

fn path(v: &OwnedVariant) -> Option<String> {
    zbus::ObjectPath::try_from(&**v).ok().map(|p| p.to_string()).or_else(|| text(v))
}

/// Players say lengths and positions in whichever integer type they like.
fn num(v: &OwnedVariant) -> Option<f64> {
    let v = &**v;
    i64::try_from(v)
        .map(|n| n as f64)
        .or_else(|_| u64::try_from(v).map(|n| n as f64))
        .or_else(|_| i32::try_from(v).map(f64::from))
        .or_else(|_| u32::try_from(v).map(f64::from))
        .or_else(|_| f64::try_from(v))
        .ok()
}

/// `xesam:artist` is a list, but some players give one string.
fn strings(v: &OwnedVariant) -> Vec<String> {
    v.try_clone().ok().and_then(|v| Vec::<String>::try_from(v).ok()).or_else(|| text(v).map(|s| vec![s])).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_paths() {
        assert_eq!(unique("mpv", &[]), "mpv");
        assert_eq!(unique("mpv", &["mpv", "mpv [2]"]), "mpv [3]");
        assert_eq!(file_path("file:///home/a/My%20Cover.jpg"), Some(PathBuf::from("/home/a/My Cover.jpg")));
        assert_eq!(file_path("file:///x/%C3%BC"), Some(PathBuf::from("/x/ü")));
        assert_eq!(file_path("https://x/a.jpg"), None);
        assert_eq!(file_path("file://relative"), None);
        assert_eq!(file_path("file:///bad%2"), Some(PathBuf::from("/bad%2")));
    }
}

/// Drives a fake player on the session bus through [run]. Run by hand:
/// `cargo test --release -- --ignored fake_player` (a "Vyoma test" player appears briefly; the
/// PC's own players are only listed, never touched).
#[cfg(test)]
mod live {
    use std::collections::HashMap;
    use std::time::Duration;

    use serde_json::{Value, json};
    use tokio::sync::mpsc;
    use vyoma_core::link::packet;
    use zbus::{OwnedVariant, Variant};

    use super::{MPRIS, MPRIS_REQUEST, Out, PATH};

    struct Root;

    #[zbus::interface(name = "org.mpris.MediaPlayer2")]
    impl Root {
        #[zbus(property)]
        fn identity(&self) -> String {
            "Vyoma test".into()
        }
    }

    struct Fake {
        status: String,
        volume: f64,
        calls: Vec<String>,
    }

    #[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
    impl Fake {
        #[zbus(property)]
        fn playback_status(&self) -> String {
            self.status.clone()
        }
        #[zbus(property)]
        fn metadata(&self) -> HashMap<String, OwnedVariant> {
            let mut m = HashMap::new();
            m.insert("xesam:title".into(), Variant::from("Song").try_into().unwrap());
            m.insert("xesam:artist".into(), Variant::from(vec!["A", "B"]).try_into().unwrap());
            m.insert("mpris:length".into(), Variant::from(180_000_000i64).try_into().unwrap());
            m.insert("mpris:trackid".into(), Variant::from(zbus::ObjectPath::try_from("/track/1").unwrap()).try_into().unwrap());
            m
        }
        #[zbus(property)]
        fn position(&self) -> i64 {
            42_000_000
        }
        #[zbus(property)]
        fn volume(&self) -> f64 {
            self.volume
        }
        #[zbus(property)]
        fn set_volume(&mut self, v: f64) {
            self.volume = v;
        }
        #[zbus(property)]
        fn can_go_next(&self) -> bool {
            true
        }
        fn play_pause(&mut self) {
            self.status = if self.status == "Playing" { "Paused".into() } else { "Playing".into() };
            self.calls.push("PlayPause".into());
        }
        fn set_position(&mut self, track: zbus::ObjectPath<'_>, us: i64) {
            self.calls.push(format!("SetPosition {track} {us}"));
        }
    }

    async fn next(out: &mut mpsc::UnboundedReceiver<Out>, wanted: impl Fn(&Value) -> bool) -> Value {
        let wait = async {
            loop {
                match out.recv().await.unwrap() {
                    Out::All(p) | Out::To(_, p) if wanted(&p["body"]) => return p,
                    _ => {}
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(5), wait).await.expect("no such packet")
    }

    #[tokio::test]
    #[ignore]
    async fn fake_player() {
        let conn = zbus::connection::Builder::session()
            .name("org.mpris.MediaPlayer2.vyomatest")
            .serve_at(PATH, Root)
            .serve_at(PATH, Fake { status: "Paused".into(), volume: 0.5, calls: Vec::new() })
            .build()
            .await
            .unwrap();
        let (requests, rx) = mpsc::unbounded_channel();
        let (tx, mut out) = mpsc::unbounded_channel();
        tokio::spawn(super::run(rx, tx));
        let ask = |body: Value| requests.send(("phone".into(), packet(MPRIS_REQUEST, body))).unwrap();

        ask(json!({ "requestPlayerList": true }));
        let list = next(&mut out, |b| b["playerList"].as_array().is_some_and(|l| l.contains(&json!("Vyoma test")))).await;
        assert_eq!(list["type"], MPRIS);

        ask(json!({ "player": "Vyoma test", "requestNowPlaying": true }));
        let state = next(&mut out, |b| b["player"] == "Vyoma test").await["body"].clone();
        assert_eq!((state["title"].as_str(), state["artist"].as_str()), (Some("Song"), Some("A, B")));
        assert_eq!((state["pos"].as_i64(), state["length"].as_i64(), state["volume"].as_i64()), (Some(42_000), Some(180_000), Some(50)));
        assert_eq!((state["isPlaying"].as_bool(), state["canGoNext"].as_bool()), (Some(false), Some(true)));

        ask(json!({ "player": "Vyoma test", "action": "PlayPause" }));
        let iface = conn.object_server().interface::<_, Fake>(PATH).await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        iface.get().await.playback_status_changed(iface.signal_emitter()).await.unwrap();
        next(&mut out, |b| b["player"] == "Vyoma test" && b["isPlaying"] == true).await;

        ask(json!({ "player": "Vyoma test", "setVolume": 80, "SetPosition": 10_000 }));
        ask(json!({ "player": "Vyoma test", "action": "rm -rf" }));
        tokio::time::sleep(Duration::from_millis(500)).await;
        let fake = iface.get().await;
        assert_eq!(fake.volume, 0.8);
        assert_eq!(fake.calls, ["PlayPause", "SetPosition /track/1 10000000"]);
        println!("ok: list, state, play/pause, volume and seek");
    }
}
