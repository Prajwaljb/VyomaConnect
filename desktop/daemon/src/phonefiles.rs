//! The phone's files in the file manager (stock `kdeconnect.sftp`, as kdeconnectd's sftp plugin):
//! we ask, the phone starts an SFTP server and says where, and sshfs mounts it under
//! `$XDG_RUNTIME_DIR/vyoma/<phone>`. Unlike kdeconnectd, the phone's SSH host key is checked: it's
//! the phone's TLS key, so it must match the certificate we pinned at pairing. We sign in with our
//! own certificate's key, and the one-time password the phone sent over TLS if that's refused.
//! Unmounted when the phone goes. Addresses and passwords never go to the log.
//! sshfs runs as its own transient user unit: mounting needs the setuid fusermount3, which this
//! app's unit (NoNewPrivileges) can't run, and it stays that way.

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use base64::Engine;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, Command};
use vyoma_core::link::packet;

pub const SFTP: &str = "kdeconnect.sftp";
pub const SFTP_REQUEST: &str = "kdeconnect.sftp.request";

/// Payload ports the companion forwards over the cable sit this far above the phone's own.
const USB_FORWARD_SHIFT: u16 = 200;
/// The phone's port range, which that forward covers.
const PHONE_PORTS: std::ops::RangeInclusive<u16> = 1739..=1764;

pub fn request() -> Value {
    packet(SFTP_REQUEST, json!({ "startBrowsing": true }))
}

#[derive(Debug, PartialEq)]
pub struct Server {
    pub ip: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub path: String,
}

#[derive(Debug, PartialEq)]
pub enum Reply {
    Ready(Server),
    Failed(String),
    /// `serverRunning: false` and the like: nothing to do.
    Nothing,
}

pub fn parse(p: &Value) -> Reply {
    let b = &p["body"];
    if let Some(e) = b["errorMessage"].as_str() {
        return Reply::Failed(e.to_owned());
    }
    let port = b["port"].as_u64().or_else(|| b["port"].as_str()?.parse().ok()).and_then(|p| u16::try_from(p).ok());
    match (b["ip"].as_str(), port, b["password"].as_str()) {
        (Some(ip), Some(port), Some(password)) => Reply::Ready(Server {
            ip: ip.to_owned(),
            port,
            user: b["user"].as_str().unwrap_or("kdeconnect").to_owned(),
            password: password.to_owned(),
            path: b["path"].as_str().filter(|p| p.starts_with('/')).unwrap_or("/").to_owned(),
        }),
        _ => Reply::Nothing,
    }
}

/// A known_hosts line for [alias] from the phone's certificate (DER): its RSA key as OpenSSH writes it.
pub fn known_host(cert: &[u8], alias: &str) -> Result<String> {
    use x509_parser::prelude::*;
    let (_, cert) = X509Certificate::from_der(cert).context("reading the phone's certificate")?;
    let key = cert.public_key().parsed().context("reading the phone's key")?;
    let x509_parser::public_key::PublicKey::RSA(rsa) = key else { bail!("the phone's key isn't RSA") };
    let mut blob = Vec::new();
    let mut field = |bytes: &[u8]| {
        blob.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        blob.extend_from_slice(bytes);
    };
    field(b"ssh-rsa");
    field(&mpint(rsa.exponent));
    field(&mpint(rsa.modulus));
    Ok(format!("{alias} ssh-rsa {}\n", base64::engine::general_purpose::STANDARD.encode(blob)))
}

/// An SSH mpint: big-endian, no leading zeros, a zero in front if the top bit is set.
fn mpint(n: &[u8]) -> Vec<u8> {
    let n = &n[n.iter().position(|&b| b != 0).unwrap_or(n.len())..];
    let mut out = Vec::with_capacity(n.len() + 1);
    if n.first().is_some_and(|&b| b & 0x80 != 0) {
        out.push(0);
    }
    out.extend_from_slice(n);
    out
}

/// Where a phone called [name] is mounted.
pub fn mount_point(name: &str) -> PathBuf {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let clean: String = name.chars().map(|c| if c == '/' || c.is_control() { '_' } else { c }).collect();
    runtime.join("vyoma").join(if clean.trim().is_empty() || clean.starts_with('.') { "Phone".into() } else { clean })
}

/// A running sshfs (its `systemd-run`, which lasts as long as it): [Mount::unmount] takes it down.
pub struct Mount {
    child: Child,
    unit: String,
    pub path: PathBuf,
}

impl Mount {
    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub async fn unmount(mut self) {
        stop(&self.unit).await;
        let _ = self.child.kill().await;
    }
}

/// Stopping sshfs's unit unmounts it (sshfs unmounts on SIGTERM); a stale mount left by a crash
/// is lazily unmounted from a unit of its own, for the same reason.
async fn stop(unit: &str) {
    let _ = Command::new("systemctl").args(["--user", "stop", unit]).output().await;
    let _ = Command::new("systemctl").args(["--user", "reset-failed", unit]).output().await;
}

async fn clear(at: &Path) {
    let _ = Command::new("systemd-run").args(["--user", "--quiet", "--wait", "--collect", "--", "fusermount3", "-uz"]).arg(at).output().await;
}

/// The unit's name for device [id]: ids are letters, digits, `_` and `-`, which unit names allow.
fn unit_name(id: &str) -> String {
    let clean: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
    format!("vyoma-files-{clean}.service")
}

