//! The phone's files in Vyoma's window (the Mac and Windows; Linux mounts them with sshfs instead,
//! `daemon/src/phonefiles.rs`). As there: we ask (stock `kdeconnect.sftp.request`), the phone
//! starts its SFTP server and says where, with a one-time password sent over the TLS link. The
//! server's host key must be the phone's TLS key, the certificate pinned at pairing, so nobody on
//! the network can stand in for it. Over the USB cable the phone's port is reached through the
//! adb forward of its payload ports (port + 200, `vyoma_core::adb`).
//!
//! One session per phone, opened on first use and closed after [IDLE] unused (the phone stops its
//! server once nothing's connected, which saves its battery) or when the phone goes. Addresses
//! and passwords never go to the log.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::task::{Context as TaskContext, Poll};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use russh::client;
use russh_sftp::client::SftpSession;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use vyoma_core::device::Command;
use vyoma_core::identity::Identity;
use vyoma_core::link::packet;

use crate::window::{Answer, Devices, text};

pub const SFTP: &str = "kdeconnect.sftp";
pub const SFTP_REQUEST: &str = "kdeconnect.sftp.request";

/// The window's requests this module answers.
pub const METHODS: &[&str] = &["PhoneFiles", "FetchPhoneFile", "PutPhoneFiles", "NewPhoneFolder", "DeletePhoneFile", "RenamePhoneFile", "PhoneFilesProgress", "StopPhoneFiles"];

/// What a copy stopped by the person answers (the window says so quietly, not as a failure).
pub const STOPPED: &str = "Stopped";

/// A session unused this long is closed.
const IDLE: Duration = Duration::from_secs(180);
/// How long the phone has to start its server and say where.
const ASK_WAIT: Duration = Duration::from_secs(20);
/// How long a failed request's check of the session may take.
const PROBE_WAIT: Duration = Duration::from_secs(5);
/// How long the connection and sign-in may take.
const CONNECT_WAIT: Duration = Duration::from_secs(15);
/// Over the cable the phone's payload ports (its file server's among them) are reached here at port + this.
const CABLE_SHIFT: u16 = 200;
/// The phone's file server's ports (KDE Connect's payload range, which the cable's forwards cover).
const PHONE_PORTS: std::ops::RangeInclusive<u16> = 1739..=1764;

/// Where the phone's server is, as it said.
#[derive(Clone, Debug, PartialEq)]
pub struct Server {
    pub port: u16,
    pub user: String,
    pub password: String,
    /// Its top folders: path and name ("/", "Internal storage").
    pub roots: Vec<(String, String)>,
}

/// The phone's `kdeconnect.sftp`: where its server is, why it can't share, or nothing to act on
/// (`serverRunning: false`, sent as a phone links).
pub fn parse(p: &Value) -> Option<Result<Server, String>> {
    let b = &p["body"];
    if let Some(why) = b["errorMessage"].as_str() {
        return Some(Err(why.to_owned()));
    }
    let port = b["port"].as_u64().or_else(|| b["port"].as_str()?.parse().ok()).and_then(|p| u16::try_from(p).ok())?;
    let password = b["password"].as_str()?.to_owned();
    let user = b["user"].as_str().unwrap_or("kdeconnect").to_owned();
    let strings = |v: &Value| v.as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_owned)).collect::<Vec<_>>()).unwrap_or_default();
    let (paths, names) = (strings(&b["multiPaths"]), strings(&b["pathNames"]));
    let mut roots: Vec<(String, String)> = paths.into_iter().zip(names.into_iter().chain(std::iter::repeat(String::new()))).filter(|(p, _)| p.starts_with('/')).collect();
    if roots.is_empty() {
        let path = b["path"].as_str().filter(|p| p.starts_with('/')).unwrap_or("/");
        roots.push((path.to_owned(), String::new()));
    }
    for (path, name) in &mut roots {
        if name.is_empty() {
            *name = if path == "/" { "Phone".into() } else { leaf(path).to_owned() };
        }
    }
    Some(Ok(Server { port, user, password, roots }))
}

/// Where to reach the phone's port: its link's address, or over the cable (a loopback link) the
/// adb forward of its payload ports.
fn address(remote: IpAddr, port: u16) -> Result<SocketAddr> {
    if remote.is_loopback() {
        anyhow::ensure!(PHONE_PORTS.contains(&port), "The phone's file server isn't reachable over the cable");
        return Ok(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port + CABLE_SHIFT));
    }
    Ok(SocketAddr::new(remote, port))
}

/// A phone, as a request needs it.
#[derive(Clone)]
pub struct Phone {
    pub id: String,
    pub name: String,
    pub remote: IpAddr,
    pub cert: Vec<u8>,
}

impl Phone {
    /// Phone [id], if it's linked and can share its files.
    pub fn of(devices: &Devices, id: &str) -> Result<Phone, String> {
        let d = devices.get(id).ok_or("That phone isn't paired")?;
        if !d.takes(SFTP_REQUEST) {
            return Err(format!("{} isn't connected, or doesn't share its files", d.name));
        }
        let remote = d.remote.ok_or_else(|| format!("{} isn't connected", d.name))?;
        Ok(Phone { id: id.to_owned(), name: d.name.clone(), remote, cert: d.cert.clone() })
    }
}

/// Checks the server's host key against the phone's pinned certificate.
struct Check {
    expected: Vec<u8>,
}

impl client::Handler for Check {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &russh::keys::PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let russh::keys::PublicKeyOrCertificate::PublicKey { key, .. } = key else { return Ok(false) };
        let ours = key.to_bytes().map(|b| b == self.expected).unwrap_or(false);
        if !ours {
            tracing::warn!("a file server answered with a host key that isn't the phone's: not connecting");
        }
        Ok(ours)
    }
}

/// An open session.
struct Open {
    sftp: SftpSession,
    ssh: client::Handle<Check>,
    roots: Vec<(String, String)>,
    /// When it was last used, in ms since [epoch].
    used: AtomicU64,
}

fn now_ms() -> u64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

