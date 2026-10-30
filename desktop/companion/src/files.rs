//! Browse, fetch and upload the PC's files from the phone. Only the user's home folder is reachable:
//! every path is resolved (`..` and symlinks included) and must still be inside it. Paths on the
//! wire are relative to home ("" is home itself), so the phone never learns the absolute layout.

use std::net::IpAddr;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;

use crate::identity::Identity;
use crate::link::{self, Sender};
use crate::payload;

/// Plenty for a phone list; a folder with more is cut short (and says so).
const MAX_ENTRIES: usize = 5000;

pub async fn list(sender: &Sender, rel: &str) -> Result<()> {
    let body = match read_dir(rel).await {
        Ok((entries, truncated)) => json!({ "path": rel, "entries": entries, "truncated": truncated }),
        Err(e) => json!({ "path": rel, "error": format!("{e:#}") }),
    };
    sender.send(&link::packet("vyoma.files.listing", body)).await
}

pub async fn send(sender: &Sender, me: &Identity, pinned: &[u8], rel: &str) -> Result<()> {
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
    let announce = link::packet("vyoma.files.file", json!({ "path": rel, "name": name }));
    payload::offer(sender, me, pinned, announce, size, tokio::io::BufReader::with_capacity(256 * 1024, file)).await
}

/// Saves a file the phone uploads into a home folder. It's written beside the target under a
/// hidden name and only renamed into place once complete, and never replaces an existing file.
pub async fn receive(sender: &Sender, me: &Identity, pinned: &[u8], phone: IpAddr, packet: &Value) -> Result<()> {
    let folder = packet["body"]["path"].as_str().unwrap_or_default().to_string();
    let wanted = packet["body"]["name"].as_str().unwrap_or_default().to_string();
    let result = async {
        let dir = resolve(&folder).await?;
        if !tokio::fs::metadata(&dir).await?.is_dir() {
            bail!("not a folder");
        }
        let name = safe_name(&wanted)?;
        let mut input = payload::fetch(me, pinned, phone, packet).await?;
        let part = dir.join(format!(".{name}.vyoma-part"));
        let mut out = tokio::fs::File::create(&part).await.context("can't write in this folder")?;
        let copied = tokio::io::copy(&mut input, &mut out).await;
        out.flush().await?;
        drop(out);
        let expected = packet["payloadSize"].as_u64().unwrap_or(0);
        match copied {
            Ok(n) if n == expected => {}
            other => {
                let _ = tokio::fs::remove_file(&part).await;
                bail!("the transfer was interrupted ({})", other.map(|n| format!("{n} of {expected} bytes")).unwrap_or_else(|e| e.to_string()));
            }
        }
        let target = free_name(&dir, &name).await;
        tokio::fs::rename(&part, &target).await?;
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

/// Home-relative path to a real path that is guaranteed to be inside home.
async fn resolve(rel: &str) -> Result<PathBuf> {
    let home = tokio::fs::canonicalize(home()?).await?;
    let rel = Path::new(rel);
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        bail!("not a path inside your home folder");
    }
    let real = tokio::fs::canonicalize(home.join(rel)).await.context("no such file or folder")?;
    if !real.starts_with(&home) {
        bail!("that's outside your home folder");
    }
    Ok(real)
}

fn home() -> Result<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).context("HOME isn't set")
}
