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

/// What happened to one of our notifications.
pub enum Heard {
    /// A button, by the key it was given.
    Pressed { notification: u32, action: String },
    /// Text typed into its inline reply (KDE).
    Replied { notification: u32, text: String },
    /// Gone; [by_user]: dismissed, rather than timed out or taken down by us.
    Closed { notification: u32, by_user: bool },
}

/// The action key of an inline reply, as KDE's notification server knows it.
pub const INLINE_REPLY: &str = "inline-reply";

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
        self.show_with(notice, None, None).await
    }

    /// [show] with a picture (a file) and, where the desktop has it, a reply field showing
    /// [reply] as its placeholder; what's typed comes back as [Heard::Replied].
    pub async fn show_with(&self, notice: Notice<'_>, image: Option<&std::path::Path>, reply: Option<&str>) -> Result<u32> {
        let mut actions: Vec<&str> = notice.actions.iter().flat_map(|(key, label)| [*key, *label]).collect();
        let mut hints: HashMap<&str, Variant> = HashMap::from([("desktop-entry", Variant::from("vyoma"))]);
        if let Some(path) = image.and_then(|p| p.to_str()) {
            hints.insert("image-path", Variant::from(path.to_owned()));
        }
        if let Some(placeholder) = reply {
            actions.extend([INLINE_REPLY, "Reply"]);
            hints.insert("x-kde-reply-placeholder-text", Variant::from(placeholder.to_owned()));
        }
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

    /// Every button pressed, reply typed and notification closed, until the bus goes. The ids are
    /// the server's, so others' notifications come through too: callers match their own.
    pub async fn presses(&self, out: tokio::sync::mpsc::UnboundedSender<Heard>) -> Result<()> {
        let rule = MatchRule::builder().msg_type(zbus::message::Type::Signal).interface(BUS).path(PATH).build()?;
        let mut stream = MessageStream::for_match_rule(rule, &self.conn, None).await?;
        while let Some(Ok(msg)) = stream.next().await {
            let body = msg.body();
            let heard = match msg.header().member().map(|m| m.as_str()) {
                Some("ActionInvoked") => body.deserialize::<(u32, String)>().map(|(notification, action)| Heard::Pressed { notification, action }),
                Some("NotificationReplied") => body.deserialize::<(u32, String)>().map(|(notification, text)| Heard::Replied { notification, text }),
                // Reason 2: dismissed by the user.
                Some("NotificationClosed") => body.deserialize::<(u32, u32)>().map(|(notification, reason)| Heard::Closed { notification, by_user: reason == 2 }),
                _ => continue,
            };
            if let Ok(h) = heard
                && out.send(h).is_err()
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
