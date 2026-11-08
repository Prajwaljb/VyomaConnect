//! The phone's files for GTK file managers (Thunar, Nautilus, Nemo, Caja...), through GVfs's SFTP
//! rather than the sshfs mount: they treat a FUSE mount as a local disk and wait on it in their
//! window, so a slow Wi-Fi moment froze them, while GVfs works in a process of its own and they
//! show it loading, as Dolphin does over the mount. KDE keeps the mount, which Dolphin lists.
//!
//! GVfs runs plain `ssh`, so each phone gets a host alias, a file of its own under our folder's
//! `ssh/`: the route (the link's address, or the cable's relay), our key, and the phone's host key
//! pinned as for sshfs. The user's `~/.ssh/config` gets one `Include` of those files and nothing
//! else; their own known hosts and keys are left alone. GVfs names the mount (in the sidebar) by
//! its host, so the alias is the phone's name.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tokio::process::Command;

/// How sshfs reached the phone, for the alias to go the same way.
pub struct Route {
    pub host: String,
    pub port: Option<u16>,
    /// ProxyCommand (over the cable).
    pub proxy: Option<String>,
}

/// Whether to browse through GVfs here: a GTK file manager, and GVfs that can do SFTP.
/// `VYOMA_GVFS=1` or `0` decides instead (a file manager on GIO that we don't know of).
pub fn wanted() -> bool {
    match std::env::var("VYOMA_GVFS").as_deref() {
        Ok("1") => vyoma_companion::setup::gvfs_sftp(),
        Ok("0") => false,
        _ => vyoma_companion::setup::gtk_file_manager() && vyoma_companion::setup::gvfs_sftp(),
    }
}

/// The alias for phone [id] called [name]: its name as a host name ("pixel-9.vyoma-desktop"),
/// which GVfs shows, unless another phone already has it. Lowercase: GIO and ssh both lowercase
/// the host, and ssh matches `Host` lines as written, so "Pixel" never matched (2026-11-08).
pub fn alias(dir: &Path, id: &str, name: &str) -> String {
    let mut clean: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    while clean.contains("--") {
        clean = clean.replace("--", "-");
    }
    let clean = clean.trim_matches('-');
    let base = format!("{}.vyoma-desktop", if clean.is_empty() { "phone" } else { clean });
    let taken = |a: &str| {
        std::fs::read_dir(dir.join("ssh")).into_iter().flatten().flatten().any(|e| {
            e.path() != conf_file(dir, id) && std::fs::read_to_string(e.path()).is_ok_and(|s| s.lines().next() == Some(&format!("Host {a}")))
        })
    };
    if !taken(&base) {
        return base;
    }
    let short: String = id.chars().filter(char::is_ascii_alphanumeric).take(6).map(|c| c.to_ascii_lowercase()).collect();
    format!("{}-{short}.vyoma-desktop", base.trim_end_matches(".vyoma-desktop"))
}

pub fn uri(alias: &str) -> String {
    format!("sftp://{alias}/")
}

fn conf_file(dir: &Path, id: &str) -> PathBuf {
    let clean: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    dir.join("ssh").join(format!("{clean}.conf"))
}

/// Writes phone [id]'s alias, and the `Include` into `~/.ssh/config` if it isn't there.
#[allow(clippy::too_many_arguments)]
pub fn set(dir: &Path, id: &str, alias: &str, route: &Route, user: &str, key: &Path, hosts: &Path, host_alias: &str) -> Result<()> {
    let text = conf_text(alias, route, user, key, hosts, host_alias)?;
    std::fs::create_dir_all(dir.join("ssh"))?;
    vyoma_core::identity::write_private(&conf_file(dir, id), text.as_bytes())?;
    include(&ssh_config(), &dir.join("ssh"))
}

/// Takes phone [id]'s alias away (the `Include` stays: it's of a folder, harmless when empty).
pub fn forget(dir: &Path, id: &str) {
    let _ = std::fs::remove_file(conf_file(dir, id));
}

