//! Browse, fetch and upload the PC's files from the phone. Only the user's home folder is reachable,
//! and nothing hidden in it: every path is resolved (`..` and symlinks included) and must still be
//! inside home with no dot-named part, which keeps out `.ssh`, keys, and `.config/autostart` (a file
//! put there would run at the next login). On a Mac, `~/Library` is off limits too: Mail, Messages,
//! keychains and browsers keep their data there. Paths on the wire are relative to home ("" is
//! home itself), so the phone never learns the absolute layout.
//!
//! What a desktop says around it (a phone reading files, a file arrived) and where sends land are
//! its own: it [set_hooks] once ([Hooks]).

use std::net::IpAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;

use crate::identity::Identity;
use crate::link::{self, Sender};
use crate::payload;

/// What a desktop does around the phone's file traffic.
pub trait Hooks: Send + Sync {
    /// [phone] is copying [name] from this computer (said once per stretch of reading).
    fn reading(&self, phone: &str, name: &str);
    /// [path] arrived from [phone]: a file, or a folder of [count] files.
    fn received(&self, phone: &str, path: &Path, count: Option<u64>);
    /// Where phone [phone_id]'s sends land.
    fn incoming_dir(&self, phone_id: &str) -> PathBuf;
}

/// Says nothing; sends land in ~/Downloads.
struct Quiet;

impl Hooks for Quiet {
    fn reading(&self, _: &str, _: &str) {}
    fn received(&self, _: &str, _: &Path, _: Option<u64>) {}
    fn incoming_dir(&self, _: &str) -> PathBuf {
        home().unwrap_or_else(|_| PathBuf::from("/tmp")).join("Downloads")
    }
}

static HOOKS: std::sync::OnceLock<Box<dyn Hooks>> = std::sync::OnceLock::new();

/// This desktop's [Hooks]; the first call wins.
pub fn set_hooks(hooks: Box<dyn Hooks>) {
    let _ = HOOKS.set(hooks);
}

pub(crate) fn hooks() -> &'static dyn Hooks {
    HOOKS.get_or_init(|| Box::new(Quiet)).as_ref()
}

/// Plenty for a phone list; a folder with more is cut short (and says so).
const MAX_ENTRIES: usize = 5000;
const TELL_EVERY: Duration = Duration::from_secs(30);
static LAST_TOLD: Mutex<Option<Instant>> = Mutex::new(None);

pub async fn list(sender: &Sender, rel: &str) -> Result<()> {
    let body = match read_dir(rel).await {
        Ok((entries, truncated)) => json!({ "path": rel, "entries": entries, "truncated": truncated }),
        Err(e) => json!({ "path": rel, "error": format!("{e:#}") }),
    };
    sender.send(&link::packet("vyoma.files.listing", body)).await
}

pub async fn send(sender: &Sender, me: &Identity, pinned: &[u8], rel: &str, phone_name: &str) -> Result<()> {
    let opened = async {
        let path = resolve(rel).await?;
        let file = tokio::fs::File::open(&path).await.context("couldn't open the file")?;
        let meta = file.metadata().await?;
        if !meta.is_file() {
            bail!("not a file");
        }
        Ok::<_, anyhow::Error>((path, file, meta.len()))
    }
    .await;
    let (path, file, size) = match opened {
        Ok(o) => o,
        Err(e) => {
            sender.send(&link::packet("vyoma.files.file", json!({ "path": rel, "error": format!("{e:#}") }))).await?;
            return Err(e);
        }
    };
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if should_tell(&LAST_TOLD, Instant::now()) {
        hooks().reading(phone_name, &name);
    }
    let announce = link::packet("vyoma.files.file", json!({ "path": rel, "name": name }));
    payload::offer(sender, me, pinned, announce, size, tokio::io::BufReader::with_capacity(256 * 1024, file)).await
}

/// One notice per stretch of reading, so a folder's worth of files doesn't fill the screen.
fn should_tell(last: &Mutex<Option<Instant>>, now: Instant) -> bool {
    let mut last = last.lock().unwrap();
    let due = last.is_none_or(|t| now.duration_since(t) >= TELL_EVERY);
    if due {
        *last = Some(now);
    }
    due
}

