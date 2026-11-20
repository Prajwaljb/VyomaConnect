//! How this computer does each thing a phone asks of it (This PC's "How this PC does it"): one row
//! per capability, the way it found or why there's none. Each side fills it from the same checks
//! its code makes when choosing, so the report can't promise a way the code wouldn't take.

use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Way {
    /// The capability, as the window names it ("Touchpad and keyboard").
    pub name: &'static str,
    /// The way found, in words ("Hyprland's virtual pointer and keyboard"), or why there's none.
    pub detail: String,
    pub found: bool,
}

impl Way {
    pub fn found(name: &'static str, by: impl Into<String>) -> Way {
        Way { name, detail: by.into(), found: true }
    }

    pub fn missing(name: &'static str, why: impl Into<String>) -> Way {
        Way { name, detail: why.into(), found: false }
    }

    /// As the window reads a row (the features' shape: name, ready, detail).
    pub fn json(&self) -> Value {
        json!({ "name": self.name, "ready": self.found, "detail": self.detail })
    }
}

/// The report's `ways`.
pub fn json(ways: &[Way]) -> Value {
    Value::Array(ways.iter().map(Way::json).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_read_as_features_do() {
        let v = json(&[Way::found("Clipboard", "X11's selections"), Way::missing("Desktops", "Not on this compositor yet")]);
        assert_eq!(v[0], json!({ "name": "Clipboard", "ready": true, "detail": "X11's selections" }));
        assert_eq!(v[1]["ready"], false);
    }
}
