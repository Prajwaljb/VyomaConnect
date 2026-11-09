//! Typing what no key types (emoji, accents, other scripts): kdeconnectd and KWin only press keys a
//! keymap has, so such text goes on the clipboard for a Ctrl+V. Whatever the user had copied comes
//! back a moment after the last one, unless they've copied something new meanwhile.

use std::time::Duration;

use anyhow::{Result, bail};
use tokio::process::Command;
use tokio::sync::Mutex;

/// Long enough for the paste, and for the next emoji of a burst to keep the first save.
const RESTORE_AFTER: Duration = Duration::from_millis(1200);
/// Kept in this order of preference: a copied file is its list, not the path text beside it.
const TYPES: [&str; 4] = ["text/uri-list", "image/png", "text/plain;charset=utf-8", "text/plain"];

struct State {
    /// What to put back: its type and bytes, or None inside if the clipboard was empty.
    saved: Option<Option<(String, Vec<u8>)>>,
    text: String,
    generation: u64,
}

static STATE: Mutex<State> = Mutex::const_new(State { saved: None, text: String::new(), generation: 0 });

/// Puts [text] on the clipboard, ready for a paste, and arranges for the old content to return.
pub async fn put(text: &str) -> Result<()> {
    let mut state = STATE.lock().await;
    if state.saved.is_none() {
        state.saved = Some(read_clipboard().await);
    }
    crate::mirror::copy(text).await;
    if current_text().await.as_deref() != Some(text) {
        bail!("couldn't set the clipboard");
    }
    state.text = text.to_string();
    state.generation += 1;
    let generation = state.generation;
    tokio::spawn(async move {
        tokio::time::sleep(RESTORE_AFTER).await;
        let mut state = STATE.lock().await;
        if state.generation != generation {
            return;
        }
        let saved = state.saved.take();
        if current_text().await.as_deref() == Some(state.text.as_str()) {
            restore(saved.flatten()).await;
        }
    });
    Ok(())
}

async fn current_text() -> Option<String> {
    crate::clip::text().await
}

/// What's on the clipboard, to put back: its best type and bytes. GNOME's extension deals in
/// text only, so there it's the text.
async fn read_clipboard() -> Option<(String, Vec<u8>)> {
    if crate::desktop::is_gnome() {
        return crate::clip::text().await.map(|t| ("text/plain;charset=utf-8".to_owned(), t.into_bytes()));
    }
    let list = String::from_utf8_lossy(&crate::clip::wl_paste(&["--list-types"]).await?).into_owned();
    let offered: Vec<&str> = list.lines().collect();
    let kind = TYPES.iter().find(|t| offered.contains(t)).copied().or(offered.first().copied())?;
    let bytes = crate::clip::wl_paste(&["--no-newline", "--type", kind]).await?;
    Some((kind.to_string(), bytes))
}

async fn restore(saved: Option<(String, Vec<u8>)>) {
    match saved {
        Some((_, bytes)) if crate::desktop::is_gnome() => {
            crate::clip::set(&String::from_utf8_lossy(&bytes)).await;
        }
        Some((kind, bytes)) => {
            crate::clip::copy_as(&kind, &bytes).await;
        }
        None if crate::desktop::is_gnome() => {
            crate::clip::set("").await;
        }
        None => {
            let _ = Command::new("wl-copy").arg("--clear").status().await;
        }
    }
}