/// Saves a file the phone uploads into a home folder. It's written beside the target under a
/// hidden name and only renamed into place once complete, and never replaces an existing file.
pub async fn receive(sender: &Sender, me: &Identity, pinned: &[u8], phone: IpAddr, packet: &Value, phone_name: &str) -> Result<()> {
    let folder = packet["body"]["path"].as_str().unwrap_or_default().to_string();
    let wanted = packet["body"]["name"].as_str().unwrap_or_default().to_string();
    let result = async {
        let dir = resolve(&folder).await?;
        if !tokio::fs::metadata(&dir).await?.is_dir() {
            bail!("not a folder");
        }
        let name = safe_name(&wanted)?;
        let mut input = payload::fetch(me, pinned, phone, packet).await?;
        // Its own name: two uploads of one name at once. Removed however this ends, an aborted task too.
        let part = PartFile(dir.join(format!(".{name}.{}.vyoma-part", NEXT_PART.fetch_add(1, std::sync::atomic::Ordering::Relaxed))));
        let mut out = tokio::fs::File::create(&part.0).await.context("can't write in this folder")?;
        let copied = tokio::io::copy(&mut input, &mut out).await;
        out.flush().await?;
        drop(out);
        let expected = packet["payloadSize"].as_u64().unwrap_or(0);
        match copied {
            Ok(n) if n == expected => {}
            other => {
                bail!("the transfer was interrupted ({})", other.map(|n| format!("{n} of {expected} bytes")).unwrap_or_else(|e| e.to_string()));
            }
        }
        let target = settle(&part.0, &dir, &name).await?;
        hooks().received(phone_name, &target, None);
        Ok::<_, anyhow::Error>(target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(name))
    }
    .await;
    let body = match &result {
        Ok(saved) => json!({ "path": folder, "name": saved }),
        Err(e) => json!({ "path": folder, "name": wanted, "error": format!("{e:#}") }),
    };
    sender.send(&link::packet("vyoma.files.saved", body)).await?;
    result.map(|_| ())
}

/// A plain file name: no folders, not hidden, not empty.
fn safe_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.starts_with('.') || name.contains(['/', '\\', '\0']) {
        bail!("that file name can't be used here");
    }
    Ok(name.to_string())
}

static NEXT_PART: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

struct PartFile(PathBuf);

impl Drop for PartFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0); // already moved to its place, normally
    }
}

/// Moves a finished [part] to a free name in [dir], never over a file: a link fails if the name was
/// taken since it was checked, which a rename wouldn't.
pub(crate) async fn settle(part: &Path, dir: &Path, name: &str) -> Result<PathBuf> {
    loop {
        let target = free_name(dir, name).await;
        match tokio::fs::hard_link(part, &target).await {
            Ok(()) => {
                let _ = tokio::fs::remove_file(part).await;
                return Ok(target);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => {
                // A filesystem without hard links.
                tokio::fs::rename(part, &target).await?;
                return Ok(target);
            }
        }
    }
}

/// `name`, or `name (2).ext`, `name (3).ext`… whichever doesn't exist yet.
pub(crate) async fn free_name(dir: &Path, name: &str) -> PathBuf {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
        _ => (name, String::new()),
    };
    let mut candidate = dir.join(name);
    let mut n = 2;
    while tokio::fs::try_exists(&candidate).await.unwrap_or(true) {
        candidate = dir.join(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    candidate
}

async fn read_dir(rel: &str) -> Result<(Vec<Value>, bool)> {
    let dir = resolve(rel).await?;
    let mut rd = tokio::fs::read_dir(&dir).await.context("couldn't open the folder")?;
    let mut entries = Vec::new();
    let mut truncated = false;
    while let Some(e) = rd.next_entry().await? {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if entries.len() == MAX_ENTRIES {
            truncated = true;
            break;
        }
        // Follows symlinks, so a link to a folder browses like one. Broken links are skipped.
        let Ok(meta) = tokio::fs::metadata(e.path()).await else { continue };
        let modified = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as u64);
        entries.push((meta.is_dir(), name, meta.len(), modified));
    }
    // Folders first, then by name the way a file manager sorts (case-insensitive).
    entries.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase())));
    let json = entries
        .into_iter()
        .map(|(dir, name, size, modified)| json!({ "name": name, "dir": dir, "size": if dir { 0 } else { size }, "modified": modified }))
        .collect();
    Ok((json, truncated))
}

