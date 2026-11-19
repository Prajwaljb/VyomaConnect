//! What an OS supplies to the engine. Every method has a default that says this computer can't, so a
//! platform starts small and grows; [Platform::supports] decides what the phone is told is here, so
//! it never shows a tile this computer would refuse.

use std::future::Future;
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::{Result, bail};
use serde_json::Value;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use vyoma_core::identity::Identity;
use vyoma_core::link::Sender;

/// One thing the phone can ask this PC for, with the packet types that carry it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Feature {
    Stats,
    Specs,
    Screenshot,
    Camera,
    Handoff,
    /// The PC's files on the phone, and resumable sends from the phone.
    Files,
    Screen,
    /// Brightness and each app's volume.
    Controls,
    /// Sleep, restart, shut down, log out.
    Power,
    /// Wakes the screen (and unlocks it, where the platform can).
    Wake,
    Wallpaper,
    Gamepad,
    /// The phone's controller over a cable without USB debugging.
    Accessory,
    Desktop,
    Webcam,
    Mirror,
    Paste,
    /// The touchpad over Bluetooth.
    Touchpad,
    Away,
}

impl Feature {
    pub const ALL: [Feature; 19] = [
        Feature::Stats, Feature::Specs, Feature::Screenshot, Feature::Camera, Feature::Handoff, Feature::Files, Feature::Screen,
        Feature::Controls, Feature::Power, Feature::Wake, Feature::Wallpaper, Feature::Gamepad, Feature::Accessory, Feature::Desktop, Feature::Webcam,
        Feature::Mirror, Feature::Paste, Feature::Touchpad, Feature::Away,
    ];

    /// The packets the phone sends for it.
    pub fn incoming(self) -> &'static [&'static str] {
        match self {
            Feature::Stats => &["vyoma.stats.request"],
            Feature::Specs => &["vyoma.specs.request"],
            Feature::Screenshot => &["vyoma.screenshot.request"],
            Feature::Camera => &["vyoma.camera.request"],
            Feature::Handoff => &["vyoma.handoff.request"],
            Feature::Files => &["vyoma.files.list", "vyoma.files.get", "vyoma.files.put", "vyoma.xfer.begin", "vyoma.xfer.data", "vyoma.xfer.end"],
            Feature::Screen => &["vyoma.screen.request", "vyoma.screen.stop", "vyoma.screen.input", "vyoma.screen.touch"],
            Feature::Controls => &["vyoma.pc.request", "vyoma.pc.set"],
            Feature::Power => &["vyoma.pc.power"],
            Feature::Wake => &["vyoma.pc.wake"],
            Feature::Wallpaper => &["vyoma.wallpaper.request"],
            Feature::Gamepad => &["vyoma.gamepad.state", "vyoma.gamepad.close"],
            Feature::Accessory => &["vyoma.usb.accessory"],
            Feature::Desktop => &["vyoma.desktop.step", "vyoma.desktop.switch"],
            Feature::Webcam => &["vyoma.webcam", "vyoma.webcam.mic", "vyoma.webcam.turn"],
            Feature::Mirror => &["vyoma.mirror", "vyoma.mirror.audio", "vyoma.mirror.copy", "vyoma.mirror.volume"],
            Feature::Paste => &["vyoma.paste"],
            Feature::Touchpad => &["vyoma.pointer.request", "vyoma.pointer.close"],
            Feature::Away => &["vyoma.away"],
        }
    }

    /// The packets this PC sends for it.
    pub fn outgoing(self) -> &'static [&'static str] {
        match self {
            Feature::Stats => &["vyoma.stats"],
            Feature::Specs => &["vyoma.specs"],
            Feature::Screenshot => &["vyoma.screenshot"],
            Feature::Camera => &["vyoma.camera"],
            Feature::Handoff => &["vyoma.handoff"],
            Feature::Files => &["vyoma.files.listing", "vyoma.files.file", "vyoma.files.saved", "vyoma.xfer.ready", "vyoma.xfer.done"],
            Feature::Screen => &["vyoma.screen"],
            Feature::Controls => &["vyoma.pc"],
            Feature::Power => &["vyoma.pc.powered"],
            Feature::Wake => &[],
            Feature::Wallpaper => &["vyoma.wallpaper"],
            Feature::Gamepad => &["vyoma.gamepad.rumble", "vyoma.gamepad.udp"],
            Feature::Accessory => &[],
            Feature::Desktop => &["vyoma.desktop.stepped"],
            Feature::Webcam => &["vyoma.webcam.state"],
            Feature::Mirror => &["vyoma.mirror.input", "vyoma.mirror.state"],
            Feature::Paste => &["vyoma.paste.ready"],
            Feature::Touchpad => &["vyoma.pointer.ready", "vyoma.bluetooth"],
            Feature::Away => &[],
        }
    }

    /// The feature a phone's packet belongs to.
    pub fn of(kind: &str) -> Option<Feature> {
        Feature::ALL.into_iter().find(|f| f.incoming().contains(&kind))
    }
}

