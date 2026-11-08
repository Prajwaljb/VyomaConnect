//! The phone's files in the file manager (stock `kdeconnect.sftp`, as kdeconnectd's sftp plugin):
//! we ask, the phone starts an SFTP server and says where, and sshfs mounts it under
//! `$XDG_RUNTIME_DIR/vyoma/<phone>`. Unlike kdeconnectd, the phone's SSH host key is checked: it's
//! the phone's TLS key, so it must match the certificate we pinned at pairing. We sign in with our
//! own certificate's key, and the one-time password the phone sent over TLS if that's refused.
//! Unmounted when the phone goes. Addresses and passwords never go to the log. A silent link is
//! given up after ~10 s: file managers (Thunar) wait on the mount in their window's own thread,
//! so a hung sshfs freezes them until it gives up.
//! sshfs runs as its own transient user unit: mounting needs the setuid fusermount3, which this
//! app's unit (NoNewPrivileges) can't run, and it stays that way.

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use base64::Engine;
use serde_json::{Value, json};
use tokio::process::{Child, Command};
use vyoma_core::link::packet;

pub const SFTP: &str = "kdeconnect.sftp";
pub const SFTP_REQUEST: &str = "kdeconnect.sftp.request";

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

/// Where a phone called [name] is mounted this time: `$XDG_RUNTIME_DIR/vyoma/<n>/<name>`, a new
/// [n] for each mount. File managers keep a folder they've shown by its path (Thunar never looks
/// again, as sshfs sends no changes), so a mount made again at the same path showed a window left
/// open on the old one as empty; on a fresh path that window's folder is gone, and it closes.
pub fn mount_point(name: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    let base = mounts_dir();
    let clean: String = name.chars().map(|c| if c == '/' || c.is_control() { '_' } else { c }).collect();
    let clean = if clean.trim().is_empty() || clean.starts_with('.') { "Phone".into() } else { clean };
    loop {
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = base.join(n.to_string());
        if !dir.exists() {
            return dir.join(clean);
        }
    }
}

/// Where every mount lives: `$XDG_RUNTIME_DIR/vyoma`.
pub fn mounts_dir() -> PathBuf {
    runtime_dir().join("vyoma")
}

/// Removes [at], its sidebar entry and its numbered folder once nothing's mounted there,
/// unmounting a dead mount first. Only empty folders go (rmdir), never what's in them.
async fn remove_mount_point(at: &Path) {
    crate::places::remove(at);
    if mounted(at) {
        clear(at).await;
    }
    if !mounted(at) && std::fs::remove_dir(at).is_ok() {
        if let Some(n) = at.parent() {
            let _ = std::fs::remove_dir(n);
        }
    }
}

/// At start: what an earlier run left under `$XDG_RUNTIME_DIR/vyoma` (it ended without unmounting).
pub async fn tidy() {
    crate::places::remove_all();
    let Ok(entries) = std::fs::read_dir(mounts_dir()) else { return };
    for entry in entries.flatten() {
        let dir = entry.path();
        if let Ok(inner) = std::fs::read_dir(&dir) {
            for at in inner.flatten() {
                remove_mount_point(&at.path()).await;
            }
        }
        // The old layout's `vyoma/<name>`, and a numbered folder emptied above.
        if mounted(&dir) {
            clear(&dir).await;
        }
        let _ = std::fs::remove_dir(&dir);
    }
}

/// A running sshfs (its `systemd-run`, which lasts as long as it): [Mount::unmount] takes it down.
pub struct Mount {
    child: Child,
    unit: String,
    pub path: PathBuf,
    /// The cable's socket it goes through, taken down with it.
    forward: Option<PathBuf>,
}

impl Mount {
    /// None while it runs; then whether sshfs ended cleanly (unmounted: the file manager's
    /// eject) rather than failing (cut off from the phone).
    pub fn alive(&mut self) -> bool {
        self.ended().is_none()
    }

    pub fn ended(&mut self) -> Option<bool> {
        match self.child.try_wait() {
            Ok(None) => None,
            Ok(Some(status)) => Some(status.success()),
            Err(_) => Some(false),
        }
    }

