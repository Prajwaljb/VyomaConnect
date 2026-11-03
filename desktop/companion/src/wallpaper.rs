//! The desktop's wallpaper as a small grid of colours, for the phone's background:
//! `vyoma.wallpaper.request` → `vyoma.wallpaper {cols, rows, colors: [[r, g, b], ...]}` (row-major,
//! top-left first) or `{error}`. ffmpeg averages the image down to the grid; the phone stretches it
//! smoothly, so it reads as the wallpaper's light, heavily blurred.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};

pub const COLS: usize = 4;
pub const ROWS: usize = 4;

pub async fn colors() -> Result<Vec<[u8; 3]>> {
    let path = current().context("couldn't find Plasma's wallpaper image")?;
    let out = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::process::Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-i"])
            .arg(&path)
            .args(["-frames:v", "1", "-vf", &format!("scale={COLS}:{ROWS}:flags=area"), "-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1"])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .context("ffmpeg took too long")??;
    if !out.status.success() || out.stdout.len() < COLS * ROWS * 3 {
        bail!("ffmpeg couldn't read {}", path.display());
    }
    Ok(out.stdout.chunks_exact(3).take(COLS * ROWS).map(|c| [c[0], c[1], c[2]]).collect())
}

/// The image Plasma's desktop shows (the first screen's), from its desktop config. A wallpaper
/// package resolves to its largest image; no image set means Plasma's default, Next.
fn current() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let config = Path::new(&home).join(".config/plasma-org.kde.plasma.desktop-appletsrc");
    let image = std::fs::read_to_string(config).ok().and_then(|text| image_setting(&text));
    let raw = image.unwrap_or_else(|| "/usr/share/wallpapers/Next/".into());
    let path = PathBuf::from(unescape(raw.strip_prefix("file://").unwrap_or(&raw)));
    resolve(&path)
}

/// `Image=` under the first `[...][Wallpaper][org.kde.image][General]` group.
fn image_setting(text: &str) -> Option<String> {
    let mut in_image = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_image = line.ends_with("[Wallpaper][org.kde.image][General]");
        } else if in_image && let Some(v) = line.strip_prefix("Image=") {
            return Some(v.to_string());
        }
    }
    None
}

fn resolve(p: &Path) -> Option<PathBuf> {
    if p.is_file() {
        return Some(p.to_path_buf());
    }
    // A wallpaper package: contents/images/<w>x<h>.<ext>, one per size.
    std::fs::read_dir(p.join("contents/images"))
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|f| f.is_file())
        .max_by_key(|f| f.metadata().map(|m| m.len()).unwrap_or(0))
}

/// A file URL's %XX escapes (spaces in a path).
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_image_setting() {
        let text = "[Containments][1][General]\nImage=nope\n[Containments][1][Wallpaper][org.kde.image][General]\nImage=file:///home/me/My%20Pics/a.jpg\n";
        assert_eq!(image_setting(text).as_deref(), Some("file:///home/me/My%20Pics/a.jpg"));
        assert_eq!(unescape("/home/me/My%20Pics/a.jpg"), "/home/me/My Pics/a.jpg");
        assert_eq!(unescape("/a%2"), "/a%2");
    }

    #[tokio::test]
    async fn reads_this_pcs_wallpaper() {
        // Only meaningful on a Plasma desktop with ffmpeg; elsewhere it just reports why not.
        match colors().await {
            Ok(c) => assert_eq!(c.len(), COLS * ROWS),
            Err(e) => eprintln!("no wallpaper here: {e:#}"),
        }
    }
}
