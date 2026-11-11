//! The PC's clipboard for the stock plugin (its rules are vyoma-core's `clipboard`), through
//! `vyoma_companion::clip`: wl-clipboard where the compositor has the data-control protocol (KWin,
//! wlroots, COSMIC), Vyoma's GNOME extension on GNOME, X11's selections on X11 desktops.

use tokio::sync::mpsc;

pub use vyoma_core::clipboard::{CLIPBOARD, CLIPBOARD_CONNECT, Clipboard, MAX_TEXT};

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