/// Mounts the phone's [server]. [phone]: the link's address (loopback over the cable, where the
/// port comes through the companion's adb forward). [key]: our private key; [cert]: the phone's.
pub async fn mount(server: &Server, phone: IpAddr, id: &str, at: &Path, key: &Path, cert: &[u8], dir: &Path) -> Result<Mount> {
    let (host, port) = if phone.is_loopback() {
        anyhow::ensure!(PHONE_PORTS.contains(&server.port), "the phone's file server is outside the cable's ports");
        ("127.0.0.1".to_owned(), server.port + USB_FORWARD_SHIFT)
    } else {
        // Its link's address, not the one it says: the one we know reaches it, and it can't send us elsewhere.
        (phone.to_string(), server.port)
    };
    let alias = format!("vyoma-{id}");
    let hosts = dir.join("phone-hosts");
    vyoma_core::identity::write_private(&hosts, known_host(cert, &alias)?.as_bytes())?;
    let unit = unit_name(id);
    stop(&unit).await;
    if mounted(at) {
        clear(at).await;
        anyhow::ensure!(!mounted(at), "{} is still mounted", at.display());
    }
    std::fs::create_dir_all(at).with_context(|| format!("making {}", at.display()))?;
    let host_arg = if host.contains(':') { format!("[{host}]") } else { host };
    let mut child = Command::new("systemd-run")
        .args(["--user", "--quiet", "--pipe", "--collect", "--unit", &unit, "--", "sshfs"])
        .arg(format!("{}@{host_arg}:{}", server.user, server.path))
        .arg(at)
        .args(["-f", "-p", &port.to_string()])
        .args(["-o", "password_stdin", "-o", "IdentitiesOnly=yes", "-o", "StrictHostKeyChecking=yes", "-o", "ConnectTimeout=10"])
        .args(["-o", "ServerAliveInterval=15", "-o", "ServerAliveCountMax=2", "-o", "idmap=user"])
        .arg("-o")
        .arg(format!("HostKeyAlias={alias}"))
        .arg("-o")
        .arg(format!("UserKnownHostsFile={}", hosts.display()))
        .arg("-o")
        .arg(format!("IdentityFile={}", key.display()))
        .arg("-o")
        .arg(format!("fsname=vyoma:{id}"))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("systemd-run isn't there")?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(format!("{}\n", server.password).as_bytes()).await?;
    }
    // Up once the mount shows; an sshfs that ends first says why on stderr.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        if mounted(at) {
            return Ok(Mount { child, unit, path: at.to_owned() });
        }
        if let Some(status) = child.try_wait()? {
            let mut why = String::new();
            if let Some(mut err) = child.stderr.take() {
                use tokio::io::AsyncReadExt;
                let _ = err.read_to_string(&mut why).await;
            }
            bail!("sshfs ended ({status}): {}", why.lines().last().unwrap_or("no reason given"));
        }
        if tokio::time::Instant::now() > deadline {
            stop(&unit).await;
            let _ = child.kill().await;
            bail!("the phone's files didn't mount in 20 s");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn mounted(at: &Path) -> bool {
    let Ok(info) = std::fs::read_to_string("/proc/self/mountinfo") else { return false };
    let at = at.to_string_lossy();
    // Spaces in mount points are written as \040.
    let at = at.replace(' ', "\\040");
    info.lines().any(|l| l.split(' ').nth(4) == Some(at.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_phones_reply() {
        let p = json!({ "body": { "ip": "10.0.0.2", "port": 1740, "user": "kdeconnect", "password": "pw", "path": "/" } });
        assert_eq!(parse(&p), Reply::Ready(Server { ip: "10.0.0.2".into(), port: 1740, user: "kdeconnect".into(), password: "pw".into(), path: "/".into() }));
        assert_eq!(parse(&json!({ "body": { "errorMessage": "no" } })), Reply::Failed("no".into()));
        assert_eq!(parse(&json!({ "body": { "serverRunning": false } })), Reply::Nothing);
    }

    #[test]
    fn writes_ssh_numbers_as_openssh_does() {
        assert_eq!(mpint(&[0, 0, 1, 0]), vec![1, 0]);
        assert_eq!(mpint(&[0x80]), vec![0, 0x80]);
        assert_eq!(mpint(&[0, 0x7f]), vec![0x7f]);
    }

    #[test]
    fn a_phone_name_is_a_safe_folder() {
        assert!(mount_point("A/B").ends_with("vyoma/A_B"));
        assert!(mount_point("..").ends_with("vyoma/Phone"));
        assert_eq!(unit_name("ab_1-c/d"), "vyoma-files-ab_1-c_d.service");
    }

    /// The same line as OpenSSH's own conversion of the certificate's key (skipped without the tools).
    #[test]
    fn an_rsa_certificate_becomes_the_known_host_openssh_would_write() {
        let dir = std::env::temp_dir().join(format!("vyoma-known-host-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let run = |cmd: &str, args: &[&str]| std::process::Command::new(cmd).args(args).current_dir(&dir).output().ok().filter(|o| o.status.success());
        let made = run("openssl", &["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", "k.pem", "-out", "c.der", "-outform", "DER", "-subj", "/CN=t", "-days", "1"]);
        let made = made.and_then(|_| run("openssl", &["x509", "-inform", "DER", "-in", "c.der", "-pubkey", "-noout", "-out", "pub.pem"]));
        let Some(theirs) = made.and_then(|_| run("ssh-keygen", &["-i", "-m", "PKCS8", "-f", "pub.pem"])) else { return };
        let ours = known_host(&std::fs::read(dir.join("c.der")).unwrap(), "alias").unwrap();
        assert_eq!(ours.trim(), format!("alias {}", String::from_utf8_lossy(&theirs.stdout).trim()));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
