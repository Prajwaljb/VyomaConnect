//! Desktop notifications over the freedesktop interface, which KDE, GNOME, Cinnamon and COSMIC all
//! serve, so one implementation does for every Linux desktop. Unlike `notify-send` it can carry
//! buttons and tell us which one was pressed: a pairing prompt is answered from the notification.

use std::collections::HashMap;

use anyhow::Result;
use futures_util::StreamExt;
use zbus::Variant;
use zbus::{Connection, MatchRule, MessageStream};

const BUS: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";

/// A notification's button, by the key it was given.
pub struct Pressed {
    pub notification: u32,
    pub action: String,
}

#[derive(Clone)]
pub struct Notifier {
    conn: Connection,
}

pub struct Notice<'a> {
    pub icon: &'a str,
    pub title: &'a str,
    pub body: &'a str,
    /// Key and label of each button.
    pub actions: &'a [(&'a str, &'a str)],
    /// Stays until answered or taken down (a pairing prompt), instead of timing out on its own.
    pub sticky: bool,
    /// The one it replaces, if it's an update.
    pub replaces: Option<u32>,
}

impl Notifier {
    pub fn new(conn: Connection) -> Self {
        Self { conn }
    }

    /// Shows [notice]; its id, to take it down or match its buttons by.
    pub async fn show(&self, notice: Notice<'_>) -> Result<u32> {
        let actions: Vec<&str> = notice.actions.iter().flat_map(|(key, label)| [*key, *label]).collect();
        let hints: HashMap<&str, Variant> = HashMap::from([("desktop-entry", Variant::from("vyoma-desktop"))]);
        // 0: never times out on its own (a request ends after 30 s anyway, and is taken down then).
        let timeout: i32 = if notice.sticky { 0 } else { -1 };
        let reply = self
            .conn
            .call_method(
                Some(BUS),
                PATH,
                Some(BUS),
                "Notify",
                &("Vyoma", notice.replaces.unwrap_or(0), notice.icon, notice.title, escape(notice.body), actions, hints, timeout),
            )
            .await?;
        Ok(reply.body().deserialize::<u32>()?)
    }

    pub async fn close(&self, id: u32) {
        if let Err(e) = self.conn.call_method(Some(BUS), PATH, Some(BUS), "CloseNotification", &(id,)).await {
            tracing::debug!("closing notification {id}: {e}");
        }
    }

    /// Every button pressed on any of our notifications, until the bus goes.
    pub async fn presses(&self, out: tokio::sync::mpsc::UnboundedSender<Pressed>) -> Result<()> {
        let rule = MatchRule::builder().msg_type(zbus::message::Type::Signal).interface(BUS).member("ActionInvoked").path(PATH).build()?;
        let mut stream = MessageStream::for_match_rule(rule, &self.conn, None).await?;
        while let Some(Ok(msg)) = stream.next().await {
            if let Ok((notification, action)) = msg.body().deserialize::<(u32, String)>()
                && out.send(Pressed { notification, action }).is_err()
            {
                break;
            }
        }
        Ok(())
    }
}

/// Bodies are read as markup, and names come from the phone.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    #[test]
    fn names_cant_be_markup() {
        assert_eq!(super::escape("Ann & <b>phone</b>"), "Ann &amp; &lt;b&gt;phone&lt;/b&gt;");
    }
}