/// Who a session talks to, for what it starts: replies go out through [Cx::sender]; payload
/// connections show this PC's identity and expect the phone's pinned certificate.
#[derive(Clone)]
pub struct Cx {
    /// The phone's device id.
    pub id: String,
    pub me: Arc<Identity>,
    pub pinned: Vec<u8>,
    /// The phone's name as it's safe to show.
    pub name: String,
    pub sender: Sender,
}

/// A web video the phone could continue: the page and where it is.
pub struct Handoff {
    pub url: String,
    pub title: String,
    pub position_ms: Option<i64>,
    /// False when the position is an estimate.
    pub exact: bool,
    /// The platform's own way back to the player, to pause it.
    pub player: String,
}

/// Something running that is told to stop rather than cut off (it lets go of what it holds first).
pub struct Started {
    pub task: JoinHandle<()>,
    /// Asks it to finish; false if it couldn't hear (then it's aborted).
    pub close: Box<dyn Fn() -> bool + Send>,
}

impl Started {
    pub fn end(self) {
        if !(self.close)() {
            self.task.abort();
        }
    }
}

/// The phone's controller, as the session passes it on.
pub enum GamepadInput {
    State(vyoma_core::gamepad::State),
    Close,
}

/// The PC's live numbers, sampled every couple of seconds while the phone's dashboard is open.
pub trait Sampler: Send {
    fn sample(&mut self) -> Pin<Box<dyn Future<Output = Value> + Send + '_>>;
}

fn cannot<T>() -> Result<T> {
    bail!("not on this computer")
}

