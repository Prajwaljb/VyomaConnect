//! Handoff: what's playing on the PC (a YouTube video in the browser), and where in it, so the
//! phone can carry on from the same moment.
//!
//! Firefox's MPRIS gives the page URL but always reports position 0. So we keep the clock
//! ourselves from the player's signals: a new track starts at 0 (or the URL's `t=`), time counts
//! while it's Playing, and every Seeked resets it. Players that do report Position (Chromium,
//! mpv) are asked directly.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{Context, Result};
use futures_util::StreamExt;
use tracing::warn;
use zbus::{Connection, MatchRule, MessageStream, OwnedVariant};

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

/// What the phone needs to continue: the page, and where we are in it.
pub struct Handoff {
    pub url: String,
    pub title: String,
    /// None when we can't know: we started after the video did and the player doesn't say.
    pub position_ms: Option<i64>,
    /// False when the position is our estimate rather than the player's own.
    pub exact: bool,
    bus: String,
}

#[derive(Clone)]
struct Track {
    /// Well-known name (org.mpris.MediaPlayer2.firefox...), for calls.
    bus: String,
    url: String,
    title: String,
    base_ms: i64,
    since: Instant,
    playing: bool,
    touched: Instant,
    /// We've seen where the clock started (the track begin, a seek, or a reported position).
    known: bool,
}

impl Track {
    fn position_ms(&self) -> i64 {
        self.base_ms + if self.playing { self.since.elapsed().as_millis() as i64 } else { 0 }
    }
}

pub struct Media {
    conn: Connection,
    /// Keyed by the player's unique bus name (":1.42"), which is what signals carry.
    tracks: Mutex<HashMap<String, Track>>,
}