/// Home-relative path to a real path that is guaranteed to be inside home and not hidden.
async fn resolve(rel: &str) -> Result<PathBuf> {
    resolve_in(&tokio::fs::canonicalize(home()?).await?, rel).await
}

async fn resolve_in(home: &Path, rel: &str) -> Result<PathBuf> {
    let rel = Path::new(rel);
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        bail!("not a path inside your home folder");
    }
    if hidden(rel) {
        bail!("hidden files and folders can't be reached from the phone");
    }
    if off_limits(rel, cfg!(target_os = "macos")) {
        bail!("Library holds your apps' data, so it can't be reached from the phone");
    }
    let real = tokio::fs::canonicalize(home.join(rel)).await.context("no such file or folder")?;
    let Ok(inside) = real.strip_prefix(home) else {
        bail!("that's outside your home folder");
    };
    // A plain-named link can lead into a hidden folder.
    if hidden(inside) {
        bail!("hidden files and folders can't be reached from the phone");
    }
    if off_limits(inside, cfg!(target_os = "macos")) {
        bail!("Library holds your apps' data, so it can't be reached from the phone");
    }
    Ok(real)
}

/// On a Mac, home's `Library` (any case: its disk usually ignores case).
fn off_limits(inside: &Path, mac: bool) -> bool {
    mac && inside.components().next().is_some_and(|c| c.as_os_str().eq_ignore_ascii_case("library"))
}

fn hidden(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str().as_encoded_bytes().starts_with(b"."))
}

fn home() -> Result<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).context("HOME isn't set")
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn a_finished_upload_never_lands_on_a_file() {
        let dir = std::env::temp_dir().join(format!("vyoma-settle-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let (taken, part) = (dir.join("a.txt"), dir.join(".a.txt.0.vyoma-part"));
        tokio::fs::write(&taken, "old").await.unwrap();
        tokio::fs::write(&part, "new").await.unwrap();
        let saved = super::settle(&part, &dir, "a.txt").await.unwrap();
        assert_eq!(saved, dir.join("a (2).txt"));
        assert_eq!(tokio::fs::read_to_string(&taken).await.unwrap(), "old");
        assert_eq!(tokio::fs::read_to_string(&saved).await.unwrap(), "new");
        assert!(!part.exists());
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    use super::*;

    #[test]
    fn a_burst_of_reads_tells_once() {
        let last = Mutex::new(None);
        let start = Instant::now();
        assert!(should_tell(&last, start));
        assert!(!should_tell(&last, start + Duration::from_secs(29)));
        assert!(should_tell(&last, start + TELL_EVERY));
    }

    #[tokio::test]
    async fn only_plain_paths_inside_home_resolve() {
        let root = std::env::temp_dir().join(format!("vyoma-files-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let home = root.join("home");
        std::fs::create_dir_all(home.join("Documents/.git")).unwrap();
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        std::fs::create_dir_all(home.join(".config/autostart")).unwrap();
        std::fs::write(home.join("Documents/a.txt"), "a").unwrap();
        std::fs::write(home.join("Documents/.git/config"), "c").unwrap();
        std::fs::write(home.join(".ssh/id_ed25519"), "k").unwrap();
        std::fs::write(root.join("outside.txt"), "o").unwrap();
        std::os::unix::fs::symlink(home.join(".ssh"), home.join("keys")).unwrap();
        std::os::unix::fs::symlink(root.join("outside.txt"), home.join("out")).unwrap();
        let home = std::fs::canonicalize(&home).unwrap();

        for fine in ["", "Documents", "Documents/a.txt"] {
            assert!(resolve_in(&home, fine).await.is_ok(), "{fine}");
        }
        for refused in [".ssh", ".ssh/id_ed25519", ".config/autostart", "Documents/.git/config", "Documents/./.git", "keys", "keys/id_ed25519", "out", "../outside.txt", "/etc/passwd", "Documents/../.ssh"] {
            assert!(resolve_in(&home, refused).await.is_err(), "{refused}");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_macs_library_is_off_limits() {
        for refused in ["Library", "Library/Mail", "library/Keychains", "LIBRARY"] {
            assert!(off_limits(Path::new(refused), true), "{refused}");
        }
        assert!(!off_limits(Path::new("Documents/Library"), true));
        assert!(!off_limits(Path::new("Library"), false));
    }
}
