//! Sharing, as the stock plugin does it: files with their payloads, text and links, both ways.
//!
//! A batch of files is announced with `kdeconnect.share.request.update {numberOfFiles,
//! totalPayloadSize}`, then each file is a `kdeconnect.share.request {filename, …}` with a payload;
//! an empty file comes with no payload at all. What arrives lands in XDG Downloads under a free
//! name, never over a file.

use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use vyoma_core::device::Command;
use vyoma_core::identity::Identity;
use vyoma_core::link::packet;
use vyoma_core::payload::{self, Offer};

pub const SHARE: &str = "kdeconnect.share.request";
pub const SHARE_UPDATE: &str = "kdeconnect.share.request.update";

/// What a share packet holds.
#[derive(Debug, PartialEq)]
pub enum Shared {
    /// A file, fetched with [receive]; [count]: how many the batch has.
    File { name: String, count: u64 },
    Text(String),
    /// A web link, opened at once as the stock PC does.
    Url(String),
    Nothing,
}

pub fn classify(p: &Value) -> Shared {
    let body = &p["body"];
    if let Some(name) = body["filename"].as_str() {
        return Shared::File { name: safe_name(name), count: body["numberOfFiles"].as_u64().unwrap_or(1).max(1) };
    }
    if let Some(url) = body["url"].as_str() {
        // Only the web opens unasked: another scheme could start any app that claims it.
        let web = url.starts_with("https://") || url.starts_with("http://");
        return if web && !url.contains(char::is_whitespace) { Shared::Url(url.to_owned()) } else { Shared::Text(url.to_owned()) };
    }
    match body["text"].as_str() {
        Some(t) if !t.is_empty() => Shared::Text(t.to_owned()),
        _ => Shared::Nothing,
    }
}

/// A name from a device as a plain file name here: no folders, not hidden, not too long.
pub fn safe_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let clean: String = base.chars().filter(|c| !c.is_control()).collect();
    let mut clean = clean.trim().trim_start_matches('.').trim().to_owned();
    while clean.len() > 200 {
        clean.pop();
    }
    if clean.is_empty() { "file".into() } else { clean }
}

/// Fetches the file [p] announces from [from] into [dir]: where it landed.
pub async fn receive(me: &Identity, pinned: &[u8], from: std::net::IpAddr, p: &Value, dir: &Path) -> Result<PathBuf> {
    let name = safe_name(p["body"]["filename"].as_str().unwrap_or(""));
    tokio::fs::create_dir_all(dir).await.with_context(|| format!("couldn't make {}", dir.display()))?;
    let part = Part(dir.join(format!(".{name}.{}-{}.vyoma-part", std::process::id(), next())));
    let mut file = tokio::fs::OpenOptions::new().write(true).create_new(true).open(&part.0).await?;
    // -1: a size the sender doesn't know, read to the end. Only no payload at all is an empty file.
    let size = p["payloadSize"].as_i64().unwrap_or(0);
    if size != 0 && p["payloadTransferInfo"].is_object() {
        let got = if size > 0 {
            tokio::io::copy(&mut payload::fetch(me, pinned, from, p).await?, &mut file).await?
        } else {
            tokio::io::copy(&mut payload::fetch_stream(me, pinned, from, p).await?, &mut file).await?
        };
        if size > 0 && got != size as u64 {
            bail!("the device sent {got} of {size} bytes");
        }
        if size < 0 && got == 0 {
            bail!("the device sent nothing");
        }
    }
    file.flush().await?;
    let file = file.into_std().await;
    if let Some(ms) = p["body"]["lastModified"].as_u64().filter(|ms| *ms > 0) {
        let _ = file.set_modified(UNIX_EPOCH + Duration::from_millis(ms));
    }
    drop(file);
    settle(&part.0, dir, &name).await
}

fn next() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// A file being received; gone if it never settles.
struct Part(PathBuf);

