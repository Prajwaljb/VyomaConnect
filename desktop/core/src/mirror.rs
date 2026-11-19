//! The phone's screen in a window on a PC (`vyoma.mirror`): what every PC side shares. The phone
//! sends its screen as live H.264 (Annex-B) and its sound as s16le PCM (`vyoma.mirror.audio`); the
//! window's clicks and keys go back as `vyoma.mirror.input` (0..1 across the picture), and
//! `vyoma.mirror.state` says when the window opens and closes. Each side draws and plays in its own
//! way; this is the stream splitting and the sound's jitter buffer.

use std::collections::VecDeque;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use serde_json::Value;
use tokio::io::AsyncReadExt;

use crate::identity::Identity;
use crate::payload;

pub const INPUT: &str = "vyoma.mirror.input";
pub const STATE: &str = "vyoma.mirror.state";
/// Most text one paste types into the phone.
pub const PASTE_MAX: usize = 20_000;

/// Splits an Annex-B stream into NAL units (without their start codes): the whole ones in [buf],
/// and how much of it they took. The last unit stays until the next start code shows it's whole.
pub fn split_nals(buf: &[u8]) -> (Vec<&[u8]>, usize) {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= buf.len() {
        if buf[i] == 0 && buf[i + 1] == 0 && buf[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut nals = Vec::new();
    for pair in starts.windows(2) {
        let (from, next) = (pair[0], pair[1] - 3);
        // A 4-byte start code's leading zero belongs to it, not to the unit before.
        let end = if next > from && buf[next - 1] == 0 { next - 1 } else { next };
        if end > from {
            nals.push(&buf[from..end]);
        }
    }
    let used = match starts.last() {
        Some(&last) => {
            let at = last - 3;
            if at > 0 && buf[at - 1] == 0 { at - 1 } else { at }
        }
        None => buf.len().saturating_sub(3),
    };
    (nals, used)
}

/// Pieces to have in hand before playing (absorbs the network's bursts) ...
const TARGET: usize = 4;
/// ... judged by the fewest there were over this long ...
const WINDOW: Duration = Duration::from_secs(2);
/// ... and never more than this (a stall that then floods in).
const MAX: usize = 30;

/// The phone's sound as it says it: 16-bit PCM at [rate] with [channels].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sound {
    pub rate: u32,
    pub channels: u32,
}

impl Sound {
    /// From a `vyoma.mirror.audio` body; PCM only (the PC said `sound: []`).
    pub fn of(body: &Value) -> Result<Sound> {
        if body["codec"].is_string() {
            bail!("only PCM here");
        }
        Ok(Sound { rate: body["rate"].as_u64().unwrap_or(48_000).clamp(8_000, 96_000) as u32, channels: body["channels"].as_u64().unwrap_or(2).clamp(1, 2) as u32 })
    }

    /// 10 ms of it, in bytes.
    pub fn piece(self) -> usize {
        (self.rate / 100 * 2 * self.channels) as usize
    }
}

/// Sound waiting for the output.
#[derive(Default)]
pub struct Queue {
    pieces: VecDeque<Vec<u8>>,
    priming: bool,
    low: usize,
    since: Option<Instant>,
}

pub type Shared = Arc<Mutex<Queue>>;

impl Queue {
    /// An empty queue that waits for a few pieces before it plays.
    pub fn shared() -> Shared {
        Arc::new(Mutex::new(Queue { priming: true, ..Default::default() }))
    }

    pub fn push(&mut self, piece: Vec<u8>) {
        self.pieces.push_back(piece);
        while self.pieces.len() > MAX {
            self.pieces.pop_front();
        }
    }

    /// Fills [out] with sound, silence where there's none yet; skips ahead if it has been more
    /// than [TARGET] behind all [WINDOW] long (the phone's clock runs a little fast).
    pub fn fill(&mut self, out: &mut [u8]) {
        let mut at = 0;
        if self.priming && self.pieces.len() < TARGET {
            out.fill(0);
            return;
        }
        self.priming = false;
        let since = *self.since.get_or_insert_with(Instant::now);
        self.low = if self.low == 0 { self.pieces.len() } else { self.low.min(self.pieces.len()) };
        if since.elapsed() >= WINDOW {
            for _ in 0..self.low.saturating_sub(TARGET).min(3) {
                self.pieces.pop_front();
            }
            (self.low, self.since) = (0, Some(Instant::now()));
        }
        while at < out.len() {
            let Some(mut piece) = self.pieces.pop_front() else {
                // Ran dry: wait for the target again.
                out[at..].fill(0);
                self.priming = true;
                return;
            };
            let n = piece.len().min(out.len() - at);
            out[at..at + n].copy_from_slice(&piece[..n]);
            at += n;
            if n < piece.len() {
                piece.drain(..n);
                self.pieces.push_front(piece);
            }
        }
    }
}

/// Reads the phone's sound stream into [queue] in 10 ms pieces until it ends.
pub async fn receive(me: &Identity, pinned: &[u8], phone: IpAddr, p: &Value, sound: Sound, queue: &Shared) -> Result<()> {
    let mut input = payload::fetch_stream(me, pinned, phone, p).await?;
    let piece = sound.piece();
    let mut buf: Vec<u8> = Vec::with_capacity(piece * 4);
    let mut chunk = vec![0u8; 16 * 1024];
    loop {
        let n = input.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
        let whole = buf.len() - buf.len() % piece;
        if whole > 0 {
            let mut q = queue.lock().unwrap();
            for p in buf[..whole].chunks(piece) {
                q.push(p.to_vec());
            }
            drop(q);
            buf.drain(..whole);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn splits_annex_b_into_whole_units() {
        let stream = [0, 0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x68, 3, 0, 0, 0, 1, 0x65, 4, 5, 6];
        let (nals, used) = split_nals(&stream);
        assert_eq!(nals, [&[0x67, 1, 2][..], &[0x68, 3]]);
        assert_eq!(&stream[used..], &[0, 0, 0, 1, 0x65, 4, 5, 6], "the last unit waits for the next start code");
        let (none, used) = split_nals(&[1, 2, 3, 4, 5]);
        assert!(none.is_empty());
        assert_eq!(used, 2, "keeps what could be the start of a start code");
    }

    #[test]
    fn waits_for_a_few_pieces_then_plays_them_in_order() {
        let mut q = Queue { priming: true, ..Default::default() };
        let mut out = [9u8; 4];
        q.push(vec![1, 1]);
        q.fill(&mut out);
        assert_eq!(out, [0; 4], "silence until there's some in hand");
        for n in 2..=4 {
            q.push(vec![n, n]);
        }
        q.fill(&mut out);
        assert_eq!(out, [1, 1, 2, 2]);
        q.fill(&mut out);
        assert_eq!(out, [3, 3, 4, 4]);
        q.fill(&mut out);
        assert_eq!(out, [0; 4], "ran dry");
        for n in 0..40 {
            q.push(vec![n, n]);
        }
        assert_eq!(q.pieces.len(), MAX, "a flood is cut to the newest");
    }

    #[test]
    fn reads_the_sound_the_phone_says() {
        assert_eq!(Sound::of(&json!({ "rate": 44100, "channels": 1 })).unwrap(), Sound { rate: 44100, channels: 1 });
        assert_eq!(Sound::of(&json!({})).unwrap().piece(), 1920);
        assert!(Sound::of(&json!({ "codec": "opus" })).is_err());
    }
}
