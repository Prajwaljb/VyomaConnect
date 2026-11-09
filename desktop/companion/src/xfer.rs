//! Files and folders sent from the phone, resumable. They land where kdeconnectd puts what it
//! receives (its `incoming_path`, else the Downloads folder), with the same "received" notification.
//!
//! `vyoma.xfer.begin {key, name, size, folder?, batch?}` → `vyoma.xfer.ready {key, offset}`: how much
//! of it is already here from an attempt that broke off (0 for a new one). Then
//! `vyoma.xfer.data {key, offset}` + payload of the rest → `vyoma.xfer.done {key, name | error}`.
//! A file is written as `.name.vyoma-part` (its key beside it in `.name.vyoma-key`) and renamed
//! into place only when complete, never over an existing file. Files of a folder carry `folder`
//! ("Photos/2024") and a `batch` id: the folder's top gets a free name once per batch, so a second
//! send of "Photos" makes "Photos (2)" instead of mixing into the first, while a resumed batch
//! keeps writing into its own. `vyoma.xfer.end {batch, count}` closes a folder (one notification).

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

use crate::identity::Identity;
use crate::link::{self, Sender};
use crate::payload;

/// A file between `begin` and `done`, by key. Process-wide, so a phone that drops and reconnects
/// (a new session) resumes into the same file.
struct Upload {
    dir: PathBuf,
    name: String,
    size: u64,
    /// Part of a folder: no notification of its own.
    in_folder: bool,
}

static UPLOADS: Mutex<Option<HashMap<String, Upload>>> = Mutex::new(None);
/// Each folder batch's top folder, as created here.
static BATCHES: Mutex<Option<HashMap<String, PathBuf>>> = Mutex::new(None);

/// Parts nobody came back for are cleared after this long (a send abandoned for good).
const STALE_PART: Duration = Duration::from_secs(24 * 3600);

pub async fn begin(sender: &Sender, phone_id: &str, body: &Value) -> Result<()> {
    let key = body["key"].as_str().unwrap_or_default().to_string();
    let reply = match prepare(phone_id, &key, body).await {
        Ok(offset) => json!({ "key": key, "offset": offset }),
        Err(e) => json!({ "key": key, "error": format!("{e:#}") }),
    };
    sender.send(&link::packet("vyoma.xfer.ready", reply)).await
}

async fn prepare(phone_id: &str, key: &str, body: &Value) -> Result<u64> {
    if key.is_empty() || key.len() > 64 || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        bail!("bad transfer key");
    }
    let size = body["size"].as_u64().context("no size")?;
    let folder = body["folder"].as_str().filter(|f| !f.is_empty());
    let name = safe_name(body["name"].as_str().unwrap_or_default(), folder.is_some())?;
    let base = incoming_dir(phone_id);
    tokio::fs::create_dir_all(&base).await.context("can't create the downloads folder")?;
    let dir = match folder {
        None => base.clone(),
        Some(f) => folder_dir(&base, f, body["batch"].as_str().unwrap_or(key)).await?,
    };
    clear_stale(&dir).await;
    let (part, keyfile) = part_paths(&dir, &name);
    // Same key as the part here: carry on from where it stopped. Anything else starts over.
    let resumable = tokio::fs::read_to_string(&keyfile).await.is_ok_and(|k| k.trim() == key);
    let offset = match tokio::fs::metadata(&part).await {
        Ok(m) if resumable => m.len().min(size),
        _ => {
            let _ = tokio::fs::remove_file(&part).await;
            tokio::fs::write(&keyfile, key).await.context("can't write in the downloads folder")?;
            0
        }
    };
    UPLOADS.lock().unwrap().get_or_insert_with(HashMap::new).insert(key.to_string(), Upload { dir, name, size, in_folder: folder.is_some() });
    Ok(offset)
}

/// The payload of one `vyoma.xfer.data`: the file from `offset` to its end.
/// The saved file's name, unless it's part of a folder (whose [end] says it).
pub async fn data(sender: &Sender, me: &Identity, pinned: &[u8], phone: IpAddr, phone_name: &str, packet: &Value) -> Result<Option<String>> {
    let key = packet["body"]["key"].as_str().unwrap_or_default().to_string();
    let result = receive(me, pinned, phone, &key, packet).await;
    let reply = match &result {
        Ok(saved) => json!({ "key": key, "name": saved.file_name().map(|n| n.to_string_lossy().into_owned()) }),
        Err(e) => json!({ "key": key, "error": format!("{e:#}") }),
    };
    sender.send(&link::packet("vyoma.xfer.done", reply)).await?;
    let saved = result?;
    let in_folder = UPLOADS.lock().unwrap().as_mut().and_then(|u| u.remove(&key)).is_some_and(|u| u.in_folder);
    if in_folder {
        return Ok(None);
    }
    notify(phone_name, &saved, None);
    Ok(saved.file_name().map(|n| n.to_string_lossy().into_owned()))
}

