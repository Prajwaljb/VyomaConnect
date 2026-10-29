//! What the companion borrows from the user's own KDE Connect daemon: which phones it trusts,
//! and its identity key, which signs the proof that this companion belongs to that PC.
//! Read-only: nothing here ever writes to kdeconnectd's config.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use base64::Engine;
use p256::ecdsa::{Signature, SigningKey, signature::Signer};
use p256::pkcs8::DecodePrivateKey;
use rustls_pki_types::CertificateDer;
use rustls_pki_types::pem::PemObject;
use sha2::{Digest, Sha256};

pub struct KdeHost {
    pub device_id: String,
    pub name: String,
    signing: SigningKey,
    config: PathBuf,
}

fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"))
        .join("kdeconnect")
}

impl KdeHost {
    pub fn load() -> Result<Self> {
        let config = config_dir();
        let cert = std::fs::read(config.join("certificate.pem")).context("reading kdeconnect certificate")?;
        let der = CertificateDer::from_pem_slice(&cert).map_err(|e| anyhow!("kdeconnect certificate: {e:?}"))?;
        let device_id = common_name(&der).context("kdeconnect certificate has no CN")?;
        let key_pem = std::fs::read_to_string(config.join("privateKey.pem")).context("reading kdeconnect key")?;
        // kdeconnectd writes SEC1 ("EC PRIVATE KEY"); accept PKCS#8 too.
        let secret = p256::SecretKey::from_sec1_pem(&key_pem)
            .or_else(|_| p256::SecretKey::from_pkcs8_pem(&key_pem))
            .map_err(|e| anyhow!("kdeconnect key is not P-256: {e}"))?;
        let name = ini_value(&std::fs::read_to_string(config.join("config")).unwrap_or_default(), "General", "name")
            .unwrap_or_else(|| "PC".into());
        Ok(Self { device_id, name, signing: SigningKey::from(secret), config })
    }

    /// Phones kdeconnectd is paired with, by device id, with the certificate it pinned (DER).
    pub fn trusted_phones(&self) -> HashMap<String, Vec<u8>> {
        let text = std::fs::read_to_string(self.config.join("trusted_devices")).unwrap_or_default();
        let mut out = HashMap::new();
        let mut section: Option<String> = None;
        let mut is_phone = false;
        let mut cert: Option<Vec<u8>> = None;
        let mut flush = |section: &Option<String>, is_phone: bool, cert: &mut Option<Vec<u8>>| {
            if let (Some(id), true, Some(der)) = (section, is_phone, cert.take()) {
                out.insert(id.clone(), der);
            }
        };
        for line in text.lines() {
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                flush(&section, is_phone, &mut cert);
                section = Some(name.to_string());
                is_phone = false;
            } else if let Some(v) = line.strip_prefix("certificate=") {
                let pem = v.trim_matches('"').replace("\\n", "\n");
                cert = CertificateDer::from_pem_slice(pem.as_bytes()).ok().map(|c| c.to_vec());
            } else if let Some(v) = line.strip_prefix("type=") {
                is_phone = matches!(v, "phone" | "tablet");
            }
        }
        flush(&section, is_phone, &mut cert);
        out
    }

    /// Signs the companion pairing proof (see [proof_message]); base64 of a DER ECDSA signature.
    pub fn sign(&self, message: &str) -> String {
        let sig: Signature = self.signing.sign(message.as_bytes());
        base64::engine::general_purpose::STANDARD.encode(sig.to_der().as_bytes())
    }
}

/// Binds the proof to this companion's certificate and to one phone, so it can't be replayed.
pub fn proof_message(companion_id: &str, companion_cert_der: &[u8], phone_id: &str, timestamp: i64) -> String {
    let fingerprint = hex::encode(Sha256::digest(companion_cert_der));
    format!("vyoma-companion-v1\n{companion_id}\n{fingerprint}\n{phone_id}\n{timestamp}")
}

/// The first commonName in a certificate. A tiny DER scan is enough for KDE Connect's
/// self-signed certificates and saves pulling in an X.509 parser.
fn common_name(der: &[u8]) -> Option<String> {
    const CN_OID: [u8; 5] = [0x06, 0x03, 0x55, 0x04, 0x03];
    let at = der.windows(CN_OID.len()).position(|w| w == CN_OID)? + CN_OID.len();
    let (tag, len) = (*der.get(at)?, *der.get(at + 1)? as usize);
    if !matches!(tag, 0x0c | 0x13) || len >= 0x80 {
        return None;
    }
    String::from_utf8(der.get(at + 2..at + 2 + len)?.to_vec()).ok()
}

fn ini_value(text: &str, section: &str, key: &str) -> Option<String> {
    let mut inside = false;
    for line in text.lines() {
        if line.starts_with('[') {
            inside = line == format!("[{section}]");
        } else if inside && let Some(v) = line.strip_prefix(&format!("{key}=")) {
            return Some(v.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_ini() {
        let t = "[General]\nkeyAlgorithm=EC\nname=vyoma\n[Other]\nname=x\n";
        assert_eq!(ini_value(t, "General", "name").as_deref(), Some("vyoma"));
    }
}
