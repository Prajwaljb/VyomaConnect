//! The touchpad's three-finger swipes, stepped the way Plasma's own touchpad gestures step:
//! up goes desktop → windows → Overview → Grid, down goes back Grid → Overview → windows → desktop.
//! A shortcut alone can't do that (Meta+D in Overview, or Meta+W over the shown desktop, land
//! somewhere odd), so each step reads where KWin is first.
//!
//! `vyoma.desktop.step {up: bool}` → `vyoma.desktop.stepped {view: "desktop"|"windows"|"overview"|"grid"}`
//! (where it ended up) or `{error}`.

use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use zbus::Connection;
use zbus::zvariant::OwnedValue;

/// Whether the open Overview is in its grid mode. KWin shows both as one effect, so this is
/// remembered from the steps taken here (and forgotten once the Overview closes).
static GRID: AtomicBool = AtomicBool::new(false);

async fn get(conn: &Connection, path: &str, iface: &str, name: &str) -> Result<OwnedValue> {
    Ok(conn
        .call_method(Some("org.kde.KWin"), path, Some("org.freedesktop.DBus.Properties"), "Get", &(iface, name))
        .await?
        .body()
        .deserialize()?)
}

async fn shortcut(conn: &Connection, name: &str) -> Result<()> {
    conn.call_method(Some("org.kde.kglobalaccel"), "/component/kwin", Some("org.kde.kglobalaccel.Component"), "invokeShortcut", &(name,))
        .await
        .with_context(|| format!("KWin didn't take \"{name}\""))?;
    Ok(())
}

/// KWin's own showDesktop(bool) over D-Bus does nothing on Plasma 6 Wayland (measured); its
/// shortcut toggles, so it's only pressed when the state is the other one.
async fn toggle_desktop(conn: &Connection) -> Result<()> {
    shortcut(conn, "Show Desktop").await
}

pub async fn step(up: bool) -> Result<&'static str> {
    let conn = Connection::session().await.context("no session bus")?;
    let effects: Vec<String> = get(&conn, "/Effects", "org.kde.kwin.Effects", "activeEffects").await?.try_into().context("activeEffects")?;
    let overview = effects.iter().any(|e| e == "overview");
    if !overview {
        GRID.store(false, Ordering::Relaxed);
    }
    let grid = GRID.load(Ordering::Relaxed);
    let desktop = bool::try_from(get(&conn, "/KWin", "org.kde.KWin", "showingDesktop").await?).context("showingDesktop")?;
    Ok(match (up, overview, grid, desktop) {
        // Up: windows back over the desktop, then the Overview, then its grid.
        (true, true, false, _) => {
            shortcut(&conn, "Grid View").await?;
            GRID.store(true, Ordering::Relaxed);
            "grid"
        }
        (true, true, true, _) => "grid",
        (true, false, _, true) => {
            toggle_desktop(&conn).await?;
            "windows"
        }
        (true, false, _, false) => {
            shortcut(&conn, "Overview").await?;
            "overview"
        }
        // Down: grid to Overview, Overview to the windows, the windows aside to show the desktop.
        (false, true, true, _) => {
            shortcut(&conn, "Overview").await?;
            GRID.store(false, Ordering::Relaxed);
            "overview"
        }
        (false, true, false, d) => {
            shortcut(&conn, "Overview").await?;
            if d { "desktop" } else { "windows" }
        }
        (false, false, _, false) => {
            toggle_desktop(&conn).await?;
            "desktop"
        }
        (false, false, _, true) => "desktop",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Walks the whole ladder on the real desktop and back to the windows (it changes the screen
    /// for a few seconds). Starts from the windows.
    #[tokio::test]
    #[ignore]
    async fn steps_up_and_down() {
        let pause = || tokio::time::sleep(std::time::Duration::from_millis(900));
        let mut seen = Vec::new();
        for up in [true, true, true, false, false, false, true] {
            seen.push(step(up).await.unwrap());
            pause().await;
        }
        assert_eq!(seen, ["overview", "grid", "grid", "overview", "windows", "desktop", "windows"]);
    }
}