    pub async fn unmount(mut self) {
        stop(&self.unit).await;
        let _ = self.child.kill().await;
        self.release().await;
    }

    /// What sshfs last said on stderr, once it's ended.
    pub async fn why(&mut self) -> String {
        last_line(&mut self.child).await
    }

    /// Takes down what an ended mount leaves behind: its cable socket and its folder.
    pub async fn release(&mut self) {
        if let Some(socket) = self.forward.take() {
            vyoma_companion::usb::unforward_unix(&socket).await;
        }
        remove_mount_point(&self.path).await;
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
    let unit = unit_name(id);
    let mut route = Vec::new();
    let mut forward = None;
    if phone.is_loopback() {
        // Loopback without a cable is Bluetooth, which carries packets only; the cable's ports
        // would answer anyway (adb holds them) and reset the connection.
        anyhow::ensure!(!vyoma_companion::usb::cable_ports().is_empty(), "Browsing the phone needs Wi-Fi or the USB cable (it's on Bluetooth only)");
        // Through a Unix socket of our own rather than the companion's TCP mirror of the port:
        // ~10 ms a file opened instead of ~50 (usb::forward_unix).
        let socket = runtime_dir().join(unit.replace(".service", ".sock"));
        vyoma_companion::usb::forward_unix(&socket, server.port).await?;
        route = vec!["127.0.0.1".to_owned(), "-o".to_owned(), format!("ProxyCommand={}", relay_command(&socket)?)];
        forward = Some(socket);
    } else {
        // Its link's address, not the one it says: the one we know reaches it, and it can't send us elsewhere.
        let host = phone.to_string();
        route.push(if host.contains(':') { format!("[{host}]") } else { host });
        route.extend(["-p".to_owned(), server.port.to_string()]);
    }
    let result = run_sshfs(server, &route, id, &unit, at, key, cert, dir).await;
    if result.is_err() {
        if let Some(socket) = &forward {
            vyoma_companion::usb::unforward_unix(socket).await;
        }
        remove_mount_point(at).await;
    } else if let Some(name) = at.file_name() {
        crate::places::add(&name.to_string_lossy(), at);
    }
    result.map(|m| Mount { forward, ..m })
}

/// This program's path, for ssh to run it as a helper ([relay], [askpass]).
fn this_program() -> Result<String> {
    let exe = std::env::current_exe().context("finding this program")?;
    // A binary replaced while running still runs; its path says so.
    Ok(exe.to_string_lossy().trim_end_matches(" (deleted)").to_owned())
}

/// `ssh`'s way to the phone over the cable: this program relaying to [socket] (see [relay]).
fn relay_command(socket: &Path) -> Result<String> {
    let exe = this_program()?;
    let socket = socket.to_string_lossy();
    // sshfs splits its options at commas, ssh expands %, and the shell reads quotes.
    anyhow::ensure!(![&exe, &*socket].iter().any(|p| p.contains([',', '\'', '%'])), "this program's path has a comma, quote or % in it");
    Ok(format!("'{exe}' {RELAY_ARG} '{socket}'"))
}

/// Started with [RELAY_ARG] (ssh's ProxyCommand): copies stdin to the Unix socket [at] and what
/// comes back to stdout, until either side ends.
pub const RELAY_ARG: &str = "--relay";
pub fn relay(at: &str) -> Result<()> {
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    let sock = UnixStream::connect(at).with_context(|| format!("connecting to {at}"))?;
    let mut to_phone = sock.try_clone()?;
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut std::io::stdin().lock(), &mut to_phone);
        let _ = to_phone.shutdown(std::net::Shutdown::Write);
    });
    // Each piece passed on as it comes: stdout is line-buffered, which held SSH's binary replies
    // back until a newline byte happened to come along.
    let (mut from_phone, mut out, mut buf) = (sock, std::io::stdout().lock(), vec![0; 64 * 1024]);
    loop {
        let n = std::io::Read::read(&mut from_phone, &mut buf)?;
        if n == 0 {
            return Ok(());
        }
        out.write_all(&buf[..n])?;
        out.flush()?;
    }
}

