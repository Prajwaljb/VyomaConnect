//! Commands the user set up on this PC for the phone to run, as kdeconnectd's runcommand plugin:
//! the phone gets their names (`commandList`, a JSON object in a string, as stock) and sends back
//! only a key, never a command line.
//!
//! Kept in `commands.json` in the daemon's folder:
//! `{"<key>": {"name": …, "command": …, "fingerprint": bool, "everyTime": bool}}`.
//! `fingerprint` (Vyoma's own, ignored by stock phones) has Vyoma on the phone ask for the
//! owner's fingerprint before running it; one check normally covers a while of using the app.
//! `everyTime` (only with `fingerprint`) has it ask on every run, and keeps it out of the phone's
//! unattended rules. Vyoma's window adds and edits them; a phone's
//! "add command" (`setup`) opens it there.
//!
//! Every desktop shares this (Linux and the Mac); each says in its own way when a command
//! couldn't start ([Commands::run]'s `failed`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result};
use serde_json::{Value, json};
use crate::link::packet;

pub const RUNCOMMAND: &str = "kdeconnect.runcommand";
pub const RUNCOMMAND_REQUEST: &str = "kdeconnect.runcommand.request";

#[derive(Clone)]
pub struct Commands {
    file: PathBuf,
}

impl Commands {
    pub fn new(dir: &Path) -> Self {
        Self { file: dir.join("commands.json") }
    }

    /// Read each time: the user may edit the file by hand.
    pub fn all(&self) -> BTreeMap<String, Value> {
        match std::fs::read_to_string(&self.file) {
            Ok(text) => serde_json::from_str::<BTreeMap<String, Value>>(&text)
                .map_err(|e| tracing::warn!("{} isn't valid: {e}", self.file.display()))
                .unwrap_or_default()
                .into_iter()
                .filter(|(_, c)| c["name"].is_string() && c["command"].is_string())
                .collect(),
            Err(_) => BTreeMap::new(),
        }
    }

    /// For changing the list: a file the user broke by hand is refused rather than overwritten, as
    /// saving over it would lose every command in it.
    fn for_change(&self) -> Result<BTreeMap<String, Value>> {
        match std::fs::read_to_string(&self.file) {
            Ok(text) if !text.trim().is_empty() => {
                serde_json::from_str::<BTreeMap<String, Value>>(&text).map_err(|e| {
                    anyhow::anyhow!("{} has a mistake on line {}, so saving would lose your commands. Fix it, or move it away to start over", self.file.display(), e.line())
                })?;
                Ok(self.all())
            }
            _ => Ok(BTreeMap::new()),
        }
    }

    pub fn list_packet(&self) -> Value {
        let list = Value::Object(self.all().into_iter().collect());
        packet(RUNCOMMAND, json!({ "commandList": list.to_string(), "canAddCommand": true }))
    }

    /// Adds a command; its key.
    pub fn add(&self, name: &str, command: &str) -> Result<String> {
        self.save_one("", name, command, false, false)
    }

    /// Changes the command under [key], or adds one when [key] is empty; its key.
    pub fn save_one(&self, key: &str, name: &str, command: &str, fingerprint: bool, every_time: bool) -> Result<String> {
        anyhow::ensure!(!name.trim().is_empty() && !command.trim().is_empty(), "A command needs a name and a command line");
        let mut all = self.for_change()?;
        let key = if key.is_empty() {
            loop {
                let k = format!("{:016x}", unique_u64());
                if !all.contains_key(&k) {
                    break k;
                }
            }
        } else {
            anyhow::ensure!(all.contains_key(key), "That command is gone");
            key.to_owned()
        };
        all.insert(key.clone(), json!({ "name": name.trim(), "command": command.trim(), "fingerprint": fingerprint, "everyTime": fingerprint && every_time }));
        self.save(&all)?;
        Ok(key)
    }

    pub fn remove(&self, key: &str) -> Result<bool> {
        let mut all = self.for_change()?;
        let gone = all.remove(key).is_some();
        if gone {
            self.save(&all)?;
        }
        Ok(gone)
    }