impl Media {
    pub async fn start() -> Result<Arc<Media>> {
        let conn = Connection::session().await.context("no session bus")?;
        let media = Arc::new(Media { conn, tracks: Mutex::default() });
        let names: Vec<String> = media.conn.call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "ListNames", &()).await?.body().deserialize()?;
        for name in names.into_iter().filter(|n| n.starts_with(PREFIX)) {
            media.add(&name).await;
        }
        let m = media.clone();
        tokio::spawn(async move {
            if let Err(e) = m.watch().await {
                warn!("media tracking stopped: {e:#}");
            }
        });
        Ok(media)
    }

    /// The video to continue: a playing one first, else the one used most recently.
    pub async fn handoff(&self) -> Option<Handoff> {
        self.handoff_where(|_| true).await
    }

    async fn handoff_where(&self, wanted: impl Fn(&Track) -> bool) -> Option<Handoff> {
        let track = self.best(wanted)?;
        // Exact when the player reports it (anything but 0, which Firefox always says).
        let reported = self.position(&track.bus).await.filter(|&p| p > 0);
        Some(Handoff {
            position_ms: reported.or_else(|| track.known.then(|| track.position_ms())),
            exact: reported.is_some(),
            url: track.url,
            title: track.title,
            bus: track.bus,
        })
    }

    /// Title of what [handoff] would hand over right now, for the phone to offer it. Cheap: no D-Bus.
    pub fn offer(&self) -> Option<String> {
        self.best(|_| true).map(|t| t.title)
    }

    /// Only web pages can continue on the phone: mpv/VLC playing a local file (`file://`, or no
    /// URL at all) never qualifies, so the phone doesn't offer it.
    fn best(&self, wanted: impl Fn(&Track) -> bool) -> Option<Track> {
        let tracks = self.tracks.lock().unwrap();
        tracks.values().filter(|t| is_web(&t.url) && wanted(t)).max_by_key(|t| (t.playing, t.touched)).cloned()
    }

    /// Pauses the player we just handed off, so it doesn't keep going on the PC.
    pub async fn pause(&self, handoff: &Handoff) {
        let _ = self.conn.call_method(Some(handoff.bus.as_str()), PATH, Some(PLAYER), "Pause", &()).await;
    }

    /// Pauses every player that's playing (any player, not only web ones); returns their bus names to resume.
    pub async fn pause_all(&self) -> Vec<String> {
        let playing: Vec<String> = self.tracks.lock().unwrap().values().filter(|t| t.playing).map(|t| t.bus.clone()).collect();
        for bus in &playing {
            let _ = self.conn.call_method(Some(bus.as_str()), PATH, Some(PLAYER), "Pause", &()).await;
        }
        playing
    }

    /// Resumes a player [pause_all] paused, if it's still there.
    pub async fn play_bus(&self, bus: &str) {
        let _ = self.conn.call_method(Some(bus), PATH, Some(PLAYER), "Play", &()).await;
    }

    async fn add(&self, name: &str) {
        let Ok(owner) = self.owner(name).await else { return };
        let Ok(props) = self.props(name).await else { return };
        let (url, title) = props.get("Metadata").map(metadata).unwrap_or_default();
        let playing = props.get("PlaybackStatus").and_then(text).is_some_and(|s| s == "Playing");
        let reported = props.get("Position").and_then(int).map(|us| us / 1000).filter(|&p| p > 0);
        let now = Instant::now();
        let base_ms = reported.unwrap_or_else(|| url_time_ms(&url));
        // Nothing playing yet means the clock will start when it does; mid-play we can't know.
        let known = reported.is_some() || !playing;
        let track = Track { bus: name.to_string(), url, title, base_ms, since: now, playing, touched: now, known };
        self.tracks.lock().unwrap().insert(owner, track);
    }

    async fn watch(&self) -> Result<()> {
        let rule = |iface: &'static str, member: &'static str| -> Result<MatchRule<'static>> {
            Ok(MatchRule::builder().msg_type(zbus::message::Type::Signal).interface(iface).member(member).build()?)
        };
        let mut changed = MessageStream::for_match_rule(rule("org.freedesktop.DBus.Properties", "PropertiesChanged")?, &self.conn, None).await?;
        let mut seeked = MessageStream::for_match_rule(rule(PLAYER, "Seeked")?, &self.conn, None).await?;
        let mut owners = MessageStream::for_match_rule(rule("org.freedesktop.DBus", "NameOwnerChanged")?, &self.conn, None).await?;
        loop {
            tokio::select! {
                Some(Ok(msg)) = changed.next() => {
                    let Some(sender) = msg.header().sender().map(|s| s.to_string()) else { continue };
                    let Ok((iface, props, _)) = msg.body().deserialize::<(String, HashMap<String, OwnedVariant>, Vec<String>)>() else { continue };
                    if iface == PLAYER {
                        self.on_changed(&sender, &props);
                    }
                }
                Some(Ok(msg)) = seeked.next() => {
                    let Some(sender) = msg.header().sender().map(|s| s.to_string()) else { continue };
                    let Ok(us) = msg.body().deserialize::<i64>() else { continue };
                    if let Some(t) = self.tracks.lock().unwrap().get_mut(&sender) {
                        (t.base_ms, t.since, t.touched, t.known) = (us / 1000, Instant::now(), Instant::now(), true);
                    }
                }
                Some(Ok(msg)) = owners.next() => {
                    let Ok((name, old, new)) = msg.body().deserialize::<(String, String, String)>() else { continue };
                    if !name.starts_with(PREFIX) { continue }
                    if !old.is_empty() { self.tracks.lock().unwrap().remove(&old); }
                    if !new.is_empty() { self.add(&name).await; }
                }
                else => break,
            }
        }
        Ok(())
    }

    fn on_changed(&self, sender: &str, props: &HashMap<String, OwnedVariant>) {
        let mut tracks = self.tracks.lock().unwrap();
        let Some(t) = tracks.get_mut(sender) else { return };
        let now = Instant::now();
        t.touched = now;
        if let Some((url, title)) = props.get("Metadata").map(metadata) {
            if url != t.url {
                // A new video: it starts from the top, or where its link says.
                (t.base_ms, t.since, t.known) = (url_time_ms(&url), now, true);
            }
            (t.url, t.title) = (url, title);
        }
        if let Some(status) = props.get("PlaybackStatus").and_then(text) {
            let playing = status == "Playing";
            if playing != t.playing {
                (t.base_ms, t.since, t.playing) = (t.position_ms(), now, playing);
            }
        }
    }

    async fn owner(&self, name: &str) -> Result<String> {
        Ok(self.conn.call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "GetNameOwner", &name).await?.body().deserialize()?)
    }

    async fn props(&self, name: &str) -> Result<HashMap<String, OwnedVariant>> {
        Ok(self.conn.call_method(Some(name), PATH, Some("org.freedesktop.DBus.Properties"), "GetAll", &PLAYER).await?.body().deserialize()?)
    }

    async fn position(&self, name: &str) -> Option<i64> {
        let props = self.props(name).await.ok()?;
        props.get("Position").and_then(int).map(|us| us / 1000)
    }
}

fn is_web(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://")
}

