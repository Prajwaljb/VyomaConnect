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
    let path = current().context("couldn't find the desktop's wallpaper image")?;
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

/// What identifies the wallpaper as shown: its file and when the file and the desktop's setting
/// last changed. Cheap (a config read and two stats), so it can be checked every few seconds.
pub fn signature() -> Option<(PathBuf, Option<std::time::SystemTime>, Option<std::time::SystemTime>)> {
    let home = std::env::var_os("HOME")?;
    let path = current()?;
    let mtime = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    // GNOME's settings (read by [current], light or dark) name the picture, which says it all; its
    // settings file changes with every setting, so its time says nothing.
    if crate::desktop::is_gnome() || other_desktop().is_some() {
        return Some((path.clone(), mtime(&path), None));
    }
    let config = Path::new(&home).join(".config/plasma-org.kde.plasma.desktop-appletsrc");
    Some((path.clone(), mtime(&path), mtime(&config)))
}

/// The desktops beside GNOME and Plasma that keep their own wallpaper setting.
#[derive(Clone, Copy)]
enum Other {
    Cinnamon,
    Mate,
    Xfce,
}

fn other_desktop() -> Option<Other> {
    let desk = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase();
    if desk.contains("cinnamon") {
        Some(Other::Cinnamon)
    } else if desk.contains("mate") {
        Some(Other::Mate)
    } else if desk.contains("xfce") {
        Some(Other::Xfce)
    } else {
        None
    }
}

/// The image the desktop shows: GNOME's background (its dark one in dark mode), Cinnamon's, MATE's
/// or Xfce's, else Plasma's.
fn current() -> Option<PathBuf> {
    match other_desktop() {
        Some(Other::Cinnamon) => return from_uri(&gsetting("org.cinnamon.desktop.background", "picture-uri")?),
        Some(Other::Mate) => return resolve(Path::new(&gsetting("org.mate.background", "picture-filename")?)),
        Some(Other::Xfce) => return xfce(),
        None => {}
    }
    if crate::desktop::is_gnome() {
        return gnome();
    }
    plasma()
}

fn gsetting(schema: &str, key: &str) -> Option<String> {
    let out = std::process::Command::new("gsettings").args(["get", schema, key]).output().ok()?;
    let v = String::from_utf8_lossy(&out.stdout).trim().trim_matches('\'').to_owned();
    (out.status.success() && !v.is_empty()).then_some(v)
}

/// A `file://` URI (or a plain path) as an image file.
fn from_uri(uri: &str) -> Option<PathBuf> {
    resolve(&PathBuf::from(unescape(uri.strip_prefix("file://").unwrap_or(uri))))
}

/// Xfce's: the first monitor's first workspace's `last-image` in xfconf.
fn xfce() -> Option<PathBuf> {
    let list = std::process::Command::new("xfconf-query").args(["-c", "xfce4-desktop", "-l"]).output().ok()?;
    let mut props: Vec<String> = String::from_utf8_lossy(&list.stdout).lines().filter(|l| l.ends_with("/last-image")).map(str::to_owned).collect();
    props.sort_by_key(|p| (!p.contains("workspace0"), p.clone()));
    let prop = props.first()?;
    let out = std::process::Command::new("xfconf-query").args(["-c", "xfce4-desktop", "-p", prop]).output().ok()?;
    resolve(Path::new(String::from_utf8_lossy(&out.stdout).trim()))
}

/// GNOME's background from its settings. A slideshow (an XML file of images, which GNOME's own
/// day-and-night ones are) gives its first image.
fn gnome() -> Option<PathBuf> {
    let get = gsetting;
    let dark = get("org.gnome.desktop.interface", "color-scheme").is_some_and(|v| v == "prefer-dark");
    let uri = (if dark { get("org.gnome.desktop.background", "picture-uri-dark") } else { None }).or_else(|| get("org.gnome.desktop.background", "picture-uri"))?;
    let path = PathBuf::from(unescape(uri.strip_prefix("file://").unwrap_or(&uri)));
    if path.extension().is_some_and(|e| e == "xml") {
        return slideshow_first(&std::fs::read_to_string(&path).ok()?).map(PathBuf::from).filter(|p| p.is_file());
    }
    resolve(&path)
}

/// The first `<file>` of a GNOME slideshow, or of its `<static>` part.
fn slideshow_first(xml: &str) -> Option<String> {
    let start = xml.find("<file>")? + "<file>".len();
    let end = start + xml[start..].find("</file>")?;
    let inner = xml[start..end].trim();
    // A timed one names a size per file: <file><size width=".." ..>path</size></file>.
    let inner = match inner.find('>') {
        Some(i) if inner.starts_with('<') => inner[i + 1..].split('<').next().unwrap_or("").trim(),
        _ => inner,
    };
    (!inner.is_empty()).then(|| inner.to_owned())
}

/// The image Plasma's desktop shows (the first screen's), from its desktop config. A wallpaper
/// package resolves to its largest image; no image set means Plasma's default, Next.
fn plasma() -> Option<PathBuf> {
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
            // By bytes: what follows a % can be the middle of a multi-byte character.
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
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
    fn reads_a_gnome_slideshow() {
        let timed = "<background><static><duration>8.0</duration><file>/usr/share/backgrounds/gnome/a-l.jpg</file></static></background>";
        assert_eq!(slideshow_first(timed).as_deref(), Some("/usr/share/backgrounds/gnome/a-l.jpg"));
        let sized = "<background><static><file><size width=\"1920\" height=\"1080\">/x/b.png</size></file></static></background>";
        assert_eq!(slideshow_first(sized).as_deref(), Some("/x/b.png"));
        assert_eq!(slideshow_first("<background/>"), None);
    }

    #[test]
    fn finds_the_image_setting() {
        let text = "[Containments][1][General]\nImage=nope\n[Containments][1][Wallpaper][org.kde.image][General]\nImage=file:///home/me/My%20Pics/a.jpg\n";
        assert_eq!(image_setting(text).as_deref(), Some("file:///home/me/My%20Pics/a.jpg"));
        assert_eq!(unescape("/home/me/My%20Pics/a.jpg"), "/home/me/My Pics/a.jpg");
        assert_eq!(unescape("/a%2"), "/a%2");
        assert_eq!(unescape("/home/me/100%日本.jpg"), "/home/me/100%日本.jpg");
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