async fn receive(me: &Identity, pinned: &[u8], phone: IpAddr, key: &str, packet: &Value) -> Result<PathBuf> {
    let (dir, name, size) = {
        let uploads = UPLOADS.lock().unwrap();
        let u = uploads.as_ref().and_then(|u| u.get(key)).context("unknown transfer; start it again")?;
        (u.dir.clone(), u.name.clone(), u.size)
    };
    let offset = packet["body"]["offset"].as_u64().unwrap_or(0);
    let (part, keyfile) = part_paths(&dir, &name);
    let mut out = tokio::fs::OpenOptions::new().create(true).write(true).truncate(false).open(&part).await.context("can't write in the downloads folder")?;
    if out.metadata().await?.len() < offset {
        bail!("the PC has less of it than the phone thinks; start it again");
    }
    // Anything past the offset is from a write cut off mid-way: overwritten by this attempt.
    out.set_len(offset).await?;
    out.seek(std::io::SeekFrom::Start(offset)).await?;
    let mut input = payload::fetch(me, pinned, phone, packet).await?;
    let copied = tokio::io::copy(&mut input, &mut out).await;
    out.flush().await?;
    out.sync_data().await?;
    drop(out);
    let have = tokio::fs::metadata(&part).await?.len();
    if have != size {
        bail!("the transfer broke off at {have} of {size} bytes{}", copied.err().map(|e| format!(" ({e})")).unwrap_or_default());
    }
    let target = crate::files::settle(&part, &dir, &name).await?;
    let _ = tokio::fs::remove_file(&keyfile).await;
    Ok(target)
}

/// A folder batch is done: one notification for it. Its folder's name and how many files.
pub fn end(phone_name: &str, body: &Value) -> Option<(String, u64)> {
    let batch = body["batch"].as_str()?;
    let root = BATCHES.lock().unwrap().as_mut().and_then(|b| b.remove(batch))?;
    notify(phone_name, &root, body["count"].as_u64());
    Some((root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), body["count"].as_u64().unwrap_or(0)))
}

/// The folder for `rel` ("Photos/2024") under `base`; its top is named once per batch.
async fn folder_dir(base: &Path, rel: &str, batch: &str) -> Result<PathBuf> {
    let mut parts = Path::new(rel).components().map(|c| match c {
        Component::Normal(s) => safe_name(&s.to_string_lossy(), true),
        _ => bail!("that folder name can't be used here"),
    });
    let top = parts.next().context("empty folder")??;
    let existing = BATCHES.lock().unwrap().as_ref().and_then(|b| b.get(batch).cloned());
    let root = match existing {
        Some(r) => r,
        None => {
            let r = crate::files::free_name(base, &top).await;
            tokio::fs::create_dir_all(&r).await.context("can't create the folder")?;
            BATCHES.lock().unwrap().get_or_insert_with(HashMap::new).insert(batch.to_string(), r.clone());
            r
        }
    };
    let mut dir = root;
    for p in parts {
        dir.push(p?);
    }
    tokio::fs::create_dir_all(&dir).await.context("can't create the folder")?;
    Ok(dir)
}

fn part_paths(dir: &Path, name: &str) -> (PathBuf, PathBuf) {
    (dir.join(format!(".{name}.vyoma-part")), dir.join(format!(".{name}.vyoma-key")))
}

/// Removes parts (and their keys) nobody has touched for a day.
async fn clear_stale(dir: &Path) {
    let Ok(mut rd) = tokio::fs::read_dir(dir).await else { return };
    while let Ok(Some(e)) = rd.next_entry().await {
        let n = e.file_name().to_string_lossy().into_owned();
        if !(n.starts_with('.') && (n.ends_with(".vyoma-part") || n.ends_with(".vyoma-key"))) {
            continue;
        }
        let old = e.metadata().await.ok().and_then(|m| m.modified().ok()).and_then(|t| SystemTime::now().duration_since(t).ok()).is_some_and(|age| age > STALE_PART);
        if old {
            let _ = tokio::fs::remove_file(e.path()).await;
        }
    }
}

/// One file or folder name: no separators, not `.`/`..`. Hidden names only inside a sent
/// folder (a project's `.gitignore`), never loose in Downloads.
fn safe_name(name: &str, hidden_ok: bool) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']) || (!hidden_ok && name.starts_with('.')) || name.len() > 255 {
        bail!("that name can't be used here");
    }
    Ok(name.to_string())
}

/// Where kdeconnectd saves what this phone sends: its share plugin's `incoming_path` if set
/// (`%1` is the phone's name there; we use the folder as is), else XDG Downloads. The desktop app
/// saves its stock shares to Downloads, so these go there too, whatever kdeconnectd's old setting.
fn incoming_dir(phone_id: &str) -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
    if crate::setup::is_standalone() {
        return downloads(&home);
    }
    let config = home.join(".config/kdeconnect").join(phone_id).join("kdeconnect_share/config");
    if let Ok(text) = std::fs::read_to_string(config)
        && let Some(v) = text.lines().find_map(|l| l.strip_prefix("incoming_path="))
    {
        let v = v.trim().trim_start_matches("file://").replace("/%1", "");
        if v.starts_with('/') {
            return PathBuf::from(v);
        }
    }
    downloads(&home)
}

fn downloads(home: &Path) -> PathBuf {
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

/// "Received X from phone", like kdeconnectd's, with Open and Show in folder. Runs on its own:
/// notify-send waits for the click.
pub(crate) fn notify(phone_name: &str, path: &Path, count: Option<u64>) {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let body = match count {
        Some(n) => format!("{name} ({n} file{}) from {phone_name}", if n == 1 { "" } else { "s" }),
        None => format!("{name} from {phone_name}"),
    };
    let path = path.to_path_buf();
    tokio::spawn(async move {
        let out = tokio::process::Command::new("notify-send")
            .args(["--app-name=Vyoma", "--hint=string:desktop-entry:vyoma", "--icon=document-save", "--action=open=Open", "--action=folder=Show in folder", "--", "Received"])
            .arg(crate::notice::escape(&body))
            .output()
            .await;
        let Ok(out) = out else { return };
        let target = match String::from_utf8_lossy(&out.stdout).trim() {
            "open" => path,
            "folder" => path.parent().map(Path::to_path_buf).unwrap_or(path),
            _ => return,
        };
        let _ = tokio::process::Command::new("xdg-open").arg(target).spawn();
    });
}