impl Open {
    fn touch(&self) {
        self.used.store(now_ms(), Ordering::Relaxed);
    }
}

/// A copy under way, for the window's progress bar and its Stop.
#[derive(Default)]
pub struct Transfer {
    /// The file being copied now.
    name: Mutex<String>,
    done: AtomicU64,
    total: AtomicU64,
    stop: AtomicBool,
}

impl Transfer {
    fn now(&self, name: &str) {
        *self.name.lock().unwrap_or_else(|e| e.into_inner()) = name.to_owned();
    }

    fn json(&self) -> Value {
        json!({
            "name": *self.name.lock().unwrap_or_else(|e| e.into_inner()),
            "done": self.done.load(Ordering::Relaxed),
            "total": self.total.load(Ordering::Relaxed),
        })
    }
}

/// Reads through [inner], counting into [transfer] and failing once it's asked to stop (what
/// it's copied into then cleans up: a fetch's part file, a put's half file).
struct Counted<'a, R> {
    inner: R,
    transfer: &'a Transfer,
}

impl<R: tokio::io::AsyncRead + Unpin> tokio::io::AsyncRead for Counted<'_, R> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>, buf: &mut tokio::io::ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        if self.transfer.stop.load(Ordering::Relaxed) {
            return Poll::Ready(Err(std::io::Error::other(STOPPED)));
        }
        let before = buf.filled().len();
        let read = Pin::new(&mut self.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = read {
            self.transfer.done.fetch_add((buf.filled().len() - before) as u64, Ordering::Relaxed);
        }
        read
    }
}

/// Every phone's session, and the server details being waited for.
pub struct PhoneFiles {
    commands: mpsc::UnboundedSender<Command>,
    /// This computer's key, which the phone also takes as a sign-in (the one-time password is the other).
    key: Option<Arc<russh::keys::PrivateKey>>,
    waiting: Mutex<HashMap<String, oneshot::Sender<Result<Server, String>>>>,
    /// Per phone: its session (one connects at a time; others wait and share it).
    sessions: Mutex<HashMap<String, Arc<tokio::sync::Mutex<Option<Arc<Open>>>>>>,
    /// Per phone: the copy under way, if one is.
    transfers: Mutex<HashMap<String, Arc<Transfer>>>,
}

impl PhoneFiles {
    pub fn new(me: &Identity, commands: mpsc::UnboundedSender<Command>) -> Arc<PhoneFiles> {
        let key = match russh::keys::pkcs8::decode_pkcs8(me.key.secret_der(), None) {
            Ok(k) => Some(Arc::new(k)),
            Err(e) => {
                tracing::info!("phone files: this computer's key can't sign in ({e}); the one-time password will");
                None
            }
        };
        Arc::new(PhoneFiles { commands, key, waiting: Mutex::default(), sessions: Mutex::default(), transfers: Mutex::default() })
    }

    /// The phone [id] sent `kdeconnect.sftp`.
    pub fn reply(&self, id: &str, p: &Value) {
        let Some(reply) = parse(p) else { return };
        if let Some(waiter) = self.waiting.lock().unwrap_or_else(|e| e.into_inner()).remove(id) {
            let _ = waiter.send(reply);
        }
    }

    /// Phone [id] went (or was unpaired): its session ends.
    pub fn forget(&self, id: &str) {
        self.waiting.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
        let slot = self.sessions.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
        if let Some(slot) = slot {
            tokio::spawn(async move {
                if let Some(open) = slot.lock().await.take() {
                    close(&open).await;
                }
            });
        }
    }

