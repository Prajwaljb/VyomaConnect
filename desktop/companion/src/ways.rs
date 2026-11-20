//! How this Linux desktop does each thing a phone asks (This PC's "How this PC does it", the
//! engine's [Way]s), asked the way each part chooses: X11 first in an X11 session, then GNOME's
//! extension on GNOME, then what the Wayland compositor offers, then the portals. The input order
//! is the daemon's `input.rs` (`Sink::open`), the rest each module's own (`clip.rs`, `screen.rs`,
//! `screenshot.rs`, `desktop.rs`, the daemon's `presenter.rs`).

use std::time::Duration;

use vyoma_engine::ways::Way;
use zbus::Connection;

use crate::screenshot::{self, Shooter};

const DATA_CONTROL: [&str; 2] = ["ext_data_control_manager_v1", "zwlr_data_control_manager_v1"];
const LAYER_SHELL: &str = "zwlr_layer_shell_v1";
/// A portal that doesn't answer in this long isn't there for the report.
const PORTAL_WAIT: Duration = Duration::from_secs(2);
const NEEDS_EXTENSION: &str = "Needs Vyoma's GNOME extension, which GNOME loads at the next login";

/// What the session offers, looked at once for the whole report.
#[derive(Clone, Debug, Default)]
pub struct Session {
    pub x11: bool,
    pub gnome: bool,
    /// Vyoma's GNOME extension is running.
    pub extension: bool,
    pub hyprland: bool,
    pub kwin: bool,
    /// The compositor's Wayland protocols.
    pub globals: Vec<String>,
    /// The compositor's virtual pointer and keyboard (`wlr.rs`).
    pub virtual_input: bool,
    pub remote_desktop_portal: bool,
    pub screencast_portal: bool,
    pub shooter: Option<Shooter>,
}

impl Session {
    pub async fn look() -> Session {
        let x11 = crate::x11::active();
        let gnome = crate::desktop::is_gnome() && !x11;
        let globals = if x11 { Vec::new() } else { tokio::task::spawn_blocking(crate::wlr::globals).await.ok().flatten().unwrap_or_default() };
        let conn = Connection::session().await.ok();
        let (remote_desktop_portal, screencast_portal, kwin) = match &conn {
            Some(c) => (portal(c, "org.freedesktop.portal.RemoteDesktop").await, portal(c, "org.freedesktop.portal.ScreenCast").await, has_owner(c, "org.kde.KWin").await),
            None => (false, false, false),
        };
        Session {
            x11,
            gnome,
            extension: gnome && crate::shell::available().await,
            hyprland: crate::hypr::active(),
            kwin,
            virtual_input: !x11 && crate::wlr::offered(),
            globals,
            remote_desktop_portal,
            screencast_portal,
            shooter: Some(screenshot::shooter()),
        }
    }

    fn offers(&self, protocol: &str) -> bool {
        self.globals.iter().any(|g| g == protocol)
    }

