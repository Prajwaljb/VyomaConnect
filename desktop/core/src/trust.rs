//! The devices this PC has paired with and the certificate each one showed then: the only one it
//! will talk to under that id. One JSON file, readable by the user only, replaced whole on change.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::identity::write_private;

#[derive(Clone, Debug, PartialEq)]
pub struct Trusted {
    pub name: String,
    /// `deviceType` from its identity: phone, tablet, desktop...
    pub kind: String,
    pub cert: Vec<u8>,
}

pub struct TrustStore {
    path: PathBuf,
    devices: HashMap<String, Trusted>,
}

impl TrustStore {
    /// `trusted.json` in [dir], empty if there's none yet. One that can't be read is an error, not
    /// an empty list: writing over it would forget every pairing.
    pub fn open(dir: &Path) -> Result<Self> {
        let path = dir.join("trusted.json");
        let devices = match std::fs::read(&path) {
            Ok(bytes) => parse(&serde_json::from_slice(&bytes).with_context(|| format!("reading {}", path.display()))?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        Ok(Self { path, devices })
    }

    pub fn get(&self, id: &str) -> Option<&Trusted> {
        self.devices.get(id)
    }

    pub fn ids(&self) -> impl Iterator<Item = &String> {
        self.devices.keys()
    }

    /// Id to certificate, for [crate::link::connect_trusted].
    pub fn certs(&self) -> HashMap<String, Vec<u8>> {
        self.devices.iter().map(|(id, d)| (id.clone(), d.cert.clone())).collect()
    }

    pub fn put(&mut self, id: &str, device: Trusted) -> Result<()> {
        if self.devices.get(id) == Some(&device) {
            return Ok(());
        }
        self.devices.insert(id.to_owned(), device);
        self.save()
    }

    pub fn remove(&mut self, id: &str) -> Result<()> {
        if self.devices.remove(id).is_some() {
            self.save()?;
        }
        Ok(())
    }

    fn save(&self) -> Result<()> {
        let devices: serde_json::Map<String, Value> = self
            .devices
            .iter()
            .map(|(id, d)| (id.clone(), json!({ "name": d.name, "type": d.kind, "cert": hex(&d.cert) })))
            .collect();
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // Written beside and renamed over, so a crash midway leaves the old list, not half of one.
        let part = self.path.with_extension("json.part");
        write_private(&part, serde_json::to_string_pretty(&json!({ "devices": devices }))?.as_bytes())?;
        std::fs::rename(&part, &self.path)?;
        Ok(())
    }
}

fn parse(v: &Value) -> HashMap<String, Trusted> {
    let Some(devices) = v["devices"].as_object() else { return HashMap::new() };
    devices
        .iter()
        .filter_map(|(id, d)| {
            let cert = unhex(d["cert"].as_str()?)?;
            Some((id.clone(), Trusted { name: d["name"].as_str().unwrap_or("").to_owned(), kind: d["type"].as_str().unwrap_or("").to_owned(), cert }))
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn keeps_pairings_across_opens_privately() {
        let dir = std::env::temp_dir().join(format!("vyoma-trust-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut store = TrustStore::open(&dir).unwrap();
        assert!(store.get("phone_one_0123456").is_none());
        let phone = Trusted { name: "Phone".into(), kind: "phone".into(), cert: vec![0, 1, 0xfe, 0xff] };
        store.put("phone_one_0123456", phone.clone()).unwrap();
        store.put("phone_two_0123456", Trusted { cert: vec![7], ..phone.clone() }).unwrap();
        store.remove("phone_two_0123456").unwrap();

        let again = TrustStore::open(&dir).unwrap();
        assert_eq!(again.get("phone_one_0123456"), Some(&phone));
        assert!(again.get("phone_two_0123456").is_none());
        let mode = std::fs::metadata(dir.join("trusted.json")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);

        std::fs::write(dir.join("trusted.json"), "{oops").unwrap();
        assert!(TrustStore::open(&dir).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn hex_round_trips() {
        assert_eq!(unhex(&hex(&[0, 9, 10, 255])), Some(vec![0, 9, 10, 255]));
        assert_eq!(unhex("abc"), None);
        assert_eq!(unhex("zz"), None);
    }
}
