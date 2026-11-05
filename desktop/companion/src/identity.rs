//! The companion's own KDE Connect identity: a device id and a self-signed certificate,
//! created once and kept in ~/.config/vyoma-companion (key readable by the user only).

use std::path::{Path, PathBuf};

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
        lock_down(&dir);
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

/// Nothing in here is anyone else's business: the portal tokens let the holder skip Plasma's
/// "share your screen?" question. Older installs left them readable, so this also corrects those.
fn lock_down(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let private = |path: &Path, mode| {
        if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)) {
            tracing::warn!("couldn't make {} private: {e}", path.display());
        }
    };
    let Ok(files) = std::fs::read_dir(dir) else { return };
    private(dir, 0o700);
    for file in files.flatten().filter(|f| f.file_type().is_ok_and(|t| t.is_file())) {
        private(&file.path(), 0o600);
    }
}

pub(crate) fn write_private(path: &PathBuf, data: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    f.write_all(data)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn the_config_folder_ends_up_private() {
        let dir = std::env::temp_dir().join(format!("vyoma-identity-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let token = dir.join("screencast-token");
        std::fs::write(&token, "t").unwrap();
        std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        lock_down(&dir);
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(std::fs::metadata(&token).unwrap().permissions().mode() & 0o777, 0o600);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