    /// Whose virtual pointer it is, for the words ([start]: at the start of a sentence).
    fn compositor(&self, start: bool) -> &'static str {
        match (self.hyprland, start) {
            (true, _) => "Hyprland's",
            (false, true) => "The compositor's",
            (false, false) => "the compositor's",
        }
    }

    /// Every row, in the phone's order of tiles.
    pub fn ways(&self) -> Vec<Way> {
        vec![self.input(), self.clipboard(), self.screen(), self.screenshots(), self.dot(), self.desktops()]
    }

    fn input(&self) -> Way {
        const NAME: &str = "Touchpad and keyboard";
        if self.x11 {
            Way::found(NAME, "X11's XTEST")
        } else if self.virtual_input {
            Way::found(NAME, format!("{} virtual pointer and keyboard", self.compositor(true)))
        } else if self.remote_desktop_portal {
            Way::found(NAME, "The Remote Desktop portal, which asks once")
        } else {
            Way::missing(NAME, "This desktop lets no app move the pointer or type: no Remote Desktop portal and no virtual pointer")
        }
    }

    fn clipboard(&self) -> Way {
        const NAME: &str = "Clipboard";
        if self.x11 {
            Way::found(NAME, "X11's selections")
        } else if self.gnome {
            if self.extension { Way::found(NAME, "Vyoma's GNOME extension") } else { Way::missing(NAME, NEEDS_EXTENSION) }
        } else if DATA_CONTROL.iter().any(|p| self.offers(p)) {
            Way::found(NAME, "Wayland's data-control, through wl-clipboard")
        } else {
            Way::missing(NAME, "This compositor shares its clipboard with no app (no data-control protocol)")
        }
    }

    fn screen(&self) -> Way {
        const NAME: &str = "Live screen";
        if self.x11 {
            return Way::found(NAME, "X11's picture of the main screen, touches through XTEST");
        }
        if !self.screencast_portal {
            return Way::missing(NAME, "No ScreenCast portal here (xdg-desktop-portal and this desktop's backend)");
        }
        let touch = if self.remote_desktop_portal {
            "touches through its Remote Desktop".to_owned()
        } else if self.virtual_input {
            format!("touches through {} virtual pointer", self.compositor(false))
        } else {
            "to watch only (no way to touch)".to_owned()
        };
        Way::found(NAME, format!("The ScreenCast portal, {touch}"))
    }

    fn screenshots(&self) -> Way {
        const NAME: &str = "Screenshots";
        match self.shooter {
            Some(Shooter::X11) => Way::found(NAME, "X11's picture (GStreamer)"),
            Some(Shooter::Gnome) if self.extension => Way::found(NAME, "Vyoma's GNOME extension"),
            Some(Shooter::Gnome) => Way::missing(NAME, NEEDS_EXTENSION),
            Some(Shooter::Spectacle) => Way::found(NAME, "Spectacle"),
            Some(Shooter::Grim) => Way::found(NAME, "grim"),
            Some(Shooter::Nothing) | None => Way::missing(NAME, "Needs Spectacle (Plasma) or grim (Hyprland, Sway and the like)"),
        }
    }

    fn dot(&self) -> Way {
        const NAME: &str = "Pointer dot";
        if self.x11 {
            Way::found(NAME, "An X11 overlay window")
        } else if self.gnome {
            if self.extension { Way::found(NAME, "Vyoma's GNOME extension") } else { Way::missing(NAME, NEEDS_EXTENSION) }
        } else if self.offers(LAYER_SHELL) {
            Way::found(NAME, "A layer-shell overlay")
        } else {
            Way::missing(NAME, "This compositor has no layer-shell for an overlay")
        }
    }

    fn desktops(&self) -> Way {
        const NAME: &str = "Desktops";
        if self.x11 {
            Way::found(NAME, "The window manager's desktops (EWMH)")
        } else if self.hyprland {
            Way::found(NAME, "Hyprland's workspaces, over its socket")
        } else if self.gnome {
            if self.extension { Way::found(NAME, "Vyoma's GNOME extension") } else { Way::missing(NAME, NEEDS_EXTENSION) }
        } else if self.kwin {
            Way::found(NAME, "KWin's shortcuts")
        } else {
            Way::missing(NAME, "Not on this compositor yet")
        }
    }
}

/// Whether the portal [iface] is there (its `version`).
async fn portal(conn: &Connection, iface: &str) -> bool {
    let args = (iface, "version");
    let ask = conn.call_method(Some("org.freedesktop.portal.Desktop"), "/org/freedesktop/portal/desktop", Some("org.freedesktop.DBus.Properties"), "Get", &args);
    matches!(tokio::time::timeout(PORTAL_WAIT, ask).await, Ok(Ok(_)))
}

async fn has_owner(conn: &Connection, name: &str) -> bool {
    match conn.call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "NameHasOwner", &(name,)).await {
        Ok(reply) => reply.body().deserialize::<bool>().unwrap_or(false),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(s: &Session) -> Vec<(&'static str, bool, String)> {
        s.ways().into_iter().map(|w| (w.name, w.found, w.detail)).collect()
    }

    #[test]
    fn hyprland_without_a_remote_desktop_portal() {
        let s = Session {
            hyprland: true,
            virtual_input: true,
            screencast_portal: true,
            globals: vec!["zwlr_data_control_manager_v1".into(), LAYER_SHELL.into()],
            shooter: Some(Shooter::Grim),
            ..Default::default()
        };
        let w = names(&s);
        assert_eq!(w[0].2, "Hyprland's virtual pointer and keyboard");
        assert_eq!(w[1].2, "Wayland's data-control, through wl-clipboard");
        assert_eq!(w[2].2, "The ScreenCast portal, touches through Hyprland's virtual pointer");
        assert!(w.iter().all(|w| w.1), "everything has a way: {w:?}");
    }

    #[test]
    fn gnome_without_the_extension_says_so() {
        let s = Session { gnome: true, remote_desktop_portal: true, screencast_portal: true, shooter: Some(Shooter::Gnome), ..Default::default() };
        let w = names(&s);
        assert!(w[0].1 && w[2].1, "input and the screen through the portals");
        for i in [1, 3, 4, 5] {
            assert_eq!((w[i].1, w[i].2.as_str()), (false, NEEDS_EXTENSION), "{}", w[i].0);
        }
    }

    /// What this session offers, printed (only the ways: no names, nothing personal).
    #[tokio::test]
    #[ignore]
    async fn reads_this_session() {
        for w in Session::look().await.ways() {
            println!("{} | {} | {}", w.name, if w.found { "found" } else { "missing" }, w.detail);
        }
    }

    #[test]
    fn x11_does_it_all_itself() {
        let s = Session { x11: true, shooter: Some(Shooter::X11), ..Default::default() };
        assert!(s.ways().iter().all(|w| w.found));
        assert!(!Session::default().ways().iter().any(|w| w.found), "a bare Wayland session with nothing offered");
    }
}