/// At start: GVfs mounts of aliases an earlier run left, and the aliases.
pub async fn tidy(dir: &Path) {
    for entry in std::fs::read_dir(dir.join("ssh")).into_iter().flatten().flatten() {
        if let Some(alias) = std::fs::read_to_string(entry.path()).ok().and_then(|s| s.lines().next()?.strip_prefix("Host ").map(str::to_owned)) {
            unmount(&uri(&alias)).await;
        }
        let _ = std::fs::remove_file(entry.path());
    }
}

fn conf_text(alias: &str, route: &Route, user: &str, key: &Path, hosts: &Path, host_alias: &str) -> Result<String> {
    for s in [alias, &route.host, user, host_alias] {
        anyhow::ensure!(!s.is_empty() && !s.contains(|c: char| c.is_whitespace() || c == '"'), "can't put {s:?} in an SSH config");
    }
    let quoted = |p: &Path| -> Result<String> {
        let p = p.to_string_lossy();
        anyhow::ensure!(!p.contains(['"', '\n']), "can't put that path in an SSH config");
        Ok(format!("\"{p}\""))
    };
    let mut lines = vec![format!("Host {alias}"), format!("  HostName {}", route.host)];
    if let Some(port) = route.port {
        lines.push(format!("  Port {port}"));
    }
    if let Some(proxy) = &route.proxy {
        anyhow::ensure!(!proxy.contains('\n'), "bad ProxyCommand");
        lines.push(format!("  ProxyCommand {proxy}"));
    }
    lines.extend([
        format!("  User {user}"),
        format!("  IdentityFile {}", quoted(key)?),
        "  IdentitiesOnly yes".into(),
        // The phone's key from its certificate, as for sshfs; never the user's known hosts.
        format!("  HostKeyAlias {host_alias}"),
        format!("  UserKnownHostsFile {}", quoted(hosts)?),
        "  StrictHostKeyChecking yes".into(),
        // The phone's password changes each start and means nothing to the user: no prompt for it.
        "  PasswordAuthentication no".into(),
        "  KbdInteractiveAuthentication no".into(),
        "  ConnectTimeout 10".into(),
        "  ServerAliveInterval 5".into(),
        "  ServerAliveCountMax 2".into(),
    ]);
    Ok(lines.join("\n") + "\n")
}

fn ssh_config() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".ssh").join("config")
}

const INCLUDE_NOTE: &str = "# Vyoma Desktop: your phones' files in GTK file managers (one alias a phone, while browsable).";

fn include_line(folder: &Path) -> String {
    format!("Include \"{}/*.conf\"", folder.display())
}

