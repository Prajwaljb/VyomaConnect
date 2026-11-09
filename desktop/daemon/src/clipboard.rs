//! The PC's clipboard, shared with paired devices as the stock plugin does: each change goes to
//! them as `kdeconnect.clipboard`, and a device that links gets the current one with when it was
//! copied (`kdeconnect.clipboard.connect`), which it takes only if that's newer than its own.
//!
//! Through `vyoma_companion::clip`: wl-clipboard where the compositor has the data-control
//! protocol (KWin, wlroots, COSMIC), Vyoma's GNOME extension on GNOME.

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use tokio::sync::mpsc;
use vyoma_core::link::packet;

pub const CLIPBOARD: &str = "kdeconnect.clipboard";
pub const CLIPBOARD_CONNECT: &str = "kdeconnect.clipboard.connect";

/// Larger copies stay on the PC: every device would get the whole of it on each change.
const MAX_TEXT: usize = 1 << 20;

/// What's on the clipboard as far as the devices know, and since when (ms).
#[derive(Default)]
pub struct Clipboard {
    content: String,
    since: i64,
}

impl Clipboard {
    /// A copy on this PC, as read after a change: the packet to send, unless it's no change
    /// (our own echo after [set], or a copy of the same text again).
    pub fn changed(&mut self, content: String) -> Option<Value> {
        if content.is_empty() || content == self.content {
            return None;
        }
        self.content = content;
        self.since = now_ms();
        Some(packet(CLIPBOARD, json!({ "content": self.content })))
    }

    /// What a device that just linked is sent.
    pub fn connect_packet(&self) -> Option<Value> {
        (!self.content.is_empty()).then(|| packet(CLIPBOARD_CONNECT, json!({ "content": self.content, "timestamp": self.since })))
    }

    /// A device's packet: the text to put on this PC's clipboard, if it should go there.
    pub fn received(&mut self, p: &Value) -> Option<String> {
        let body = &p["body"];
        let content = body["content"].as_str().filter(|c| !c.is_empty() && *c != self.content)?;
        let since = match p["type"].as_str()? {
            CLIPBOARD => now_ms(),
            CLIPBOARD_CONNECT => {
                // 0: the device doesn't know when it was copied; only something newer replaces ours.
                let at = body["timestamp"].as_i64().or_else(|| body["timestamp"].as_str()?.parse().ok()).unwrap_or(0);
                if at <= 0 || at <= self.since {
                    return None;
                }
                at
            }
            _ => return None,
        };
        self.content = content.to_owned();
        self.since = since;
        Some(self.content.clone())
    }
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Sends the clipboard's text each time it changes, for as long as it can follow it.
pub async fn watch(out: mpsc::UnboundedSender<String>) -> anyhow::Result<()> {
    let (tick, mut ticks) = mpsc::unbounded_channel::<()>();
    let reader = tokio::spawn(async move {
        while ticks.recv().await.is_some() {
            // A burst of changes (an app offering several types) reads once.
            while ticks.try_recv().is_ok() {}
            let Some(text) = current().await else { continue };
            if out.send(text).is_err() {
                break;
            }
        }
    });
    let followed = vyoma_companion::clip::follow(move || {
        let _ = tick.send(());
    })
    .await;
    reader.abort();
    followed
}

/// The clipboard's text, unless it has none, is too big or is a password.
async fn current() -> Option<String> {
    let text = vyoma_companion::clip::text().await?;
    if text.len() > MAX_TEXT {
        tracing::info!("clipboard holds {} KiB of text, too much to send", text.len() >> 10);
        return None;
    }
    Some(text)
}

/// Puts [text] on this PC's clipboard.
pub async fn set(text: &str) {
    vyoma_companion::clip::set(text).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_echo_and_older_copies_are_ignored() {
        let mut c = Clipboard::default();
        assert!(c.connect_packet().is_none(), "nothing copied yet");
        let sent = c.changed("one".into()).unwrap();
        assert_eq!(sent["body"], json!({ "content": "one" }));
        assert!(c.changed("one".into()).is_none());

        assert_eq!(c.received(&packet(CLIPBOARD, json!({ "content": "two" }))).as_deref(), Some("two"));
        assert!(c.changed("two".into()).is_none(), "the echo of what a device set");

        let since = c.connect_packet().unwrap()["body"]["timestamp"].as_i64().unwrap();
        assert!(c.received(&packet(CLIPBOARD_CONNECT, json!({ "content": "old", "timestamp": since - 1 }))).is_none());
        assert!(c.received(&packet(CLIPBOARD_CONNECT, json!({ "content": "unknown", "timestamp": 0 }))).is_none());
        assert_eq!(c.received(&packet(CLIPBOARD_CONNECT, json!({ "content": "new", "timestamp": since + 1 }))).as_deref(), Some("new"));
        assert_eq!(c.connect_packet().unwrap()["body"], json!({ "content": "new", "timestamp": since + 1 }));
    }
}
