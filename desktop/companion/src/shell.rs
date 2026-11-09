//! Vyoma's GNOME Shell extension (`desktop/gnome-extension`). GNOME gives apps no way to follow
//! the clipboard or draw over the desktop, and only lets its own parts open the Overview, so the
//! extension does those inside the shell and offers them on the session bus:
//! `dev.vyoma.Shell` at `/dev/vyoma/Shell`, interface `dev.vyoma.Shell1`.
//!
//! - `GetClipboard() → s`: the clipboard's text ("" when none, or a password manager's copy).
//! - `SetClipboard(s)`.
//! - signal `ClipboardChanged()`: no text in it, as any app can listen to signals.
//! - `Point(d dx, d dy)` and `HidePointer()`: the presenter's dot, moved as kdeconnectd moves it
//!   (the screen 2 across, dy scaled by its aspect); it shows in the middle and hides after a quiet second.
//! - `Step(b up) → s`: the touchpad's three-finger swipe, windows ↔ Overview ↔ apps: "windows",
//!   "overview" or "grid", the names Plasma's steps use.
//! - `SwitchWorkspace(b right)`: the next workspace that way, with GNOME's own animation.
//! - `Screenshot() → ay`: the whole desktop as PNG, captured by the shell without asking.
//! - `Screens() → a(ii)`: each screen's logical size.
//! - `AskReply(s heading, s message, s placeholder) → s`: GNOME's own dialog with a text field, for
//!   replying to a phone's notification (GNOME's notifications have no reply field); "" if cancelled.
//! - property `Version` (u).
//!
//! It answers only `vyoma-desktop` (it checks the caller's program), so other apps can't read the
//! clipboard through it.

use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::StreamExt;
use zbus::{Connection, MatchRule, MessageStream};

pub const BUS: &str = "dev.vyoma.Shell";
const PATH: &str = "/dev/vyoma/Shell";
const IFACE: &str = "dev.vyoma.Shell1";
/// Calls into the shell are quick; a frozen shell mustn't hold anything up.
const LIMIT: Duration = Duration::from_secs(2);
/// The extension's `Version` this build expects; an older one runs until the next login.
pub const VERSION: u32 = 3;

/// The extension is running in this session.
pub async fn available() -> bool {
    let Ok(conn) = Connection::session().await else { return false };
    match conn.call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "NameHasOwner", &(BUS,)).await {
        Ok(reply) => reply.body().deserialize::<bool>().unwrap_or(false),
        Err(_) => false,
    }
}

/// The running extension's version, if it runs.
pub async fn version() -> Option<u32> {
    let conn = Connection::session().await.ok()?;
    let reply = conn.call_method(Some(BUS), PATH, Some("org.freedesktop.DBus.Properties"), "Get", &(IFACE, "Version")).await.ok()?;
    u32::try_from(reply.body().deserialize::<zbus::OwnedVariant>().ok()?).ok()
}

async fn call<B>(method: &str, body: &B) -> Result<zbus::Message>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    let conn = Connection::session().await.context("no session bus")?;
    let reply = tokio::time::timeout(LIMIT, conn.call_method(Some(BUS), PATH, Some(IFACE), method, body)).await.context("GNOME Shell didn't answer")?;
    explain(reply, method).await
}

async fn explain(reply: zbus::Result<zbus::Message>, method: &str) -> Result<zbus::Message> {
    match reply {
        Ok(m) => Ok(m),
        Err(_) if !available().await => anyhow::bail!("Vyoma's GNOME extension isn't on: log out and back in once, or turn it on in the Extensions app"),
        Err(e) => Err(e).with_context(|| format!("Vyoma's GNOME extension refused {method}")),
    }
}

pub async fn clipboard() -> Result<String> {
    Ok(call("GetClipboard", &()).await?.body().deserialize::<String>()?)
}

pub async fn set_clipboard(text: &str) -> Result<()> {
    call("SetClipboard", &(text,)).await?;
    Ok(())
}

/// Calls [changed] each time the clipboard changes, for as long as the extension runs.
pub async fn follow_clipboard(mut changed: impl FnMut()) -> Result<()> {
    let conn = Connection::session().await.context("no session bus")?;
    let rule = MatchRule::builder().msg_type(zbus::message::Type::Signal).interface(IFACE).member("ClipboardChanged").path(PATH).build()?;
    let mut signals = MessageStream::for_match_rule(rule, &conn, None).await?;
    // And when the extension goes (disabled, or the shell restarted), to notice it.
    let gone = MatchRule::builder().msg_type(zbus::message::Type::Signal).interface("org.freedesktop.DBus").member("NameOwnerChanged").arg(0, BUS).build()?;
    let mut owners = MessageStream::for_match_rule(gone, &conn, None).await?;
    anyhow::ensure!(available().await, "Vyoma's GNOME extension isn't running");
    loop {
        tokio::select! {
            Some(Ok(_)) = signals.next() => changed(),
            Some(Ok(m)) = owners.next() => {
                let (_, _, new): (String, String, String) = m.body().deserialize()?;
                if new.is_empty() {
                    anyhow::bail!("Vyoma's GNOME extension stopped");
                }
            }
            else => anyhow::bail!("the session bus closed"),
        }
    }
}

pub async fn point(dx: f64, dy: f64) -> Result<()> {
    call("Point", &(dx, dy)).await?;
    Ok(())
}

pub async fn hide_pointer() -> Result<()> {
    call("HidePointer", &()).await?;
    Ok(())
}

pub async fn switch_workspace(right: bool) -> Result<()> {
    call("SwitchWorkspace", &(right,)).await?;
    Ok(())
}

pub async fn screenshot() -> Result<Vec<u8>> {
    let conn = Connection::session().await.context("no session bus")?;
    // A big desktop takes a moment to encode.
    let reply = tokio::time::timeout(Duration::from_secs(15), conn.call_method(Some(BUS), PATH, Some(IFACE), "Screenshot", &()))
        .await
        .context("GNOME Shell didn't answer")?;
    Ok(explain(reply, "Screenshot").await?.body().deserialize::<Vec<u8>>()?)
}

pub async fn screens() -> Result<Vec<(i32, i32)>> {
    Ok(call("Screens", &()).await?.body().deserialize()?)
}

/// Asks for a reply to [message] in a dialog; `None` if it was cancelled or left for 10 minutes.
pub async fn ask_reply(heading: &str, message: &str, placeholder: &str) -> Result<Option<String>> {
    let conn = Connection::session().await.context("no session bus")?;
    let body = (heading, message, placeholder);
    let call = conn.call_method(Some(BUS), PATH, Some(IFACE), "AskReply", &body);
    let Ok(reply) = tokio::time::timeout(Duration::from_secs(600), call).await else { return Ok(None) };
    let text: String = explain(reply, "AskReply").await?.body().deserialize()?;
    Ok(Some(text).filter(|t| !t.trim().is_empty()))
}

/// One three-finger swipe; where the shell ended up: "windows", "overview" or "grid".
pub async fn step(up: bool) -> Result<String> {
    Ok(call("Step", &(up,)).await?.body().deserialize::<String>()?)
}
