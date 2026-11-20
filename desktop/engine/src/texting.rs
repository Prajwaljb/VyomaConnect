//! The window's Messages page, the same on every side: who may be asked for texts, the phone's
//! texts lock, sending a text and calling from a conversation. The texts themselves are core's
//! [Store]; each answer also says what to send the phone (texts and contacts not loaded yet, the
//! unlock request). How the side reaches the phone, and how a call's sound is routed, is its own.

use serde_json::{Value, json};
use vyoma_core::messages::{self, Store};

/// What the Messages page needs to know of a phone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Texting {
    pub linked: bool,
    /// It shares its texts with this PC (stock sms).
    pub shares: bool,
    /// They're behind its texts lock.
    pub guarded: bool,
    /// It takes calls asked from here.
    pub dials: bool,
}

pub const NO_TEXTS: &str = "This phone doesn't share its texts";
pub const NOT_CONNECTED: &str = "The phone isn't connected";

/// Packets for the phone, sent with an answer.
pub type Ask = Vec<Value>;

impl Texting {
    fn shared(self) -> Result<bool, String> {
        if self.shares { Ok(self.guarded) } else { Err(NO_TEXTS.into()) }
    }

    /// The conversations, newest first; [calls_here]: a call's sound comes to this PC.
    pub fn conversations(self, store: &Store, id: &str, calls_here: bool) -> Result<(Value, Ask), String> {
        let (json, ask) = store.conversations(id, self.shared()?);
        let mut v: Value = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        // Whether the window offers to call: a phone that takes calls from here.
        (v["canCall"], v["callHere"]) = (json!(self.dials), json!(calls_here));
        Ok((v, ask))
    }

    /// The contacts with a number, by name.
    pub fn contacts(self, store: &Store, id: &str) -> Result<(String, Ask), String> {
        Ok(store.contacts(id, self.shared()?))
    }

    /// Conversations and contacts matching [query] (also loads more texts to search).
    pub fn search(self, store: &Store, id: &str, query: &str) -> Result<(String, Ask), String> {
        Ok(store.search(id, query, self.shared()?))
    }

    /// The conversation with [number], started here if there's none yet.
    pub fn thread_for(self, store: &Store, id: &str, number: &str) -> Result<i64, String> {
        store.thread_for(id, number, self.shared()?)
    }

    /// One conversation's texts, oldest first.
    pub fn thread(self, store: &Store, id: &str, thread: i64) -> (String, Ask) {
        store.thread(id, thread, self.guarded)
    }

    /// The packet that texts [body] to [thread]'s people from the phone.
    pub fn send(self, store: &Store, id: &str, thread: i64, body: &str) -> Result<Value, String> {
        if !(self.linked && self.shares) {
            return Err(NOT_CONNECTED.into());
        }
        store.send(id, thread, body, self.guarded)
    }

    /// The number to have the phone call: plain numbers only, while Messages is open.
    pub fn dial(self, store: &Store, id: &str, number: &str) -> Result<String, String> {
        if !self.linked {
            return Err(NOT_CONNECTED.into());
        }
        if !self.dials {
            return Err("Update Vyoma on the phone to call from here".into());
        }
        let number = messages::dialable(number).ok_or("That isn't a number to call")?;
        store.may_call(id, self.guarded)?;
        Ok(number)
    }

    /// The page was opened, a conversation picked or a text sent: a locked phone is asked to let
    /// the texts in (its owner confirms there), an open one stays open longer.
    pub fn unlock(self, store: &Store, id: &str) -> Result<Option<Value>, String> {
        if !self.guarded {
            return Ok(None);
        }
        if !self.linked {
            return Err(NOT_CONNECTED.into());
        }
        Ok(store.unlock(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_phone_that_shares_is_asked() {
        let store = Store::default();
        let none = Texting { linked: true, ..Default::default() };
        assert_eq!(none.conversations(&store, "p", false).unwrap_err(), NO_TEXTS);
        assert_eq!(none.send(&store, "p", 1, "hi").unwrap_err(), NOT_CONNECTED);
        assert_eq!(none.unlock(&store, "p"), Ok(None), "no lock, nothing to ask");

        let open = Texting { linked: true, shares: true, guarded: false, dials: false };
        let (v, _) = open.conversations(&store, "p", true).unwrap();
        assert_eq!((v["canCall"].as_bool(), v["callHere"].as_bool()), (Some(false), Some(true)));
        assert_eq!(open.dial(&store, "p", "5551234").unwrap_err(), "Update Vyoma on the phone to call from here");
        let dials = Texting { dials: true, ..open };
        assert_eq!(dials.dial(&store, "p", "*#06#").unwrap_err(), "That isn't a number to call");
        assert_eq!(Texting { linked: false, ..dials }.dial(&store, "p", "5551234").unwrap_err(), NOT_CONNECTED);
    }
}