/// What an OS does for the engine.
pub trait Platform: Send + Sync + 'static {
    /// Whether [feature] works here: only those are offered to the phone.
    fn supports(&self, feature: Feature) -> bool;

    /// Installs what [vyoma_core::files] calls back into; once, before any session.
    fn install_file_hooks(&self) {}

    /// How the activity log names this computer ("Saved X to this Mac").
    fn this_computer(&self) -> &'static str {
        "this PC"
    }
    /// The activity log's line for waking it.
    fn wake_text(&self) -> &'static str {
        "Woke and unlocked this PC"
    }

    fn stats(&self) -> Result<Box<dyn Sampler>> {
        cannot()
    }
    /// On a charger; None without a battery.
    fn on_ac(&self) -> impl Future<Output = Option<bool>> + Send {
        async { None }
    }
    /// Whether a photo can be taken now, and if not, whether a closed lid is why.
    fn camera_state(&self) -> impl Future<Output = (bool, bool)> + Send {
        async { (false, false) }
    }
    /// The title of a video the phone could continue, if one is playing.
    fn handoff_offer(&self) -> Option<String> {
        None
    }
    fn handoff(&self) -> impl Future<Output = Option<Handoff>> + Send {
        async { None }
    }
    fn pause_handoff(&self, _: &Handoff) -> impl Future<Output = ()> + Send {
        async {}
    }

    fn specs(&self) -> impl Future<Output = Value> + Send {
        async { serde_json::json!({ "error": "not on this computer" }) }
    }
    /// Takes a screenshot and sends it to the phone.
    fn screenshot(&self, _: &Cx) -> impl Future<Output = Result<()>> + Send {
        async { cannot() }
    }
    /// Takes a photo with this computer's camera and sends it to the phone.
    fn camera_photo(&self, _: &Cx) -> impl Future<Output = Result<()>> + Send {
        async { cannot() }
    }
    fn screen_fps(&self, _asked: Option<u64>) -> u32 {
        30
    }
    /// Casts the screen to the phone until cut off, doing the viewer's touches from [inputs].
    fn screen(&self, _: &Cx, _fps: u32, _inputs: mpsc::UnboundedReceiver<Value>) -> impl Future<Output = Result<()>> + Send {
        async { cannot() }
    }
    fn webcam(&self, _: &Cx, _phone: IpAddr, _packet: &Value, _turn: watch::Receiver<u32>) -> impl Future<Output = Result<()>> + Send {
        async { cannot() }
    }
    fn webcam_mic(&self, _: &Cx, _phone: IpAddr, _packet: &Value) -> impl Future<Output = Result<()>> + Send {
        async { cannot() }
    }
    fn mirror(&self, _: &Cx, _phone: IpAddr, _packet: &Value) -> impl Future<Output = Result<()>> + Send {
        async { cannot() }
    }
    fn mirror_sound(&self, _: &Cx, _phone: IpAddr, _packet: &Value) -> impl Future<Output = Result<()>> + Send {
        async { cannot() }
    }
    /// The mirrored phone's copy, onto this computer's clipboard.
    fn mirror_copy(&self, _text: &str) -> impl Future<Output = ()> + Send {
        async {}
    }
    fn mirror_volume(&self, _up: bool) -> impl Future<Output = ()> + Send {
        async {}
    }
    /// Puts [text] into the app in front, as if pasted.
    fn paste(&self, _text: &str) -> impl Future<Output = Result<()>> + Send {
        async { cannot() }
    }
    /// One step of the overview gesture; what's showing after it.
    fn desktop_step(&self, _up: bool) -> impl Future<Output = Result<&'static str>> + Send {
        async { cannot() }
    }
    fn desktop_switch(&self, _right: bool) -> impl Future<Output = Result<()>> + Send {
        async { cannot() }
    }
    fn wallpaper_colors(&self) -> impl Future<Output = Result<Vec<[u8; 3]>>> + Send {
        async { cannot() }
    }
    /// Changes when the wallpaper does (checked every few seconds); None when it can't be told.
    fn wallpaper_signature(&self) -> Option<String> {
        None
    }
    /// Takes the phone's [Feature::Controls], [Feature::Power] and [Feature::Wake] packets in order,
    /// answering through [cx].
    fn pc_controls(&self, _cx: &Cx) -> Option<mpsc::UnboundedSender<Value>> {
        None
    }
    /// Runs the phone's controller until its input ends.
    fn gamepad(&self, _cx: &Cx, _input: mpsc::Receiver<GamepadInput>) -> impl Future<Output = ()> + Send {
        async {}
    }
    fn usb_accessory(&self, _reswitch: bool) -> Result<()> {
        cannot()
    }
    /// The touchpad over Bluetooth, while its page is open.
    fn touchpad(&self, _: &Cx) -> Option<Started> {
        None
    }
    /// So the phone can find this PC over Bluetooth once Wi-Fi is gone.
    fn bluetooth_address(&self) -> impl Future<Output = Option<String>> + Send {
        async { None }
    }
    /// The phone's walk-away rules.
    fn away(&self, _phone: &str, _body: &Value) {}
}

/// What to tell phones this PC takes and sends, for [platform].
pub fn capabilities<P: Platform>(platform: &P) -> (Vec<&'static str>, Vec<&'static str>) {
    capabilities_for(|f| platform.supports(f))
}

/// What to tell phones, for the features [offers] says yes to (before there's a platform to ask).
pub fn capabilities_for(offers: impl Fn(Feature) -> bool) -> (Vec<&'static str>, Vec<&'static str>) {
    let offered: Vec<Feature> = Feature::ALL.into_iter().filter(|&f| offers(f)).collect();
    let incoming = offered.iter().flat_map(|f| f.incoming().iter().copied()).collect();
    // A refusal can answer any of them (a switch in the window turned it off).
    let outgoing = offered.iter().flat_map(|f| f.outgoing().iter().copied()).chain(["vyoma.denied"]).collect();
    (incoming, outgoing)
}