impl Drop for Part {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Moves a finished [part] to a free name in [dir], never over a file: a link fails if the name was
/// taken since it was checked, which a rename wouldn't.
async fn settle(part: &Path, dir: &Path, name: &str) -> Result<PathBuf> {
    loop {
        let target = free_name(dir, name).await;
        match tokio::fs::hard_link(part, &target).await {
            Ok(()) => return Ok(target),
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
async fn free_name(dir: &Path, name: &str) -> PathBuf {
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

/// XDG Downloads.
pub fn downloads() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
    if let Ok(text) = std::fs::read_to_string(home.join(".config/user-dirs.dirs"))
        && let Some(v) = text.lines().find_map(|l| l.strip_prefix("XDG_DOWNLOAD_DIR="))
    {
        let v = v.trim().trim_matches('"').replace("$HOME", &home.to_string_lossy());
        if v.starts_with('/') {
            return PathBuf::from(v);
        }
    }
    home.join("Downloads")
}

/// A file to send, checked before anything goes out.
pub struct Outgoing {
    path: PathBuf,
    name: String,
    size: u64,
    modified: u64,
    /// The folder it's in when it goes as part of one ("Trip/day 1"), for the phone to rebuild.
    folder: Option<String>,
}

impl Outgoing {
    fn new(path: PathBuf, name: String, meta: &std::fs::Metadata, folder: Option<String>) -> Self {
        let modified = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as u64);
        Self { path, name, size: meta.len(), modified, folder }
    }
}

/// The most files one send takes: a folder bigger than that goes better as one zip.
const MAX_FILES: usize = 10_000;

/// What [paths] send: each file as it is, each folder as every file inside it, in order, each
/// with the folder it's in. Fails at once if one can't be read.
pub fn expand(paths: &[String]) -> Result<Vec<Outgoing>> {
    let mut out = Vec::new();
    for p in paths {
        let path = PathBuf::from(p);
        let meta = std::fs::metadata(&path).with_context(|| format!("can't read {}", path.display()))?;
        if meta.is_dir() {
            let top = path.file_name().map(|n| n.to_string_lossy().into_owned()).context("that folder has no name")?;
            walk(&path, &top, &mut out)?;
        } else if meta.is_file() {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).context("no file name")?;
            out.push(Outgoing::new(path, name, &meta, None));
        } else {
            bail!("{} isn't a file or a folder", path.display());
        }
    }
    Ok(out)
}

/// Adds every file under [dir], which goes as [folder]. Folders linked from inside aren't
/// followed (one could lead back up and never end); linked files are sent.
fn walk(dir: &Path, folder: &str, out: &mut Vec<Outgoing>) -> Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir).with_context(|| format!("can't read {}", dir.display()))?.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let Ok(kind) = e.file_type() else { continue };
        let name = e.file_name().to_string_lossy().into_owned();
        let path = e.path();
        if kind.is_dir() {
            walk(&path, &format!("{folder}/{name}"), out)?;
        } else if let Ok(meta) = std::fs::metadata(&path) {
            if meta.is_file() {
                out.push(Outgoing::new(path, name, &meta, Some(folder.to_owned())));
            }
        }
        if out.len() > MAX_FILES {
            bail!("{folder} has more than {MAX_FILES} files: zip it and send that instead");
        }
    }
    Ok(())
}

/// The send in a few words, for a notification: "photo.jpg", "Trip, 120 files" or "3 files".
pub fn summary(files: &[Outgoing]) -> String {
    let count = |n: usize| if n == 1 { "1 file".to_owned() } else { format!("{n} files") };
    let top = |f: &Outgoing| f.folder.as_deref().map(|d| d.split('/').next().unwrap_or(d).to_owned());
    match files {
        [one] if one.folder.is_none() => one.name.clone(),
        [first, ..] if files.iter().all(|f| f.folder.is_some() && top(f) == top(first)) => format!("{}, {}", top(first).unwrap_or_default(), count(files.len())),
        _ => count(files.len()),
    }
}

/// Sends [files] to [id], one after the other; each waits for the device to fetch it.
pub async fn send_files(commands: &mpsc::UnboundedSender<Command>, id: &str, me: &Identity, pinned: &[u8], files: Vec<Outgoing>) -> Result<()> {
    let count = files.len();
    let total: u64 = files.iter().map(|f| f.size).sum();
    let send = |p: Value| commands.send(Command::Send(id.to_owned(), p)).map_err(|_| anyhow::anyhow!("stopping"));
    send(packet(SHARE_UPDATE, json!({ "numberOfFiles": count, "totalPayloadSize": total })))?;
    for f in files {
        let mut body = json!({ "filename": f.name, "lastModified": f.modified, "open": false, "numberOfFiles": count, "totalPayloadSize": total });
        // Ours, which stock phones ignore (they save the files side by side).
        if let Some(folder) = &f.folder {
            body["folder"] = json!(folder);
        }
        if f.size == 0 {
            // As stock: an empty file is its name alone.
            send(packet(SHARE, body))?;
            continue;
        }
        offer_file(commands, id, me, pinned, packet(SHARE, body), &f.path, f.size).await.with_context(|| format!("{} wasn't sent", f.name))?;
    }
    Ok(())
}

/// Sends [packet] with the file at [path] as its payload, and waits for the device to fetch it.
pub async fn offer_file(commands: &mpsc::UnboundedSender<Command>, id: &str, me: &Identity, pinned: &[u8], packet: Value, path: &Path, size: u64) -> Result<()> {
    let source = tokio::fs::File::open(path).await.with_context(|| format!("can't read {}", path.display()))?;
    let mut offer = Offer::open().await?;
    commands.send(Command::Send(id.to_owned(), offer.announce(packet, size as i64)?)).map_err(|_| anyhow::anyhow!("stopping"))?;
    offer.serve(me, pinned, source).await
}

