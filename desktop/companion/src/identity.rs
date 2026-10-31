//! The companion's own KDE Connect identity: a device id and a self-signed certificate,
//! created once and kept in ~/.config/vyoma-companion (key readable by the user only).

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use rcgen::{CertificateParams, DnType, KeyPair};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};

pub struct Identity {
    pub device_id: String,
    pub cert: CertificateDer<'static>,
    pub key: PrivateKeyDer<'static>,
}

pub(crate) fn dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"))
        .join("vyoma-companion")
}

impl Identity {
    pub fn load_or_create() -> Result<Self> {
        let dir = dir();
        let (id_path, cert_path, key_path) = (dir.join("device_id"), dir.join("certificate.pem"), dir.join("privateKey.pem"));
        if id_path.exists() && cert_path.exists() && key_path.exists() {
            let device_id = std::fs::read_to_string(&id_path)?.trim().to_string();
            let cert = CertificateDer::from_pem_file(&cert_path).map_err(|e| anyhow!("companion certificate: {e:?}"))?;
            let key = PrivateKeyDer::from_pem_file(&key_path).map_err(|e| anyhow!("companion key: {e:?}"))?;
            return Ok(Self { device_id, cert, key });
        }

        std::fs::create_dir_all(&dir).context("creating companion config dir")?;
        // Same shape as KDE Connect's own ids and certificates, so every peer accepts it.
        let device_id = uuid::Uuid::new_v4().to_string().replace('-', "_");
        let key_pair = KeyPair::generate()?;
        let mut params = CertificateParams::new(Vec::<String>::new())?;
        params.distinguished_name.push(DnType::CommonName, device_id.as_str());
        params.distinguished_name.push(DnType::OrganizationName, "KDE");
        params.distinguished_name.push(DnType::OrganizationalUnitName, "Kde connect");
        params.not_before = rcgen::date_time_ymd(2024, 1, 1);
        params.not_after = rcgen::date_time_ymd(2054, 1, 1);
        let cert = params.self_signed(&key_pair)?;

        std::fs::write(&cert_path, cert.pem())?;
        write_private(&key_path, key_pair.serialize_pem().as_bytes())?;
        std::fs::write(&id_path, &device_id)?;
        tracing::info!("created companion identity {device_id}");
        Self::load_or_create()
    }
}

fn write_private(path: &PathBuf, data: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    f.write_all(data)?;
    Ok(())
}
