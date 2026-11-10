//! The PC's clipboard as text, wherever it runs: on the X server itself in X11 sessions (`x11.rs`),
//! through Vyoma's GNOME extension on GNOME (which has no clipboard protocol for apps), through
//! wl-clipboard elsewhere (the data-control protocol of KWin, wlroots and COSMIC).

use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

/// Password managers (KeePassXC, KWallet) mark what they copy with this; it's never read.
pub const SECRET_HINT: &str = "x-kde-passwordManagerHint";

/// On GNOME (Wayland), where the extension is the only way.
fn through_shell() -> bool {
    crate::desktop::is_gnome() && !crate::x11::active()
}

/// Where only text can be read and put back (no other types, as wl-clipboard has).
pub fn text_only() -> bool {
    through_shell() || crate::x11::active()
}

/// The clipboard's text, unless it holds none or a password manager's copy.
pub async fn text() -> Option<String> {
    if crate::x11::active() {
        return crate::x11::text().await;
    }
    if through_shell() {
        return crate::shell::clipboard().await.ok().filter(|t| !t.is_empty());
    }
    let types = String::from_utf8(wl_paste(&["--list-types"]).await?).ok()?;
    if types.lines().any(|t| t == SECRET_HINT) || !types.lines().any(|t| t.starts_with("text/") || t == "UTF8_STRING" || t == "STRING" || t == "TEXT") {
        return None;
    }
    String::from_utf8(wl_paste(&["--no-newline", "--type", "text"]).await?).ok().filter(|t| !t.is_empty())
}

/// Puts [text] on the clipboard; whether it took.
pub async fn set(text: &str) -> bool {
    if crate::x11::active() {
        return crate::x11::set(Some(text)).await;
    }
    if through_shell() {
        return match crate::shell::set_clipboard(text).await {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!("can't set the clipboard: {e:#}");
                false
            }
        };
    }
    copy_as("text/plain;charset=utf-8", text.as_bytes()).await
}

/// Calls [changed] on each copy, for as long as it can follow them.
pub async fn follow(mut changed: impl FnMut()) -> anyhow::Result<()> {
    if crate::x11::active() {
        return crate::x11::follow(changed).await;
    }
    if through_shell() {
        return crate::shell::follow_clipboard(changed).await;
    }
    let mut child = Command::new("wl-paste").args(["--watch", "echo"]).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true).spawn()?;
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    while lines.next_line().await?.is_some() {
        changed();
    }
    anyhow::bail!("wl-paste stopped: {}", child.wait().await?)
}

/// Empties the clipboard.
pub async fn clear() {
    if crate::x11::active() {
        crate::x11::set(None).await;
    } else if through_shell() {
        set("").await;
    } else {
        let _ = Command::new("wl-copy").arg("--clear").status().await;
    }
}

/// wl-paste waits on whichever app owns the clipboard, which can be frozen.
pub async fn wl_paste(args: &[&str]) -> Option<Vec<u8>> {
    let out = tokio::time::timeout(Duration::from_secs(2), Command::new("wl-paste").args(args).stderr(Stdio::null()).kill_on_drop(true).output()).await.ok()?.ok()?;
    out.status.success().then_some(out.stdout)
}

/// Puts [bytes] on the clipboard as [kind] through wl-copy (not GNOME).
pub async fn copy_as(kind: &str, bytes: &[u8]) -> bool {
    let Ok(mut c) = Command::new("wl-copy").args(["--type", kind]).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() else {
        tracing::warn!("can't set the clipboard: no wl-copy");
        return false;
    };
    if let Some(mut stdin) = c.stdin.take() {
        let _ = stdin.write_all(bytes).await;
    }
    // wl-copy forks a server to hold the clipboard and its first process exits.
    matches!(tokio::time::timeout(Duration::from_secs(2), c.wait()).await, Ok(Ok(s)) if s.success())
}