/// Text, or a link when it's one alone.
pub fn text_packet(text: &str) -> Value {
    let t = text.trim();
    let link = (t.starts_with("https://") || t.starts_with("http://")) && !t.contains(char::is_whitespace);
    if link { packet(SHARE, json!({ "url": t })) } else { packet(SHARE, json!({ "text": text })) }
}

/// Opens [target] (a file, folder or web link) in the user's app for it.
pub fn open(target: &str) {
    match std::process::Command::new("xdg-open").arg(target).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn() {
        // Reaped on its own thread: xdg-open waits on nothing, it hands over.
        Ok(mut child) => drop(std::thread::spawn(move || child.wait())),
        Err(e) => tracing::warn!("can't open it: no xdg-open ({e})"),
    }
}

/// Shows [path] selected in the file manager (Dolphin, Nautilus, Nemo and others take this call),
/// else opens its folder.
pub async fn show_in_folder(conn: &zbus::Connection, path: &Path) {
    let uri = format!("file://{}", percent(&path.to_string_lossy()));
    let shown = conn
        .call_method(Some("org.freedesktop.FileManager1"), "/org/freedesktop/FileManager1", Some("org.freedesktop.FileManager1"), "ShowItems", &(vec![uri], ""))
        .await;
    if shown.is_err()
        && let Some(dir) = path.parent()
    {
        open(&dir.to_string_lossy());
    }
}

/// A path as a URI's path: anything but unreserved characters and `/` escaped.
pub fn percent(path: &str) -> String {
    path.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_goes_as_its_files_with_where_each_was() {
        let root = std::env::temp_dir().join(format!("vyoma-expand-{}", std::process::id()));
        let trip = root.join("Trip");
        std::fs::create_dir_all(trip.join("day 1")).unwrap();
        std::fs::create_dir_all(trip.join("empty")).unwrap();
        std::fs::write(trip.join("notes.txt"), "hi").unwrap();
        std::fs::write(trip.join("day 1/a.jpg"), "jpg").unwrap();
        std::os::unix::fs::symlink(&root, trip.join("loop")).unwrap();
        std::fs::write(root.join("single.pdf"), "pdf").unwrap();

        let files = expand(&[trip.to_string_lossy().into_owned()]).unwrap();
        let got: Vec<_> = files.iter().map(|f| (f.folder.clone().unwrap(), f.name.clone())).collect();
        assert_eq!(got, [("Trip/day 1".to_owned(), "a.jpg".to_owned()), ("Trip".to_owned(), "notes.txt".to_owned())]);
        assert_eq!(summary(&files), "Trip, 2 files");

        let one = expand(&[root.join("single.pdf").to_string_lossy().into_owned()]).unwrap();
        assert_eq!(summary(&one), "single.pdf");
        let both = expand(&[root.join("single.pdf").to_string_lossy().into_owned(), trip.to_string_lossy().into_owned()]).unwrap();
        assert_eq!(summary(&both), "3 files");
        assert!(expand(&[root.join("missing").to_string_lossy().into_owned()]).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn names_from_devices_stay_in_the_folder() {
        assert_eq!(safe_name("photo.jpg"), "photo.jpg");
        assert_eq!(safe_name("../../.bashrc"), "bashrc");
        assert_eq!(safe_name("C:\\x\\a.txt"), "a.txt");
        assert_eq!(safe_name(".."), "file");
        assert_eq!(safe_name("a\nb\u{7}.txt"), "ab.txt");
        assert!(safe_name(&"é".repeat(300)).len() <= 200);
    }

    #[test]
    fn only_web_links_open() {
        let share = |body: Value| classify(&packet(SHARE, body));
        assert_eq!(share(json!({ "url": "https://kde.org" })), Shared::Url("https://kde.org".into()));
        assert_eq!(share(json!({ "url": "file:///etc/passwd" })), Shared::Text("file:///etc/passwd".into()));
        assert_eq!(share(json!({ "text": "hi" })), Shared::Text("hi".into()));
        assert_eq!(share(json!({ "filename": "a/b.txt", "numberOfFiles": 3 })), Shared::File { name: "b.txt".into(), count: 3 });
        assert_eq!(share(json!({})), Shared::Nothing);
        assert_eq!(text_packet(" https://kde.org\n")["body"], json!({ "url": "https://kde.org" }));
        assert_eq!(text_packet("see https://kde.org")["body"], json!({ "text": "see https://kde.org" }));
        assert_eq!(percent("/home/a b/ü#.txt"), "/home/a%20b/%C3%BC%23.txt");
    }

    #[tokio::test]
    async fn received_files_never_replace_one() {
        let dir = std::env::temp_dir().join(format!("vyoma-share-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "old").unwrap();
        std::fs::write(dir.join("part"), "new").unwrap();
        let got = settle(&dir.join("part"), &dir, "a.txt").await.unwrap();
        assert_eq!(got, dir.join("a (2).txt"));
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "old");
        assert_eq!(std::fs::read_to_string(got).unwrap(), "new");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
