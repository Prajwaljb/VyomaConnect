//! Browse and fetch the PC's files from the phone. Only the user's home folder is reachable:
//! every path is resolved (`..` and symlinks included) and must still be inside it. Paths on the
//! wire are relative to home ("" is home itself), so the phone never learns the absolute layout.

use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

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
