//! The phone as a game controller, once for every PC side: the states as they come in (the session's
//! packets, then the fast routes: UDP, the cable), kept in order, the pad made with the first and
//! remade when the phone asks for another kind, the game's rumble sent back, the gyro's mouse, and
//! everything let go when the phone stops. What a pad is on this computer is a [Pads]'s: uinput on
//! Linux (the companion's gamepad.rs, which has the wire format), a USB/IP device on Windows.

use std::future::Future;
use std::time::Duration;

use anyhow::Result;
use serde_json::json;
use tokio::sync::mpsc;
use tracing::{info, warn};
use vyoma_core::gamepad::{Fast, Kind, Order, State};
use vyoma_core::link::{Sender, packet};

use crate::GamepadInput;

/// Without a state (or the heartbeat) for this long the phone has gone: let everything go.
pub const IDLE: Duration = Duration::from_secs(4);

/// A virtual pad games take for a real one. Dropping it removes it, every button let go.
pub trait VirtualPad: Send {
    /// The kind the phone asked for (a platform without it may stand in another).
    fn kind(&self) -> Kind;
    /// Sends what changed since the last state.
    fn apply(&mut self, s: State) -> Result<()>;
    /// Waits for the game to play (or stop) a rumble: strong, weak (0..65535) and ms (0 = until the next).
    fn rumble(&mut self) -> impl Future<Output = Result<(u16, u16, u16)>> + Send;
}

/// The gyro's virtual mouse (aiming in PC games, which take a mouse over a stick).
pub trait GyroMouse: Send {
    fn moved(&mut self, dx: i32, dy: i32) -> Result<()>;
}

/// How this computer makes them.
pub trait Pads: Send {
    type Pad: VirtualPad;
    type Mouse: GyroMouse;
    /// Err in words the phone can show (it does, as the controller's reason).
    fn pad(&mut self, kind: Kind) -> impl Future<Output = Result<Self::Pad>> + Send;
    fn mouse(&mut self) -> Result<Self::Mouse>;
}

/// Stutter on the side channels: they repeat at least every 100 ms, so a longer silence is the
/// route's fault, not an idle thumb.
pub struct Gaps {
    started: std::time::Instant,
    last: Option<std::time::Instant>,
    frames: u64,
    long: u64,
    longest: Duration,
}

impl Default for Gaps {
    fn default() -> Gaps {
        Gaps::new()
    }
}

impl Gaps {
    const LONG: Duration = Duration::from_millis(150);

    pub fn new() -> Gaps {
        Gaps { started: std::time::Instant::now(), last: None, frames: 0, long: 0, longest: Duration::ZERO }
    }

    pub fn frame(&mut self) {
        let now = std::time::Instant::now();
        if let Some(last) = self.last {
            let gap = now - last;
            self.longest = self.longest.max(gap);
            self.long += (gap > Self::LONG) as u64;
        }
        self.last = Some(now);
        self.frames += 1;
    }

    pub fn report(&self, name: &str) {
        if self.frames > 0 {
            info!(
                "{name}: controller frames: {} in {} s, {} gaps over {} ms, longest {} ms",
                self.frames, self.started.elapsed().as_secs(), self.long, Self::LONG.as_millis(), self.longest.as_millis()
            );
        }
    }
}

