//! A session-bus connection for the desktop portal that names us first: host apps are known to the
//! portal only by their systemd unit's name (`app-<id>-….scope`), and ours is `vyoma-desktop.service`, so
//! Plasma's "Remote Control Started" said "An unidentifiable application". `Register` (xdg-desktop-portal
//! 1.19+) ties this connection to `vyoma.desktop`; it must come before any other portal call on it.

use anyhow::{Context, Result};
use std::collections::HashMap;
use zbus::Connection;
use zbus::Variant;

/// The app id the portal shows: our desktop entry's name (`vyoma.desktop`).
pub const APP_ID: &str = "vyoma";

/// A new session-bus connection registered with the portal as [APP_ID]. An older portal without
/// `Register` still works, only unnamed.
pub async fn connection() -> Result<Connection> {
    let conn = Connection::session().await.context("no session bus")?;
    let options: HashMap<&str, Variant> = HashMap::new();
    if let Err(e) = conn
        .call_method(Some("org.freedesktop.portal.Desktop"), "/org/freedesktop/portal/desktop", Some("org.freedesktop.host.portal.Registry"), "Register", &(APP_ID, options))
        .await
    {
        tracing::debug!("portal won't take our name: {e}");
    }
    Ok(conn)
}
