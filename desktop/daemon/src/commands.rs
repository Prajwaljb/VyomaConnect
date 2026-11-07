//! Commands the user set up on this PC for the phone to run, as kdeconnectd's runcommand plugin:
//! the phone gets their names (`commandList`, a JSON object in a string, as stock) and sends back
//! only a key, never a command line.
//!
//! Kept in `commands.json` in the daemon's folder: `{"<key>": {"name": …, "command": …}}`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result};
use serde_json::{Value, json};
use vyoma_core::link::packet;

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

    pub fn list_packet(&self) -> Value {
        let list = Value::Object(self.all().into_iter().collect());
        packet(RUNCOMMAND, json!({ "commandList": list.to_string(), "canAddCommand": false }))
    }

    /// Adds a command; its key.
    pub fn add(&self, name: &str, command: &str) -> Result<String> {
        let mut all = self.all();
        let key = loop {
            let k = format!("{:016x}", unique_u64());
            if !all.contains_key(&k) {
                break k;
            }
        };
        all.insert(key.clone(), json!({ "name": name, "command": command }));
        self.save(&all)?;
        Ok(key)
    }

    pub fn remove(&self, key: &str) -> Result<bool> {
        let mut all = self.all();
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

    /// Runs the command stored under [key], if there is one; its name.
    pub fn run(&self, key: &str) -> Option<String> {
        let all = self.all();
        let c = all.get(key)?;
        let (name, command) = (c["name"].as_str()?.to_owned(), c["command"].as_str()?);
        match std::process::Command::new("sh").args(["-c", command]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            // Reaped on its own thread; it may run for as long as it likes.
            Ok(mut child) => drop(std::thread::spawn(move || child.wait())),
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
    fn only_stored_commands_run() {
        let dir = std::env::temp_dir().join(format!("vyoma-commands-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let c = Commands::new(&dir);
        assert!(c.run("nope").is_none());
        let flag = dir.join("ran");
        let key = c.add("Touch", &format!("touch '{}'", flag.display())).unwrap();
        let list: Value = serde_json::from_str(c.list_packet()["body"]["commandList"].as_str().unwrap()).unwrap();
        assert_eq!(list[&key]["name"], "Touch");
        assert_eq!(c.run(&key).as_deref(), Some("Touch"));
        for _ in 0..50 {
            if flag.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(flag.exists());
        assert!(c.remove(&key).unwrap());
        assert!(c.run(&key).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