/// Runs the pad for one phone while it sends states; returns when it closes or goes quiet.
pub async fn drive<P: Pads>(mut pads: P, mut input: mpsc::Receiver<GamepadInput>, sender: Sender, name: String) {
    // Only the phone's own new states count: a rumbling game or a replayed datagram wakes this loop
    // too, and mustn't keep a held button down for ever.
    let mut quiet_at = tokio::time::Instant::now() + IDLE;
    let mut pad: Option<P::Pad> = None;
    let mut mouse: Option<P::Mouse> = None;
    let mut fast: Option<Fast> = None;
    let mut order = Order::default();
    // The gyro mouse's totals last seen (a fresh stream starts its own from anywhere).
    let mut moved: Option<(u32, i32, i32)> = None;
    let mut over_udp = false;
    let mut gaps = Gaps::new();
    loop {
        let rumble = async {
            match pad.as_mut() {
                Some(p) => p.rumble().await,
                None => std::future::pending().await,
            }
        };
        let datagram = async {
            match fast.as_mut() {
                Some(f) => f.next().await,
                None => std::future::pending().await,
            }
        };
        let state = tokio::select! {
            got = tokio::time::timeout_at(quiet_at, input.recv()) => match got {
                Ok(Some(GamepadInput::State(s))) => s,
                Ok(Some(GamepadInput::Close)) | Ok(None) | Err(_) => {
                    if pad.take().is_some() {
                        info!("{name}: controller disconnected");
                    }
                    gaps.report(&name);
                    return;
                }
            },
            (s, wired) = datagram => {
                gaps.frame();
                if !wired && !over_udp {
                    over_udp = true;
                    info!("{name}: controller input arriving over UDP");
                }
                s
            }
            r = rumble => {
                match r {
                    Ok((strong, weak, ms)) => {
                        let _ = sender.send(&packet("vyoma.gamepad.rumble", json!({ "strong": strong, "weak": weak, "ms": ms }))).await;
                    }
                    Err(e) => {
                        warn!("{name}: controller feedback failed: {e:#}");
                        return;
                    }
                }
                continue;
            }
        };
        if !order.fresh(&state) {
            continue;
        }
        quiet_at = tokio::time::Instant::now() + IDLE;
        // A different pad asked for: the old one goes (letting go of everything) and the new one comes.
        if pad.as_ref().is_some_and(|p| p.kind() != state.kind) {
            pad = None;
        }
        if pad.is_none() {
            match pads.pad(state.kind).await {
                Ok(p) => {
                    info!("{name}: controller connected ({:?})", state.kind);
                    pad = Some(p);
                }
                Err(e) => {
                    warn!("{name}: no virtual controller: {e:#}");
                    let _ = sender.send(&packet("vyoma.gamepad.rumble", json!({ "error": format!("{e:#}") }))).await;
                    return;
                }
            }
            if fast.is_none() {
                match Fast::open().await {
                    Ok((f, port, key)) => {
                        fast = Some(f);
                        let _ = sender.send(&packet("vyoma.gamepad.udp", json!({ "port": port, "key": key }))).await;
                    }
                    Err(e) => warn!("{name}: controller stays on the link (no UDP port: {e:#})"),
                }
            }
        }
        if let Some(p) = pad.as_mut()
            && let Err(e) = p.apply(state)
        {
            warn!("{name}: controller input failed: {e:#}");
        }
        let sid = state.order.map_or(0, |o| o.0);
        let (dx, dy) = match moved {
            Some((s, x, y)) if s == sid => (state.mx.wrapping_sub(x), state.my.wrapping_sub(y)),
            _ => (0, 0),
        };
        moved = Some((sid, state.mx, state.my));
        if dx != 0 || dy != 0 {
            if mouse.is_none() {
                match pads.mouse() {
                    Ok(m) => mouse = Some(m),
                    Err(e) => warn!("{name}: no gyro mouse: {e:#}"),
                }
            }
            if let Some(m) = mouse.as_mut()
                && let Err(e) = m.moved(dx, dy)
            {
                warn!("{name}: gyro mouse failed: {e:#}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// What the fake pads saw.
    #[derive(Default)]
    struct Seen {
        made: Vec<Kind>,
        applied: Vec<u32>,
        dropped: usize,
        moved: Vec<(i32, i32)>,
    }

    struct FakePad {
        kind: Kind,
        seen: Arc<Mutex<Seen>>,
        rumbles: mpsc::UnboundedReceiver<(u16, u16, u16)>,
    }

    impl VirtualPad for FakePad {
        fn kind(&self) -> Kind {
            self.kind
        }
        fn apply(&mut self, s: State) -> Result<()> {
            self.seen.lock().unwrap().applied.push(s.buttons);
            Ok(())
        }
        async fn rumble(&mut self) -> Result<(u16, u16, u16)> {
            match self.rumbles.recv().await {
                Some(r) => Ok(r),
                None => std::future::pending().await,
            }
        }
    }

    impl Drop for FakePad {
        fn drop(&mut self) {
            self.seen.lock().unwrap().dropped += 1;
        }
    }

    struct FakeMouse(Arc<Mutex<Seen>>);

    impl GyroMouse for FakeMouse {
        fn moved(&mut self, dx: i32, dy: i32) -> Result<()> {
            self.0.lock().unwrap().moved.push((dx, dy));
            Ok(())
        }
    }

    struct Fakes {
        seen: Arc<Mutex<Seen>>,
        /// Each pad made gets the next rumble channel; none left: making one fails.
        rumbles: Vec<mpsc::UnboundedReceiver<(u16, u16, u16)>>,
    }

    impl Pads for Fakes {
        type Pad = FakePad;
        type Mouse = FakeMouse;
        async fn pad(&mut self, kind: Kind) -> Result<FakePad> {
            let rumbles = self.rumbles.pop().ok_or_else(|| anyhow::anyhow!("This PC needs a driver"))?;
            self.seen.lock().unwrap().made.push(kind);
            Ok(FakePad { kind, seen: self.seen.clone(), rumbles })
        }
        fn mouse(&mut self) -> Result<FakeMouse> {
            Ok(FakeMouse(self.seen.clone()))
        }
    }

    fn state(kind: Kind, seq: u32, buttons: u32, mx: i32) -> GamepadInput {
        GamepadInput::State(State { kind, buttons, mx, order: Some((1, seq)), ..Default::default() })
    }

    async fn sent(out: &mut mpsc::UnboundedReceiver<serde_json::Value>, kind: &str) -> serde_json::Value {
        loop {
            let p = tokio::time::timeout(Duration::from_secs(5), out.recv()).await.expect("in time").expect("open");
            if p["type"] == kind {
                return p;
            }
        }
    }

    #[tokio::test]
    async fn drives_the_pad_in_order_and_lets_go() {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let (rumble_a, rx_a) = mpsc::unbounded_channel();
        let (_rumble_b, rx_b) = mpsc::unbounded_channel();
        let fakes = Fakes { seen: seen.clone(), rumbles: vec![rx_b, rx_a] };
        let (tx, input) = mpsc::channel(16);
        let (out_tx, mut out) = mpsc::unbounded_channel();
        let driving = tokio::spawn(drive(fakes, input, Sender::queue(out_tx), "Test phone".into()));

        tx.send(state(Kind::Xbox, 1, 1, 0)).await.unwrap();
        tx.send(state(Kind::Xbox, 1, 9, 0)).await.unwrap(); // a repeat: dropped
        tx.send(state(Kind::Xbox, 2, 3, 5)).await.unwrap(); // the gyro moved 5 since the first state
        tx.send(state(Kind::Xbox, 3, 3, 12)).await.unwrap();
        let rumble = sent(&mut out, "vyoma.gamepad.udp").await;
        assert!(rumble["body"]["port"].is_u64(), "{rumble}");
        rumble_a.send((40000, 100, 250)).unwrap();
        let r = sent(&mut out, "vyoma.gamepad.rumble").await;
        assert_eq!((r["body"]["strong"].as_u64(), r["body"]["weak"].as_u64(), r["body"]["ms"].as_u64()), (Some(40000), Some(100), Some(250)));
        // Another kind asked for: the first pad goes, a new one comes.
        tx.send(state(Kind::DualSense, 4, 0, 12)).await.unwrap();
        tx.send(GamepadInput::Close).await.unwrap();
        driving.await.unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(seen.made, [Kind::Xbox, Kind::DualSense]);
        assert_eq!(seen.applied, [1, 3, 3, 0]);
        assert_eq!(seen.moved, [(5, 0), (7, 0)]);
        assert_eq!(seen.dropped, 2, "both pads gone at the end");
    }

    #[tokio::test]
    async fn a_pad_that_cant_be_made_is_explained_to_the_phone() {
        let fakes = Fakes { seen: Arc::default(), rumbles: vec![] };
        let (tx, input) = mpsc::channel(16);
        let (out_tx, mut out) = mpsc::unbounded_channel();
        let driving = tokio::spawn(drive(fakes, input, Sender::queue(out_tx), "Test phone".into()));
        tx.send(state(Kind::Xbox, 1, 1, 0)).await.unwrap();
        let why = sent(&mut out, "vyoma.gamepad.rumble").await;
        assert_eq!(why["body"]["error"], "This PC needs a driver");
        tokio::time::timeout(Duration::from_secs(5), driving).await.expect("it ends").unwrap();
    }
}
