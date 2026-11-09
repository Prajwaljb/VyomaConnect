//! The touchpad's three-finger swipes, stepped the way Plasma's own touchpad gestures step:
//! up goes desktop → windows → Overview → Grid, down goes back Grid → Overview → windows → desktop.
//! A shortcut alone can't do that (Meta+D in Overview, or Meta+W over the shown desktop, land
//! somewhere odd), so each step reads where KWin is first. On GNOME, Vyoma's extension steps the
//! Overview as GNOME's own swipes do: windows → Overview → apps grid.
//!
//! `vyoma.desktop.step {up: bool}` → `vyoma.desktop.stepped {view: "desktop"|"windows"|"overview"|"grid"}`
//! (where it ended up) or `{error}`. Sideways, `vyoma.desktop.switch {right: bool}` goes to the next
//! or previous virtual desktop (workspace on GNOME), no answer: each desktop's own shortcut differs.

use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use zbus::Connection;
use zbus::OwnedVariant;

/// The session runs GNOME (also Ubuntu's and other GNOME-based ones), from the desktop name the
/// session gives its services.
pub fn is_gnome() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.split(':').any(|p| p.eq_ignore_ascii_case("gnome")))
}

/// The desktop's first keyboard layout and its variant ("de", "nodeadkeys"), for pressing the
/// keys that type each character: GNOME's input sources, or Plasma's kxkbrc; "us" when neither says.
pub fn keyboard_layout() -> (String, String) {
    let from_gnome = || {
        let out = std::process::Command::new("gsettings").args(["get", "org.gnome.desktop.input-sources", "sources"]).output().ok()?;
        parse_gnome_sources(&String::from_utf8_lossy(&out.stdout))
    };
    let from_plasma = || {
        let home = std::env::var("HOME").ok()?;
        let text = std::fs::read_to_string(format!("{home}/.config/kxkbrc")).ok()?;
        let first = |key: &str| text.lines().find_map(|l| l.strip_prefix(key).map(|v| v.split(',').next().unwrap_or("").trim().to_owned()));
        first("LayoutList=").filter(|l| !l.is_empty()).map(|l| (l, first("VariantList=").unwrap_or_default()))
    };
    let found = if is_gnome() { from_gnome() } else { from_plasma() };
    found.unwrap_or_else(|| ("us".into(), String::new()))
}

/// `[('xkb', 'de+nodeadkeys'), ('ibus', 'mozc')]` → the first xkb one, split.
fn parse_gnome_sources(text: &str) -> Option<(String, String)> {
    let at = text.find("('xkb', '")? + "('xkb', '".len();
    let name = &text[at..at + text[at..].find('\'')?];
    let (layout, variant) = name.split_once('+').unwrap_or((name, ""));
    (!layout.is_empty()).then(|| (layout.to_owned(), variant.to_owned()))
}

/// Scrolls through the RemoteDesktop portal: GNOME reads them the other way round from Plasma
/// (measured: the touchpad scrolled backwards there), so they're turned for it.
pub fn scroll_sign() -> f64 {
    if is_gnome() { -1.0 } else { 1.0 }
}

/// GNOME takes the portal's scrolls as fingers on a touchpad, which go on until told they lifted;
/// Plasma takes each as a wheel turn.
pub fn scroll_needs_finish() -> bool {
    is_gnome()
}

/// The next virtual desktop to the right, or the left.
pub async fn switch(right: bool) -> Result<()> {
    if is_gnome() {
        crate::shell::switch_workspace(right).await?;
        return Ok(());
    }
    let conn = Connection::session().await.context("no session bus")?;
    shortcut(&conn, if right { "Switch One Desktop to the Right" } else { "Switch One Desktop to the Left" }).await
}

/// Whether the open Overview is in its grid mode. KWin shows both as one effect, so this is
/// remembered from the steps taken here (and forgotten once the Overview closes).
static GRID: AtomicBool = AtomicBool::new(false);

async fn get(conn: &Connection, path: &str, iface: &str, name: &str) -> Result<OwnedVariant> {
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
    if is_gnome() {
        return Ok(match crate::shell::step(up).await?.as_str() {
            "grid" => "grid",
            "overview" => "overview",
            _ => "windows",
        });
    }
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
    #[test]
    fn reads_gnomes_layout() {
        assert_eq!(parse_gnome_sources("[('ibus', 'mozc'), ('xkb', 'de+nodeadkeys')]\n"), Some(("de".into(), "nodeadkeys".into())));
        assert_eq!(parse_gnome_sources("[('xkb', 'us')]"), Some(("us".into(), String::new())));
        assert_eq!(parse_gnome_sources("@a(ss) []"), None);
    }

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