    /// A new copy for phone [id], [total] bytes; the record goes once [Begun] is dropped.
    fn begin(self: &Arc<Self>, id: &str, total: u64) -> Begun {
        let transfer = Arc::new(Transfer { total: AtomicU64::new(total), ..Default::default() });
        self.transfers.lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_owned(), transfer.clone());
        Begun { files: self.clone(), id: id.to_owned(), transfer }
    }

    /// Phone [id]'s copy under way: its file, bytes done and total; null if none is.
    pub fn progress(&self, id: &str) -> Value {
        self.transfers.lock().unwrap_or_else(|e| e.into_inner()).get(id).map_or(Value::Null, |t| t.json())
    }

    /// Stops phone [id]'s copy under way, if one is.
    pub fn stop(&self, id: &str) {
        if let Some(t) = self.transfers.lock().unwrap_or_else(|e| e.into_inner()).get(id) {
            t.stop.store(true, Ordering::Relaxed);
        }
    }

    /// Phone [id]'s session's request time limit (tests: a frozen server fails sooner).
    #[cfg(all(test, unix))]
    async fn sftp_timeout(&self, id: &str, secs: u64) {
        if let Some(open) = self.slot(id).lock().await.as_ref() {
            open.sftp.set_timeout(secs);
        }
    }

    /// After a failed request: is phone [id]'s session still answering? A connection that died
    /// without closing (the phone left the network, its screen-off cut) would otherwise fail every
    /// request until the idle limit; dropped, the next request connects again.
    pub async fn recover(&self, id: &str) {
        let slot = self.slot(id);
        let mut held = slot.lock().await;
        let Some(open) = held.as_ref() else { return };
        let root = open.roots.first().map_or_else(|| "/".to_owned(), |r| r.0.clone());
        let answers = !open.ssh.is_closed() && matches!(tokio::time::timeout(PROBE_WAIT, open.sftp.metadata(root)).await, Ok(Ok(_)));
        if !answers {
            tracing::info!("{id}: its file server stopped answering; connecting again next time");
            if let Some(open) = held.take() {
                tokio::spawn(async move { close(&open).await });
            }
        }
    }

    fn slot(&self, id: &str) -> Arc<tokio::sync::Mutex<Option<Arc<Open>>>> {
        self.sessions.lock().unwrap_or_else(|e| e.into_inner()).entry(id.to_owned()).or_default().clone()
    }

    /// Phone [phone]'s session: the open one, else a new one.
    async fn session(self: &Arc<Self>, phone: &Phone) -> Result<Arc<Open>> {
        let slot = self.slot(&phone.id);
        let mut held = slot.lock().await;
        if let Some(open) = held.as_ref().filter(|o| !o.ssh.is_closed()) {
            open.touch();
            return Ok(open.clone());
        }
        let open = Arc::new(self.connect(phone).await?);
        *held = Some(open.clone());
        drop(held);
        self.watch_idle(phone.id.clone(), Arc::downgrade(&slot));
        Ok(open)
    }

    /// Asks the phone to start its server, then connects and signs in.
    async fn connect(&self, phone: &Phone) -> Result<Open> {
        let expected = vyoma_core::cert::ssh_rsa_key(&phone.cert).context("The phone's key can't be checked (it isn't RSA)")?;
        let (tx, rx) = oneshot::channel();
        self.waiting.lock().unwrap_or_else(|e| e.into_inner()).insert(phone.id.clone(), tx);
        self.commands
            .send(Command::Send(phone.id.clone(), packet(SFTP_REQUEST, json!({ "startBrowsing": true }))))
            .map_err(|_| anyhow!("Vyoma is stopping"))?;
        let server = match tokio::time::timeout(ASK_WAIT, rx).await {
            Ok(Ok(Ok(server))) => server,
            Ok(Ok(Err(why))) => bail!("{why}"),
            Ok(Err(_)) => bail!("{} went away", phone.name),
            Err(_) => {
                self.waiting.lock().unwrap_or_else(|e| e.into_inner()).remove(&phone.id);
                bail!("{} didn't start sharing its files. Open Vyoma on it, then try again", phone.name)
            }
        };
        let at = address(phone.remote, server.port)?;
        let config = Arc::new(client::Config { inactivity_timeout: Some(IDLE + Duration::from_secs(60)), nodelay: true, ..Default::default() });
        let signed_in = async {
            let mut ssh = client::connect(config, at, Check { expected }).await.context("couldn't reach the phone's file server")?;
            sign_in(&mut ssh, &server, self.key.clone()).await?;
            let channel = ssh.channel_open_session().await?;
            channel.request_subsystem(true, "sftp").await?;
            let sftp = SftpSession::new(channel.into_stream()).await.context("the phone's file server didn't start SFTP")?;
            anyhow::Ok((ssh, sftp))
        };
        let (ssh, sftp) = tokio::time::timeout(CONNECT_WAIT, signed_in)
            .await
            .map_err(|_| anyhow!("{}'s file server didn't answer", phone.name))?
            .with_context(|| format!("Couldn't open {}'s files", phone.name))?;
        tracing::info!("{}: browsing its files", phone.id);
        let open = Open { sftp, ssh, roots: server.roots, used: AtomicU64::new(0) };
        open.touch();
        Ok(open)
    }

    /// Closes the session in [slot] once it's been unused for [IDLE].
    fn watch_idle(self: &Arc<Self>, id: String, slot: std::sync::Weak<tokio::sync::Mutex<Option<Arc<Open>>>>) {
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                let Some(slot) = slot.upgrade() else { return };
                let mut held = slot.lock().await;
                let Some(open) = held.as_ref() else { return };
                if open.ssh.is_closed() {
                    *held = None;
                    return;
                }
                if now_ms().saturating_sub(open.used.load(Ordering::Relaxed)) >= IDLE.as_millis() as u64 {
                    if let Some(open) = held.take() {
                        close(&open).await;
                    }
                    tracing::info!("{id}: done browsing its files (unused)");
                    return;
                }
            }
        });
    }

    /// A folder's entries (folders first, then by name; hidden ones left out) and the phone's top folders.
    pub async fn list(self: &Arc<Self>, phone: &Phone, path: Option<&str>) -> Result<Value> {
        let open = self.session(phone).await?;
        let path = match path.filter(|p| p.starts_with('/')) {
            Some(p) => clean(p).context("That folder's name can't be used")?,
            None => open.roots.first().map(|r| r.0.clone()).unwrap_or_else(|| "/".into()),
        };
        let read = open.sftp.read_dir(path.clone()).await.with_context(|| format!("Couldn't open {}", leaf(&path)))?;
        let mut entries: Vec<(bool, String, u64, u64)> = read
            .filter(|e| !e.file_name().starts_with('.'))
            .map(|e| {
                let meta = e.metadata();
                let modified = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
                (e.file_type().is_dir(), e.file_name(), if e.file_type().is_dir() { 0 } else { meta.len() }, modified)
            })
            .collect();
        entries.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase())));
        open.touch();
        Ok(json!({
            "path": path,
            "name": open.roots.iter().find(|r| r.0 == path).map_or_else(|| leaf(&path).to_owned(), |r| r.1.clone()),
            "parent": parent(&path, &open.roots),
            "roots": open.roots.iter().map(|(p, n)| json!({ "path": p, "name": n })).collect::<Vec<_>>(),
            "entries": entries.into_iter().map(|(folder, name, size, modified)| json!({ "name": name, "folder": folder, "size": size, "modified": modified })).collect::<Vec<_>>(),
        }))
    }

    /// Copies the phone's file [path], or folder with everything in it, into [dir] (a free name,
    /// never over what's there); where it landed.
    pub async fn fetch(self: &Arc<Self>, phone: &Phone, path: &str, dir: &Path) -> Result<PathBuf> {
        let path = clean(path).context("That file's name can't be used")?;
        let open = self.session(phone).await?;
        let meta = open.sftp.metadata(path.clone()).await.with_context(|| format!("{} isn't there any more", leaf(&path)))?;
        if meta.is_dir() {
            return self.fetch_folder(phone, &open, &path, dir).await;
        }
        let file = open.sftp.open(path.clone()).await.with_context(|| format!("Couldn't open {}", leaf(&path)))?;
        let begun = self.begin(&phone.id, meta.len());
        begun.transfer.now(leaf(&path));
        let mut counted = Counted { inner: file, transfer: &begun.transfer };
        let landed = vyoma_core::share::land(&mut counted, dir, leaf(&path), meta.modified().ok()).await;
        let landed = stopped(&begun.transfer, landed.with_context(|| format!("Couldn't copy {}", leaf(&path))))?;
        open.touch();
        tracing::info!("{}: copied a file from the phone", phone.id);
        Ok(landed)
    }

    /// Copies the phone's folder [path] into a new folder in [dir]. Half a folder isn't left
    /// behind: if it fails or is stopped, what came so far is removed.
    async fn fetch_folder(self: &Arc<Self>, phone: &Phone, open: &Open, path: &str, dir: &Path) -> Result<PathBuf> {
        let total = Box::pin(phone_size(open, path)).await?;
        let begun = self.begin(&phone.id, total);
        tokio::fs::create_dir_all(dir).await.with_context(|| format!("Couldn't make {}", dir.display()))?;
        let target = new_folder_in(dir, &vyoma_core::share::safe_name(leaf(path))).await?;
        let copied = Box::pin(fetch_tree(open, path, &target, &begun.transfer)).await;
        if copied.is_err() {
            let _ = tokio::fs::remove_dir_all(&target).await;
        }
        stopped(&begun.transfer, copied.with_context(|| format!("Couldn't copy {}", leaf(path))))?;
        open.touch();
        tracing::info!("{}: copied a folder from the phone", phone.id);
        Ok(target)
    }

    /// Copies files from this computer into the phone's folder [dir] (folders with what's in them);
    /// a name already there gets " (2)" and so on. How many files went.
    pub async fn put(self: &Arc<Self>, phone: &Phone, dir: &str, locals: &[PathBuf]) -> Result<usize> {
        let dir = clean(dir).context("That folder's name can't be used")?;
        let open = self.session(phone).await?;
        let mut total = 0;
        for local in locals {
            total += size_of(local).await;
        }
        let begun = self.begin(&phone.id, total);
        let mut sent = 0;
        for local in locals {
            sent += stopped(&begun.transfer, Box::pin(put_one(&open, local, &dir, &begun.transfer)).await)?;
            open.touch();
        }
        tracing::info!("{}: copied {sent} file(s) to the phone", phone.id);
        Ok(sent)
    }

    pub async fn new_folder(self: &Arc<Self>, phone: &Phone, path: &str) -> Result<()> {
        let path = clean(path).context("That name can't be used for a folder")?;
        let open = self.session(phone).await?;
        anyhow::ensure!(!open.sftp.try_exists(path.clone()).await.unwrap_or(false), "There's already something called {} there", leaf(&path));
        open.sftp.create_dir(path.clone()).await.with_context(|| format!("Couldn't make {}", leaf(&path)))?;
        open.touch();
        Ok(())
    }

    /// Renames the phone's file or folder [path] to [name] in the same folder; never over
    /// something already called that. The new path.
    pub async fn rename(self: &Arc<Self>, phone: &Phone, path: &str, name: &str) -> Result<String> {
        let path = clean(path).context("That name can't be used")?;
        let name = name.trim();
        anyhow::ensure!(!name.is_empty() && name != "." && name != ".." && !name.contains('/') && !name.chars().any(char::is_control), "A name can't have / in it");
        let open = self.session(phone).await?;
        anyhow::ensure!(path != "/" && !open.roots.iter().any(|r| r.0 == path), "The phone's top folders can't be renamed");
        let to = join(path.rsplit_once('/').map_or("/", |(up, _)| if up.is_empty() { "/" } else { up }), name);
        if to == path {
            return Ok(to);
        }
        // Only a change of case may land on "itself" (a phone's storage ignores case).
        let same_but_case = to.to_lowercase() == path.to_lowercase();
        anyhow::ensure!(same_but_case || !open.sftp.try_exists(to.clone()).await.unwrap_or(false), "There's already something called {name} there");
        open.sftp.rename(path.clone(), to.clone()).await.with_context(|| format!("Couldn't rename {}", leaf(&path)))?;
        open.touch();
        Ok(to)
    }

    /// Deletes the phone's file [path], or folder with everything in it (links themselves, never
    /// what they point to). A top folder can't be.
    pub async fn delete(self: &Arc<Self>, phone: &Phone, path: &str) -> Result<()> {
        let path = clean(path).context("That name can't be used")?;
        let open = self.session(phone).await?;
        anyhow::ensure!(path != "/" && !open.roots.iter().any(|r| r.0 == path), "The phone's top folders can't be deleted");
        Box::pin(delete_one(&open, &path)).await?;
        open.touch();
        tracing::info!("{}: deleted from the phone", phone.id);
        Ok(())
    }
}