    fn save(&self, all: &BTreeMap<String, Value>) -> Result<()> {
        let part = self.file.with_extension("json.part");
        std::fs::write(&part, serde_json::to_vec_pretty(all)?).with_context(|| format!("can't write {}", part.display()))?;
        std::fs::rename(&part, &self.file)?;
        Ok(())
    }

    /// Runs the command stored under [key], if there is one; its name. If its program isn't
    /// there or can't be run, [failed] gets its name and why, in words for the person.
    pub fn run(&self, key: &str, failed: impl FnOnce(String, &'static str) + Send + 'static) -> Option<String> {
        let all = self.all();
        let c = all.get(key)?;
        let (name, command) = (c["name"].as_str()?.to_owned(), c["command"].as_str()?);
        match std::process::Command::new("sh").args(["-c", command]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn() {
            // Reaped on its own thread; it may run for as long as it likes.
            Ok(child) => {
                let name = name.clone();
                drop(std::thread::spawn(move || {
                    let Ok(out) = child.wait_with_output() else { return };
                    // 127: no such program, 126: it can't be run. The phone can't be told (stock
                    // runcommand has no reply), so the PC says so. Other exit codes are left alone:
                    // plenty of commands use them for ordinary outcomes.
                    if let Some(code @ (126 | 127)) = out.status.code() {
                        let said = String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or_default().trim().to_owned();
                        tracing::warn!("{name} couldn't run ({code}): {said}");
                        failed(name, if code == 127 { "Its program isn't on this computer" } else { "Its program can't be run (is it executable?)" });
                    }
                }));
            }
            Err(e) => tracing::warn!("can't run {name}: {e}"),
        }
        Some(name)
    }
}

/// A fresh key; it needn't be secret, as the phone only names commands we listed.
fn unique_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_broken_file_is_kept_not_overwritten() {
        let dir = std::env::temp_dir().join(format!("vyoma-commands-broken-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let broken = "{\"a\": {\"name\": \"Backup\", \"command\": \"backup\"},\n}";
        std::fs::write(dir.join("commands.json"), broken).unwrap();
        let c = Commands::new(&dir);
        let e = c.add("New", "true").unwrap_err().to_string();
        assert!(e.contains("line 2"), "{e}");
        assert!(c.remove("a").is_err());
        assert_eq!(std::fs::read_to_string(dir.join("commands.json")).unwrap(), broken);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_stored_commands_run() {
        let dir = std::env::temp_dir().join(format!("vyoma-commands-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let c = Commands::new(&dir);
        assert!(c.run("nope", |_, _| {}).is_none());
        let flag = dir.join("ran");
        let key = c.add("Touch", &format!("touch '{}'", flag.display())).unwrap();
        let list: Value = serde_json::from_str(c.list_packet()["body"]["commandList"].as_str().unwrap()).unwrap();
        assert_eq!(list[&key]["name"], "Touch");
        assert_eq!(c.save_one(&key, "Touch it", &format!("touch '{}'", flag.display()), true, true).unwrap(), key);
        let list: Value = serde_json::from_str(c.list_packet()["body"]["commandList"].as_str().unwrap()).unwrap();
        assert_eq!((list[&key]["name"].as_str(), list[&key]["fingerprint"].as_bool(), list[&key]["everyTime"].as_bool()), (Some("Touch it"), Some(true), Some(true)));
        // Every time means nothing without the fingerprint.
        c.save_one(&key, "Touch it", &format!("touch '{}'", flag.display()), false, true).unwrap();
        let list: Value = serde_json::from_str(c.list_packet()["body"]["commandList"].as_str().unwrap()).unwrap();
        assert_eq!(list[&key]["everyTime"].as_bool(), Some(false));
        assert!(c.save_one("", " ", "true", false, false).is_err());
        assert_eq!(c.run(&key, |_, _| {}).as_deref(), Some("Touch it"));
        for _ in 0..50 {
            if flag.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(flag.exists());
        assert!(c.remove(&key).unwrap());
        assert!(c.run(&key, |_, _| {}).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
