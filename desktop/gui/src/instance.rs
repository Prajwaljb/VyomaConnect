//! One window at a time: the first takes `dev.vyoma.Window` on the session bus, and a second
//! start (or the daemon, when a phone asks for the command editor) has it show a page instead.

use std::sync::{Mutex, OnceLock};

use anyhow::Result;
use slint::ComponentHandle;
use zbus::fdo::{RequestNameFlags, RequestNameReply};

use crate::MainWindow;

pub const NAME: &str = "dev.vyoma.Window";
const PATH: &str = "/dev/vyoma/Window";
const INTERFACE: &str = "dev.vyoma.Window1";

/// Kept for as long as the window runs: dropping it gives the name up.
static HELD: OnceLock<zbus::Connection> = OnceLock::new();

struct Window(Mutex<slint::Weak<MainWindow>>);

#[zbus::interface(name = "dev.vyoma.Window1")]
impl Window {
    /// Brings the window up on [page]: "commands", "this-pc", "activity", a phone's id, or "" as
    /// it was.
    fn show(&self, page: String) {
        let ui = self.0.lock().unwrap().clone();
        let _ = ui.upgrade_in_event_loop(move |win| {
            crate::open_page(&win, &page);
            let _ = win.show();
            win.window().set_minimized(false);
        });
    }
}

/// Takes the name, or, if another window has it, asks that one to show [page]: false then, and
/// this one should end.
pub fn claim(ui: &MainWindow, page: &str) -> Result<bool> {
    let weak = ui.as_weak();
    crate::daemon::runtime().block_on(async {
        let conn = zbus::Connection::session().await?;
        conn.object_server().at(PATH, Window(Mutex::new(weak))).await?;
        match conn.request_name_with_flags(NAME, RequestNameFlags::DO_NOT_QUEUE).await {
            Ok(RequestNameReply::PrimaryOwner) | Ok(RequestNameReply::AlreadyOwner) => {
                let _ = HELD.set(conn);
                Ok(true)
            }
            _ => {
                conn.call_method(Some(NAME), PATH, Some(INTERFACE), "Show", &(page,)).await?;
                Ok(false)
            }
        }
    })
}