/// Signs in with this computer's key (the phone knows it from pairing), else the one-time password.
async fn sign_in(ssh: &mut client::Handle<Check>, server: &Server, key: Option<Arc<russh::keys::PrivateKey>>) -> Result<()> {
    if let Some(key) = key {
        let hash = if key.algorithm().is_rsa() { ssh.best_supported_rsa_hash().await.ok().flatten().flatten() } else { None };
        let by_key = ssh.authenticate_publickey(server.user.clone(), russh::keys::PrivateKeyWithHashAlg::new(key, hash)).await;
        if matches!(by_key, Ok(client::AuthResult::Success)) {
            return Ok(());
        }
    }
    let by_password = ssh.authenticate_password(server.user.clone(), server.password.clone()).await?;
    anyhow::ensure!(matches!(by_password, client::AuthResult::Success), "the phone refused the sign-in");
    Ok(())
}

async fn close(open: &Open) {
    let _ = open.sftp.close().await;
    let _ = open.ssh.disconnect(russh::Disconnect::ByApplication, "", "en").await;
}

/// Phone [id]'s copy record, gone when the copy ends.
struct Begun {
    files: Arc<PhoneFiles>,
    id: String,
    transfer: Arc<Transfer>,
}

impl Drop for Begun {
    fn drop(&mut self) {
        let mut all = self.files.transfers.lock().unwrap_or_else(|e| e.into_inner());
        if all.get(&self.id).is_some_and(|t| Arc::ptr_eq(t, &self.transfer)) {
            all.remove(&self.id);
        }
    }
}