/// Puts the `Include` of [folder] at the top of [config] (made if missing), keeping the rest as
/// it was. At the top, as ssh reads options first come, first kept, and a Host block in an
/// included file ends with it.
fn include(config: &Path, folder: &Path) -> Result<()> {
    let line = include_line(folder);
    let before = match std::fs::read_to_string(config) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).context("reading ~/.ssh/config"),
    };
    if before.lines().any(|l| l.trim() == line) {
        return Ok(());
    }
    let ssh_dir = config.parent().context("no ~/.ssh")?;
    if !ssh_dir.exists() {
        std::fs::create_dir_all(ssh_dir)?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(ssh_dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let after = format!("{INCLUDE_NOTE}\n{line}\n\n{before}");
    // Whole or not at all, and with the file's own permissions (ssh refuses a config others can write).
    let tmp = ssh_dir.join(".config.vyoma");
    vyoma_core::identity::write_private(&tmp, after.as_bytes())?;
    if let Ok(meta) = std::fs::metadata(config) {
        std::fs::set_permissions(&tmp, meta.permissions())?;
    }
    std::fs::rename(&tmp, config).context("writing ~/.ssh/config")?;
    Ok(())
}

/// Mounts [uri] in GVfs (no window), so it's in the sidebars and opens at once.
pub async fn mount(uri: &str) -> Result<()> {
    let out = tokio::time::timeout(Duration::from_secs(30), Command::new("gio").args(["mount", uri]).stdin(std::process::Stdio::null()).output())
        .await
        .context("GVfs didn't connect in 30 s")?
        .context("gio isn't there")?;
    let why = String::from_utf8_lossy(&out.stderr);
    // Already mounted is fine.
    if !out.status.success() && !why.contains("lready mounted") {
        bail!("GVfs: {}", why.lines().last().unwrap_or("couldn't connect").trim());
    }
    Ok(())
}

/// Opens [uri] in the user's file manager.
pub async fn open(uri: &str) -> Result<()> {
    let out = Command::new("gio").args(["open", uri]).output().await.context("gio isn't there")?;
    anyhow::ensure!(out.status.success(), "gio open: {}", String::from_utf8_lossy(&out.stderr).trim());
    Ok(())
}

pub async fn unmount(uri: &str) {
    let _ = tokio::time::timeout(Duration::from_secs(10), Command::new("gio").args(["mount", "-u", uri]).output()).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_alias_reads_as_the_phone_and_pins_its_key() {
        let dir = std::env::temp_dir().join(format!("vyoma-gvfs-{}", std::process::id()));
        assert_eq!(alias(&dir, "a", "Pixel 9 Pro"), "pixel-9-pro.vyoma-desktop");
        assert_eq!(alias(&dir, "a", "  ?? "), "phone.vyoma-desktop");
        let wifi = Route { host: "192.168.1.5".into(), port: Some(1739), proxy: None };
        let text = conf_text("P.Vyoma-Desktop", &wifi, "kdeconnect", Path::new("/k/key.pem"), Path::new("/k/phone hosts"), "vyoma-a").unwrap();
        assert!(text.starts_with("Host P.Vyoma-Desktop\n  HostName 192.168.1.5\n  Port 1739\n"));
        assert!(text.contains("UserKnownHostsFile \"/k/phone hosts\"") && text.contains("StrictHostKeyChecking yes"));
        assert!(text.contains("PasswordAuthentication no") && !text.contains("ProxyCommand"));
        let cable = Route { host: "127.0.0.1".into(), port: None, proxy: Some("'/bin/v' --relay '/run/s.sock'".into()) };
        let text = conf_text("P.Vyoma-Desktop", &cable, "kdeconnect", Path::new("/k"), Path::new("/h"), "vyoma-a").unwrap();
        assert!(text.contains("  ProxyCommand '/bin/v' --relay '/run/s.sock'\n") && !text.contains("Port"));
        assert!(conf_text("a b", &wifi, "u", Path::new("/k"), Path::new("/h"), "x").is_err());
    }

    #[test]
    fn a_second_phone_of_the_same_name_gets_its_own_alias() {
        let dir = std::env::temp_dir().join(format!("vyoma-gvfs-same-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("ssh")).unwrap();
        std::fs::write(conf_file(&dir, "first"), "Host pixel.vyoma-desktop\n").unwrap();
        assert_eq!(alias(&dir, "first", "Pixel"), "pixel.vyoma-desktop");
        assert_eq!(alias(&dir, "second_1", "Pixel"), "pixel-second.vyoma-desktop");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_include_goes_on_top_once_and_keeps_the_rest() {
        let dir = std::env::temp_dir().join(format!("vyoma-gvfs-inc-{}", std::process::id()));
        let config = dir.join(".ssh").join("config");
        let folder = Path::new("/c/vyoma-desktop/ssh");
        include(&config, folder).unwrap();
        let made = std::fs::read_to_string(&config).unwrap();
        assert!(made.starts_with(INCLUDE_NOTE) && made.contains("Include \"/c/vyoma-desktop/ssh/*.conf\""));
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(dir.join(".ssh")).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(std::fs::metadata(&config).unwrap().permissions().mode() & 0o777, 0o600);
        let mine = "Host work\n  User me\n";
        std::fs::write(&config, mine).unwrap();
        std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o644)).unwrap();
        include(&config, folder).unwrap();
        include(&config, folder).unwrap();
        let after = std::fs::read_to_string(&config).unwrap();
        assert!(after.ends_with(&format!("\n\n{mine}")), "the user's own config is kept as it was");
        assert_eq!(after.matches("Include").count(), 1);
        assert_eq!(std::fs::metadata(&config).unwrap().permissions().mode() & 0o777, 0o644);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