/// `xesam:url` and `xesam:title` from an MPRIS Metadata value.
fn metadata(value: &OwnedVariant) -> (String, String) {
    let Ok(map) = HashMap::<String, OwnedVariant>::try_from(value.try_clone().unwrap_or_else(|_| OwnedVariant::from(0u8))) else {
        return Default::default();
    };
    (map.get("xesam:url").and_then(text).unwrap_or_default(), map.get("xesam:title").and_then(text).unwrap_or_default())
}

fn text(v: &OwnedVariant) -> Option<String> {
    <&str>::try_from(&**v).ok().map(str::to_owned)
}

fn int(v: &OwnedVariant) -> Option<i64> {
    i64::try_from(&**v).ok()
}

/// A YouTube-style `t=` in a URL (`t=90`, `t=90s`, `t=1m30s`), in ms; 0 if there's none.
fn url_time_ms(url: &str) -> i64 {
    let Some(query) = url.split_once('?').map(|(_, q)| q) else { return 0 };
    let Some(t) = query.split(['&', '#']).find_map(|kv| kv.strip_prefix("t=")) else { return 0 };
    let (mut total, mut num) = (0i64, 0i64);
    for c in t.chars() {
        match c {
            '0'..='9' => num = num * 10 + c.to_digit(10).unwrap() as i64,
            'h' => (total, num) = (total + num * 3600, 0),
            'm' => (total, num) = (total + num * 60, 0),
            's' => (total, num) = (total + num, 0),
            _ => break,
        }
    }
    (total + num) * 1000
}

#[cfg(test)]
mod tests {
    #[test]
    fn url_times() {
        assert_eq!(super::url_time_ms("https://www.youtube.com/watch?v=x&t=14s"), 14_000);
        assert_eq!(super::url_time_ms("https://youtu.be/x?t=90"), 90_000);
        assert_eq!(super::url_time_ms("https://www.youtube.com/watch?v=x&t=1h2m3s"), 3_723_000);
        assert_eq!(super::url_time_ms("https://www.youtube.com/watch?v=x"), 0);
    }
}

/// Drives a fake Firefox (URL, no position) on the session bus and checks the clock we keep.
/// Run by hand: `cargo test --release -- --ignored fake_firefox` (a "vyomatest" player appears briefly).
#[cfg(test)]
mod live {
    use std::collections::HashMap;
    use std::time::Duration;

    use zbus::object_server::SignalEmitter;
    use zbus::{OwnedVariant, Variant};

    struct Fake {
        status: String,
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
            m.insert("xesam:url".into(), Variant::from("https://www.youtube.com/watch?v=test&t=30s").try_into().unwrap());
            m.insert("xesam:title".into(), Variant::from("Vyoma test").try_into().unwrap());
            m
        }
        #[zbus(property)]
        fn position(&self) -> i64 {
            0 // like Firefox
        }
        fn pause(&mut self) {
            self.status = "Paused".into();
        }
        #[zbus(signal)]
        async fn seeked(emitter: &SignalEmitter<'_>, position: i64) -> zbus::Result<()>;
    }

    async fn set_status(conn: &zbus::Connection, status: &str) {
        let iface = conn.object_server().interface::<_, Fake>(super::PATH).await.unwrap();
        iface.get_mut().await.status = status.into();
        iface.get().await.playback_status_changed(iface.signal_emitter()).await.unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn fake_firefox() {
        let conn = zbus::connection::Builder::session()
            .name("org.mpris.MediaPlayer2.vyomatest")
            .serve_at(super::PATH, Fake { status: "Paused".into() })
            .build().await.unwrap();
        let media = super::Media::start().await.unwrap();
        let pos = || async { media.handoff_where(|t| t.bus.ends_with("vyomatest")).await.and_then(|h| h.position_ms) };

        // Seen while paused: the clock starts at the link's t=30s.
        assert_eq!(pos().await, Some(30_000));
        set_status(&conn, "Playing").await;
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let p = pos().await.unwrap();
        assert!((31_300..=31_800).contains(&p), "playing 1.5 s from 30 s gave {p}");

        let iface = conn.object_server().interface::<_, Fake>(super::PATH).await.unwrap();
        Fake::seeked(iface.signal_emitter(), 600_000_000).await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        let p = pos().await.unwrap();
        assert!((600_200..=600_500).contains(&p), "seek to 600 s gave {p}");

        set_status(&conn, "Paused").await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        let a = pos().await.unwrap();
        tokio::time::sleep(Duration::from_millis(700)).await;
        assert_eq!(pos().await.unwrap(), a, "paused clock moved");
        println!("ok: clock follows play, seek and pause");
    }
}