/// [done], or [STOPPED] as its whole error if the person stopped it.
fn stopped<T>(transfer: &Transfer, done: Result<T>) -> Result<T> {
    match done {
        Err(_) if transfer.stop.load(Ordering::Relaxed) => Err(anyhow!(STOPPED)),
        other => other,
    }
}

/// The bytes in the phone's folder [path] (links not followed).
async fn phone_size(open: &Open, path: &str) -> Result<u64> {
    let mut total = 0;
    for e in open.sftp.read_dir(path.to_owned()).await.with_context(|| format!("Couldn't open {}", leaf(path)))? {
        let name = e.file_name();
        if name == "." || name == ".." {
            continue;
        }
        if e.file_type().is_dir() {
            total += Box::pin(phone_size(open, &join(path, &name))).await?;
        } else if e.file_type().is_file() {
            total += e.metadata().len();
        }
    }
    Ok(total)
}

/// What's in the phone's folder [path], copied into [into] (made already).
async fn fetch_tree(open: &Open, path: &str, into: &Path, transfer: &Transfer) -> Result<()> {
    let entries: Vec<_> = open.sftp.read_dir(path.to_owned()).await.with_context(|| format!("Couldn't open {}", leaf(path)))?.collect();
    for e in entries {
        let name = e.file_name();
        if name == "." || name == ".." {
            continue;
        }
        let local = vyoma_core::share::safe_name(&name);
        let from = join(path, &name);
        if e.file_type().is_dir() {
            let sub = into.join(&local);
            tokio::fs::create_dir(&sub).await.with_context(|| format!("Couldn't make {local}"))?;
            Box::pin(fetch_tree(open, &from, &sub, transfer)).await?;
        } else if e.file_type().is_file() {
            let file = open.sftp.open(from.clone()).await.with_context(|| format!("Couldn't open {name}"))?;
            transfer.now(&name);
            let mut counted = Counted { inner: file, transfer };
            vyoma_core::share::land(&mut counted, into, &local, e.metadata().modified().ok()).await.with_context(|| format!("Couldn't copy {name}"))?;
        }
    }
    Ok(())
}

/// A new folder [name] in [dir], or `name (2)` and so on: made at once, so nothing else takes it.
async fn new_folder_in(dir: &Path, name: &str) -> Result<PathBuf> {
    for n in 1..1000 {
        let candidate = if n == 1 { dir.join(name) } else { dir.join(format!("{name} ({n})")) };
        match tokio::fs::create_dir(&candidate).await {
            Ok(()) => return Ok(candidate),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e).with_context(|| format!("Couldn't make {name}")),
        }
    }
    bail!("Couldn't find a free name for {name}")
}

/// The bytes in [local]: a file's size, a folder's files' sizes (links not followed).
async fn size_of(local: &Path) -> u64 {
    let Ok(meta) = tokio::fs::symlink_metadata(local).await else { return 0 };
    if meta.is_file() {
        return meta.len();
    }
    if !meta.is_dir() {
        return 0;
    }
    let mut total = 0;
    if let Ok(mut read) = tokio::fs::read_dir(local).await {
        while let Ok(Some(e)) = read.next_entry().await {
            total += Box::pin(size_of(&e.path())).await;
        }
    }
    total
}

async fn put_one(open: &Open, local: &Path, dir: &str, transfer: &Transfer) -> Result<usize> {
    let name = local.file_name().map(|n| n.to_string_lossy().into_owned()).context("That file has no name")?;
    let name = vyoma_core::share::safe_name(&name);
    let target = free_name(open, dir, &name).await;
    let meta = tokio::fs::symlink_metadata(local).await.with_context(|| format!("Couldn't read {name}"))?;
    if meta.is_dir() {
        open.sftp.create_dir(target.clone()).await.with_context(|| format!("Couldn't make {name} on the phone"))?;
        let mut sent = 0;
        let mut read = tokio::fs::read_dir(local).await?;
        while let Some(e) = read.next_entry().await? {
            sent += Box::pin(put_one(open, &e.path(), &target, transfer)).await?;
        }
        return Ok(sent);
    }
    if !meta.is_file() {
        return Ok(0); // A link or a device: not copied.
    }
    let from = tokio::fs::File::open(local).await.with_context(|| format!("Couldn't read {name}"))?;
    let mut to = open.sftp.create(target.clone()).await.with_context(|| format!("Couldn't write {name} on the phone"))?;
    transfer.now(&name);
    let mut from = Counted { inner: from, transfer };
    let copied = async {
        tokio::io::copy(&mut from, &mut to).await?;
        tokio::io::AsyncWriteExt::shutdown(&mut to).await?;
        anyhow::Ok(())
    };
    if let Err(e) = copied.await {
        // Half a file isn't left behind.
        let _ = open.sftp.remove_file(target).await;
        return Err(e.context(format!("Couldn't copy {name}")));
    }
    Ok(1)
}

/// `dir/name`, or `dir/name (2).ext` and so on, whichever isn't on the phone yet.
async fn free_name(open: &Open, dir: &str, name: &str) -> String {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
        _ => (name, String::new()),
    };
    let mut candidate = join(dir, name);
    let mut n = 2;
    while open.sftp.try_exists(candidate.clone()).await.unwrap_or(false) && n < 1000 {
        candidate = join(dir, &format!("{stem} ({n}){ext}"));
        n += 1;
    }
    candidate
}

async fn delete_one(open: &Open, path: &str) -> Result<()> {
    let meta = open.sftp.symlink_metadata(path.to_owned()).await.with_context(|| format!("{} isn't there any more", leaf(path)))?;
    if meta.is_dir() {
        let entries: Vec<String> = open.sftp.read_dir(path.to_owned()).await?.map(|e| e.file_name()).filter(|n| n != "." && n != "..").collect();
        for name in entries {
            Box::pin(delete_one(open, &join(path, &name))).await?;
        }
        open.sftp.remove_dir(path.to_owned()).await.with_context(|| format!("Couldn't delete {}", leaf(path)))?;
    } else {
        open.sftp.remove_file(path.to_owned()).await.with_context(|| format!("Couldn't delete {}", leaf(path)))?;
    }
    Ok(())
}

