//! The phone's run command (`kdeconnect.runcommand.request`), answered once for every PC side:
//! the list, the phone's "add command" (which opens the PC's own editor), and running one. The
//! list itself lives in `vyoma_core::commands`; opening the editor and saying a command that
//! couldn't start are each side's.

use serde_json::Value;

pub use vyoma_core::commands::{Commands, RUNCOMMAND, RUNCOMMAND_REQUEST};

/// What a request came to.
#[derive(Debug, Default, PartialEq)]
pub struct Answer {
    /// The list, for the phone that asked.
    pub list: Option<Value>,
    /// The phone asked to add a command: open the PC's Commands page.
    pub setup: bool,
    /// The command started, by name.
    pub ran: Option<String>,
    /// A key that names no command.
    pub unknown: Option<String>,
}

impl Answer {
    /// The activity log's line for the command that ran.
    pub fn activity(&self) -> Option<String> {
        self.ran.as_ref().map(|name| format!("Ran \u{201c}{name}\u{201d}"))
    }
}

/// Answers a request's [body]; [failed] hears of a command whose program wasn't there or couldn't
/// run (the phone can't be told: stock runcommand has no reply).
pub fn on_request(run: &Commands, body: &Value, failed: impl FnOnce(String, &'static str) + Send + 'static) -> Answer {
    let mut a = Answer { list: (body["requestCommandList"].as_bool() == Some(true)).then(|| run.list_packet()), setup: body["setup"].as_bool() == Some(true), ..Default::default() };
    if let Some(key) = body["key"].as_str() {
        match run.run(key, failed) {
            Some(name) => a.ran = Some(name),
            None => a.unknown = Some(key.to_owned()),
        }
    }
    a
}

/// The notice for a command that couldn't start: title and body.
pub fn failure(name: &str, why: &str) -> (String, String) {
    (format!("\u{201c}{name}\u{201d} didn't run"), format!("{why}. Check it in Vyoma's Commands."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lists_opens_and_runs() {
        let dir = std::env::temp_dir().join(format!("vyoma-engine-commands-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let run = Commands::new(&dir);
        let key = run.add("Nothing", "true").unwrap();

        let a = on_request(&run, &json!({ "requestCommandList": true, "setup": true }), |_, _| {});
        assert!(a.setup && a.ran.is_none());
        assert!(a.list.unwrap()["body"]["commandList"].as_str().unwrap().contains("Nothing"));

        let a = on_request(&run, &json!({ "key": key }), |_, _| panic!("it runs"));
        assert_eq!((a.activity().as_deref(), a.list, a.setup), (Some("Ran \u{201c}Nothing\u{201d}"), None, false));
        assert_eq!(on_request(&run, &json!({ "key": "gone" }), |_, _| {}).unknown.as_deref(), Some("gone"));
        assert_eq!(failure("Backup", "It isn't installed").0, "\u{201c}Backup\u{201d} didn't run");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