/// Set (to a file) for this program run as ssh's SSH_ASKPASS: it prints the phone's one-time
/// password, should ssh ask for it (our key was refused). Not sshfs's password_stdin, which
/// waits for the prompt and, as our key gets in without one, gave up after a minute (2026-11-08).
pub const ASKPASS_ENV: &str = "VYOMA_ASKPASS";
pub fn askpass() -> Result<()> {
    let file = std::env::var_os(ASKPASS_ENV).context("no password file")?;
    let password = std::fs::read_to_string(&file).context("the password is gone")?;
    println!("{password}");
    Ok(())
}

/// Removes the password file however the mount ends.
struct Forget(PathBuf);

impl Drop for Forget {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
}

#[allow(clippy::too_many_arguments)]
async fn run_sshfs(server: &Server, route: &[String], id: &str, unit: &str, at: &Path, key: &Path, cert: &[u8], dir: &Path) -> Result<Mount> {
    let alias = format!("vyoma-{id}");
    let hosts = dir.join("phone-hosts");
    vyoma_core::identity::write_private(&hosts, known_host(cert, &alias)?.as_bytes())?;
    stop(unit).await;
    if mounted(at) {
        clear(at).await;
        anyhow::ensure!(!mounted(at), "{} is still mounted", at.display());
    }
    std::fs::create_dir_all(at).with_context(|| format!("making {}", at.display()))?;
    let pass = Forget(runtime_dir().join(unit.replace(".service", ".pass")));
    vyoma_core::identity::write_private(&pass.0, server.password.as_bytes())?;
    let mut child = Command::new("systemd-run")
        .args(["--user", "--quiet", "--pipe", "--collect", "--unit", unit])
        .arg(format!("--setenv=SSH_ASKPASS={}", this_program()?))
        .args(["--setenv=SSH_ASKPASS_REQUIRE=force", &format!("--setenv={ASKPASS_ENV}={}", pass.0.display()), "--", "sshfs"])
        .arg(format!("{}@{}:{}", server.user, route[0], server.path))
        .arg(at)
        .arg("-f")
        .args(&route[1..])
        .args(["-o", "IdentitiesOnly=yes", "-o", "StrictHostKeyChecking=yes", "-o", "ConnectTimeout=10"])
        .args(["-o", "ServerAliveInterval=5", "-o", "ServerAliveCountMax=2", "-o", "idmap=user"])
        .arg("-o")
        .arg(format!("HostKeyAlias={alias}"))
        .arg("-o")
        .arg(format!("UserKnownHostsFile={}", hosts.display()))
        .arg("-o")
        .arg(format!("IdentityFile={}", key.display()))
        .arg("-o")
        .arg(format!("fsname=vyoma:{id}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("systemd-run isn't there")?;
    // Up once the mount shows; an sshfs that ends first says why on stderr.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        if mounted(at) {
            return Ok(Mount { child, unit: unit.to_owned(), path: at.to_owned(), forward: None });
        }
        if let Some(status) = child.try_wait()? {
            bail!("sshfs ended ({status}): {}", last_line(&mut child).await);
        }
        if tokio::time::Instant::now() > deadline {
            stop(unit).await;
            let _ = child.kill().await;
            bail!("the phone's files didn't mount in 20 s");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// The last line an ended sshfs wrote to stderr.
async fn last_line(child: &mut Child) -> String {
    let mut why = String::new();
    if let Some(mut err) = child.stderr.take() {
        use tokio::io::AsyncReadExt;
        let _ = tokio::time::timeout(Duration::from_secs(2), err.read_to_string(&mut why)).await;
    }
    why.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("no reason given").to_owned()
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
        assert!(mount_point("A/B").ends_with("A_B"));
        assert!(mount_point("..").ends_with("Phone"));
        let (a, b) = (mount_point("P"), mount_point("P"));
        assert_ne!(a, b, "each mount gets a path of its own");
        assert_eq!(a.parent().unwrap().parent(), b.parent().unwrap().parent());
        assert!(a.parent().unwrap().parent().unwrap().ends_with("vyoma"));
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