/// [path] as one absolute path on the phone: no `.` or `..` parts, no empty ones, no control
/// characters. None if it isn't absolute or has a part that can't be.
fn clean(path: &str) -> Option<String> {
    if !path.starts_with('/') || path.chars().any(char::is_control) {
        return None;
    }
    let mut parts = Vec::new();
    for part in path.split('/').filter(|p| !p.is_empty()) {
        if part == "." || part == ".." {
            return None;
        }
        parts.push(part);
    }
    Some(format!("/{}", parts.join("/")))
}

fn join(dir: &str, name: &str) -> String {
    format!("{}/{}", dir.trim_end_matches('/'), name)
}

/// The last part of a path ("/DCIM/Camera" → "Camera").
fn leaf(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().filter(|l| !l.is_empty()).unwrap_or(path)
}

/// The folder above [path], unless it's a top folder (or above one).
fn parent(path: &str, roots: &[(String, String)]) -> Option<String> {
    if roots.iter().any(|r| r.0 == path) || path == "/" {
        return None;
    }
    let up = match path.trim_end_matches('/').rsplit_once('/') {
        Some(("", _)) => "/".to_owned(),
        Some((up, _)) => up.to_owned(),
        None => return None,
    };
    // Not above where the phone shares from.
    roots.iter().any(|r| r.0 == "/" || up == r.0 || up.starts_with(&format!("{}/", r.0))).then_some(up)
}

/// The window's request [method], as a task that doesn't hold the side's loop (listing or copying
/// can take a while); None if it isn't one of [METHODS]. [downloads]: where fetched files go.
pub fn request(files: &Arc<PhoneFiles>, method: &str, args: &Value, devices: &Devices, downloads: PathBuf) -> Option<impl Future<Output = Answer> + Send + 'static> {
    if !METHODS.contains(&method) {
        return None;
    }
    let (files, method, args) = (files.clone(), method.to_owned(), args.clone());
    // These two answer at once, connected or not.
    let quick = match method.as_str() {
        "PhoneFilesProgress" => Some(text(&args, 0).map(|id| files.progress(&id))),
        "StopPhoneFiles" => Some(text(&args, 0).map(|id| {
            files.stop(&id);
            Value::Null
        })),
        _ => None,
    };
    let phone = text(&args, 0).and_then(|id| Phone::of(devices, &id));
    Some(async move {
        if let Some(answer) = quick {
            return answer;
        }
        let phone = phone?;
        let arg = |i: usize| text(&args, i);
        let done: Result<Value> = match method.as_str() {
            "PhoneFiles" => files.list(&phone, args[1].as_str()).await,
            "FetchPhoneFile" => files.fetch(&phone, &arg(1)?, &downloads).await.map(|p| json!(p.to_string_lossy())),
            "PutPhoneFiles" => {
                let locals: Vec<PathBuf> = crate::window::strings(&args[2]).into_iter().map(PathBuf::from).collect();
                files.put(&phone, &arg(1)?, &locals).await.map(|n| json!(n))
            }
            "NewPhoneFolder" => files.new_folder(&phone, &arg(1)?).await.map(|_| Value::Null),
            "DeletePhoneFile" => files.delete(&phone, &arg(1)?).await.map(|_| Value::Null),
            "RenamePhoneFile" => files.rename(&phone, &arg(1)?, &arg(2)?).await.map(|to| json!(to)),
            _ => return Err("Not a files request".into()),
        };
        if done.is_err() {
            files.recover(&phone.id).await;
        }
        done.map_err(|e| {
            tracing::info!("{}: {method}: {e:#}", phone.id);
            // The first line is for people; the causes underneath are for the log.
            format!("{e}")
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn copies_count_and_stop() {
        let transfer = Transfer { total: AtomicU64::new(10), ..Default::default() };
        transfer.now("a.jpg");
        let mut out = Vec::new();
        tokio::io::copy(&mut Counted { inner: &b"0123456789"[..], transfer: &transfer }, &mut out).await.unwrap();
        assert_eq!(transfer.json(), json!({ "name": "a.jpg", "done": 10, "total": 10 }));
        transfer.stop.store(true, Ordering::Relaxed);
        let failed = tokio::io::copy(&mut Counted { inner: &b"more"[..], transfer: &transfer }, &mut out).await;
        assert!(failed.is_err());
        let said = stopped(&transfer, Err::<(), _>(anyhow!(failed.unwrap_err())).context("Couldn't copy a.jpg"));
        assert_eq!(format!("{}", said.unwrap_err()), STOPPED);
    }

    #[test]
    fn reads_where_the_server_is() {
        let p = json!({ "body": { "ip": "192.168.1.5", "port": 1740, "user": "kdeconnect", "password": "pw", "path": "/",
            "multiPaths": ["/", "/storage/1234-ABCD"], "pathNames": ["Internal storage", "SD card"] } });
        let s = parse(&p).unwrap().unwrap();
        assert_eq!((s.port, s.user.as_str(), s.password.as_str()), (1740, "kdeconnect", "pw"));
        assert_eq!(s.roots, vec![("/".into(), "Internal storage".into()), ("/storage/1234-ABCD".into(), "SD card".into())]);
        // Stock KDE Connect: one path, no names.
        let stock = parse(&json!({ "body": { "port": "1739", "password": "x", "path": "/storage/emulated/0" } })).unwrap().unwrap();
        assert_eq!(stock.roots, vec![("/storage/emulated/0".into(), "0".into())]);
        assert_eq!(parse(&json!({ "body": { "errorMessage": "Turn it on" } })), Some(Err("Turn it on".into())));
        assert_eq!(parse(&json!({ "body": { "serverRunning": false } })), None);
    }

    #[test]
    fn over_the_cable_the_forward_is_used() {
        assert_eq!(address("127.0.0.1".parse().unwrap(), 1740).unwrap(), "127.0.0.1:1940".parse().unwrap());
        assert!(address("127.0.0.1".parse().unwrap(), 2222).is_err(), "only the forwarded range");
        assert_eq!(address("192.168.1.5".parse().unwrap(), 1740).unwrap(), "192.168.1.5:1740".parse().unwrap());
    }

    #[test]
    fn paths_stay_on_the_phone() {
        assert_eq!(clean("/DCIM//Camera/").as_deref(), Some("/DCIM/Camera"));
        assert_eq!(clean("/"), Some("/".into()));
        assert_eq!(clean("/DCIM/../Android"), None);
        assert_eq!(clean("DCIM"), None);
        assert_eq!(clean("/a\nb"), None);
        assert_eq!(leaf("/DCIM/Camera"), "Camera");
        assert_eq!(join("/", "a.txt"), "/a.txt");
        let root = vec![("/".to_owned(), "Phone".to_owned())];
        assert_eq!(parent("/DCIM/Camera", &root).as_deref(), Some("/DCIM"));
        assert_eq!(parent("/DCIM", &root).as_deref(), Some("/"));
        assert_eq!(parent("/", &root), None);
        let sd = vec![("/storage/1234".to_owned(), "SD card".to_owned())];
        assert_eq!(parent("/storage/1234/Music", &sd).as_deref(), Some("/storage/1234"));
        assert_eq!(parent("/storage/1234", &sd), None, "not above where it shares from");
    }
}

/// Against a real OpenSSH sshd standing in for the phone (unprivileged, on loopback): its RSA host
/// key and a certificate made from it are the phone's, this computer's key is in its
/// authorized_keys. `VYOMA_SSHD_TEST=1 cargo test --release --features phone-files phonefiles::live`.
#[cfg(all(test, unix))]
mod live {
    use super::*;
    use std::process::Command as Run;

    struct Sshd {
        child: std::process::Child,
        dir: PathBuf,
    }

    impl Drop for Sshd {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn run(cmd: &mut Run) {
        let out = cmd.output().unwrap();
        assert!(out.status.success(), "{cmd:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    /// A host key and its certificate in [dir]; the certificate's DER.
    fn phone_key(dir: &Path, name: &str) -> Vec<u8> {
        let key = dir.join(name);
        run(Run::new("ssh-keygen").args(["-q", "-t", "rsa", "-b", "2048", "-m", "PEM", "-N", "", "-f"]).arg(&key));
        let cert = dir.join(format!("{name}.der"));
        run(Run::new("openssl").args(["req", "-x509", "-days", "1", "-subj", "/CN=phone", "-outform", "DER", "-key"]).arg(&key).arg("-out").arg(&cert));
        std::fs::read(cert).unwrap()
    }

    fn sshd(dir: &Path, port: u16, me: &Identity) -> Sshd {
        let public = russh::keys::pkcs8::decode_pkcs8(me.key.secret_der(), None).unwrap().public_key().to_openssh().unwrap();
        std::fs::write(dir.join("authorized_keys"), format!("{public}\n")).unwrap();
        let config = dir.join("sshd_config");
        std::fs::write(
            &config,
            format!(
                "Port {port}\nListenAddress 127.0.0.1\nHostKey {d}/host\nAuthorizedKeysFile {d}/authorized_keys\nPidFile {d}/pid\n\
                 PasswordAuthentication no\nKbdInteractiveAuthentication no\nUsePAM no\nStrictModes no\nSubsystem sftp internal-sftp\n",
                d = dir.display()
            ),
        )
        .unwrap();
        let child = Run::new("/usr/sbin/sshd").args(["-D", "-e", "-f"]).arg(&config).stderr(std::process::Stdio::null()).spawn().unwrap();
        for _ in 0..50 {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Sshd { child, dir: dir.to_owned() }
    }

    #[tokio::test]
    async fn browses_a_real_sftp_server() {
        if std::env::var_os("VYOMA_SSHD_TEST").is_none() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("vyoma-sshd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("share/DCIM")).unwrap();
        std::fs::write(dir.join("share/DCIM/photo.jpg"), b"jpeg bytes").unwrap();
        std::fs::write(dir.join("share/.hidden"), b"x").unwrap();
        let cert = phone_key(&dir, "host");
        let other = phone_key(&dir, "other");
        let me = Identity::load_or_create_in(&dir.join("me")).unwrap();
        // The phone answers on 1763; over the "cable" (a loopback link) that's this PC's 1963.
        let _sshd = sshd(&dir, 1963, &me);
        let share = dir.join("share").to_string_lossy().into_owned();

        let (tx, mut rx) = mpsc::unbounded_channel();
        let files = PhoneFiles::new(&me, tx);
        let answering = files.clone();
        let (user, root) = (std::env::var("USER").unwrap(), share.clone());
        tokio::spawn(async move {
            while let Some(Command::Send(id, p)) = rx.recv().await {
                assert_eq!(p["type"], SFTP_REQUEST);
                let body = json!({ "ip": "127.0.0.1", "port": 1763, "user": user, "password": "unused", "multiPaths": [root], "pathNames": ["Internal storage"] });
                answering.reply(&id, &json!({ "type": SFTP, "body": body }));
            }
        });
        let phone = Phone { id: "p".into(), name: "Pixel".into(), remote: "127.0.0.1".parse().unwrap(), cert: cert.clone() };

        let top = files.list(&phone, None).await.unwrap();
        assert_eq!(top["path"], share.as_str());
        assert_eq!(top["name"], "Internal storage");
        assert_eq!(top["parent"], Value::Null);
        assert_eq!(top["entries"], json!([{ "name": "DCIM", "folder": true, "size": 0, "modified": top["entries"][0]["modified"] }]), "hidden files left out");
        let dcim = files.list(&phone, Some(&format!("{share}/DCIM"))).await.unwrap();
        assert_eq!((dcim["entries"][0]["name"].as_str(), dcim["entries"][0]["size"].as_u64()), (Some("photo.jpg"), Some(10)));
        assert_eq!(dcim["parent"], share.as_str());

        let downloads = dir.join("downloads");
        let got = files.fetch(&phone, &format!("{share}/DCIM/photo.jpg"), &downloads).await.unwrap();
        assert_eq!(std::fs::read(&got).unwrap(), b"jpeg bytes");
        let again = files.fetch(&phone, &format!("{share}/DCIM/photo.jpg"), &downloads).await.unwrap();
        assert_eq!(again, downloads.join("photo (2).jpg"), "never over a file");

        let local = dir.join("from-pc");
        std::fs::create_dir_all(local.join("album")).unwrap();
        std::fs::write(local.join("notes.txt"), b"hello phone").unwrap();
        std::fs::write(local.join("album/a.png"), b"png").unwrap();
        let sent = files.put(&phone, &format!("{share}/DCIM"), &[local.join("notes.txt"), local.join("album"), local.join("notes.txt")]).await.unwrap();
        assert_eq!(sent, 3);
        assert_eq!(std::fs::read(dir.join("share/DCIM/notes.txt")).unwrap(), b"hello phone");
        assert_eq!(std::fs::read(dir.join("share/DCIM/notes (2).txt")).unwrap(), b"hello phone", "a name already there gets a number");
        assert_eq!(std::fs::read(dir.join("share/DCIM/album/a.png")).unwrap(), b"png");
        assert!(files.progress("p").is_null(), "no copy under way once it's done");

        // A whole folder from the phone, into a new folder beside an older copy.
        let album = files.fetch(&phone, &format!("{share}/DCIM/album"), &downloads).await.unwrap();
        assert_eq!(album, downloads.join("album"));
        assert_eq!(std::fs::read(album.join("a.png")).unwrap(), b"png");
        let again = files.fetch(&phone, &format!("{share}/DCIM/album"), &downloads).await.unwrap();
        assert_eq!(again, downloads.join("album (2)"));

        // A big copy, stopped once it's started: Stopped, and no half file on the phone.
        std::fs::write(local.join("big.bin"), vec![7u8; 64 << 20]).unwrap();
        let big = local.join("big.bin");
        let copying = tokio::spawn({
            let (files, phone, to) = (files.clone(), phone.clone(), format!("{share}/DCIM"));
            async move { files.put(&phone, &to, &[big]).await }
        });
        let started = async {
            loop {
                let p = files.progress("p");
                if p["done"].as_u64().unwrap_or(0) > 0 {
                    return p;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        };
        let p = tokio::time::timeout(Duration::from_secs(10), started).await.expect("the copy shows its progress");
        assert_eq!((p["name"].as_str(), p["total"].as_u64()), (Some("big.bin"), Some(64 << 20)));
        files.stop("p");
        let e = copying.await.unwrap().unwrap_err();
        assert_eq!(format!("{e}"), STOPPED);
        assert!(!dir.join("share/DCIM/big.bin").exists(), "the half file is removed");
        assert!(files.progress("p").is_null());

        files.new_folder(&phone, &format!("{share}/Music")).await.unwrap();
        assert!(files.new_folder(&phone, &format!("{share}/Music")).await.is_err(), "already there");
        let renamed = files.rename(&phone, &format!("{share}/DCIM/notes (2).txt"), "plans.txt").await.unwrap();
        assert_eq!(renamed, format!("{share}/DCIM/plans.txt"));
        assert_eq!(std::fs::read(dir.join("share/DCIM/plans.txt")).unwrap(), b"hello phone");
        assert!(files.rename(&phone, &format!("{share}/DCIM/plans.txt"), "notes.txt").await.is_err(), "never over a file");
        assert!(files.rename(&phone, &format!("{share}/DCIM/plans.txt"), "a/b").await.is_err(), "no slashes");
        assert!(files.rename(&phone, &share, "Elsewhere").await.is_err(), "not a top folder");
        files.rename(&phone, &format!("{share}/Music"), "Songs").await.unwrap();
        assert!(dir.join("share/Songs").is_dir(), "folders too");
        files.delete(&phone, &format!("{share}/DCIM/album")).await.unwrap();
        assert!(!dir.join("share/DCIM/album").exists(), "a folder with what's in it");
        assert!(files.delete(&phone, &share).await.is_err(), "not a top folder");
        assert!(files.list(&phone, Some(&format!("{share}/../.."))).await.is_err(), "no way above");

        // The connection dies without closing (the session's sshd frozen: the socket stays open,
        // nothing answers): the request fails, the check drops the session, and the next request
        // connects again.
        // sshd-session and its privilege-separated child: every process under the listener.
        let mut session: Vec<String> = Vec::new();
        let mut parents = vec![_sshd.child.id().to_string()];
        while let Some(pid) = parents.pop() {
            let children = std::fs::read_to_string(format!("/proc/{pid}/task/{pid}/children")).unwrap_or_default();
            for child in children.split_whitespace() {
                session.push(child.to_owned());
                parents.push(child.to_owned());
            }
        }
        assert!(!session.is_empty(), "sshd serves the session in a child");
        files.sftp_timeout("p", 2).await;
        for pid in &session {
            run(Run::new("kill").args(["-STOP", pid]));
        }
        assert!(files.list(&phone, None).await.is_err(), "a frozen server fails the request");
        files.recover("p").await;
        for pid in &session {
            run(Run::new("kill").args(["-KILL", pid]));
        }
        let back = tokio::time::timeout(Duration::from_secs(20), files.list(&phone, None)).await.expect("connects again").unwrap();
        assert_eq!(back["path"], share.as_str());

        // Someone else's server on the phone's port: a host key that isn't the phone's.
        files.forget("p");
        let stranger = Phone { cert: other, ..phone.clone() };
        let e = files.list(&stranger, None).await.unwrap_err();
        assert!(format!("{e:#}").contains("Couldn't open Pixel's files"), "{e:#}");
    }
}
