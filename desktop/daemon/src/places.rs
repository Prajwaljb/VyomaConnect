//! A mounted phone in the file managers' sidebars, as KDE Connect's was: its folder, "<phone>
//! (Vyoma Desktop)", in the GTK bookmarks that Thunar, Nautilus, Nemo, Caja and COSMIC Files share
//! (`gtk-3.0/bookmarks`), which they watch, so it shows while mounted and goes with the mount.
//! Dolphin lists the mount by itself (under Remote, by its folder's name), so it gets no entry of
//! ours: one there doubled it. Entries an older version put in Dolphin's `user-places.xbel` are
//! still taken out.
//!
//! An entry is ours when it points under `$XDG_RUNTIME_DIR/vyoma/`, where only our mounts live:
//! everything else in those files is the user's and is written back as it was.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Adds the mount at [at], called [label], to the GTK file managers' sidebar.
pub fn add(label: &str, at: &Path) {
    let uri = uri(at);
    edit(&gtk_file(), true, |s| with_gtk(&without_gtk(s), &uri, label));
}

/// Takes every entry of ours out (a mount ended, or at start for what a crash left).
pub fn remove_all() {
    edit(&gtk_file(), false, without_gtk);
    edit(&kde_file(), false, without_kde);
}

/// Takes [at]'s entry out.
pub fn remove(at: &Path) {
    let uri = uri(at);
    edit(&gtk_file(), false, |s| s.lines().filter(|l| !is_entry(l, &uri)).map(|l| format!("{l}\n")).collect());
    edit(&kde_file(), false, |s| drop_kde(s, |href| href == uri));
}

fn edit(file: &Path, create: bool, change: impl FnOnce(&str) -> String) {
    let before = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(_) if create => String::new(),
        Err(_) => return,
    };
    let after = change(&before);
    if after != before {
        if let Err(e) = write(file, &after) {
            tracing::warn!("couldn't update the file manager's sidebar: {e:#}");
        }
    }
}

/// Whole or not at all: a file manager reading meanwhile sees the old file or the new.
fn write(file: &Path, text: &str) -> Result<()> {
    let dir = file.parent().context("no folder")?;
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{}.vyoma", file.file_name().and_then(|n| n.to_str()).unwrap_or("places")));
    std::fs::write(&tmp, text)?;
    if let Ok(meta) = std::fs::metadata(file) {
        std::fs::set_permissions(&tmp, meta.permissions())?;
    }
    std::fs::rename(&tmp, file)?;
    Ok(())
}

fn ours() -> String {
    uri(&crate::phonefiles::mounts_dir()) + "/"
}

fn uri(path: &Path) -> String {
    format!("file://{}", crate::share::percent(&path.to_string_lossy()))
}

fn is_entry(line: &str, uri: &str) -> bool {
    line.split(' ').next() == Some(uri)
}

fn gtk_file() -> PathBuf {
    config_home().join("gtk-3.0").join("bookmarks")
}

fn kde_file() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".local/share")).join("user-places.xbel")
}

fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".config"))
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

/// GTK's file: one `URI label` a line.
fn without_gtk(s: &str) -> String {
    let ours = ours();
    s.lines().filter(|l| !l.starts_with(&ours)).map(|l| format!("{l}\n")).collect()
}

fn with_gtk(s: &str, uri: &str, label: &str) -> String {
    format!("{s}{uri} {}\n", label.replace(['\n', '\r'], " "))
}

/// KDE's: XBEL, an entry a `<bookmark>` element.
fn without_kde(s: &str) -> String {
    let ours = ours();
    drop_kde(s, |href| href.starts_with(&ours))
}

fn drop_kde(s: &str, drop: impl Fn(&str) -> bool) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("<bookmark ") {
        let Some(len) = rest[start..].find("</bookmark>").map(|e| e + "</bookmark>".len()) else { break };
        let element = &rest[start..start + len];
        let href = element.split("href=\"").nth(1).and_then(|h| h.split('"').next()).unwrap_or("");
        // The element's own line: its indent before, its line break after.
        let line_start = rest[..start].rfind('\n').map_or(0, |i| i + 1);
        let whole_line = rest[line_start..start].trim().is_empty();
        if drop(&unescape(href)) {
            out.push_str(&rest[..if whole_line { line_start } else { start }]);
            rest = &rest[start + len..];
            if whole_line {
                rest = rest.strip_prefix('\n').unwrap_or(rest);
            }
        } else {
            out.push_str(&rest[..start + len]);
            rest = &rest[start + len..];
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
fn with_kde(s: &str, uri: &str, label: &str) -> String {
    let Some(end) = s.rfind("</xbel>") else { return s.to_owned() };
    let entry = format!(
        " <bookmark href=\"{}\">\n  <title>{}</title>\n  <info>\n   <metadata owner=\"http://freedesktop.org\">\n    <bookmark:icon name=\"smartphone\"/>\n   </metadata>\n   <metadata owner=\"http://www.kde.org\">\n    <isSystemItem>false</isSystemItem>\n   </metadata>\n  </info>\n </bookmark>\n",
        escape(uri),
        escape(label)
    );
    format!("{}{entry}{}", &s[..end], &s[end..])
}

#[cfg(test)]
fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&gt;", ">").replace("&lt;", "<").replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    const XBEL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<xbel>\n <bookmark href=\"file:///home/u\">\n  <title>Home</title>\n </bookmark>\n <separator/>\n</xbel>\n";

    #[test]
    fn dolphins_places_get_the_phone_and_lose_only_it() {
        let at = crate::phonefiles::mounts_dir().join("3").join("My <Phone>");
        let uri = uri(&at);
        let added = with_kde(&without_kde(XBEL), &uri, "My <Phone> (Vyoma Desktop)");
        assert!(added.contains("<title>My &lt;Phone&gt; (Vyoma Desktop)</title>"));
        assert!(added.contains("smartphone") && added.ends_with("</xbel>\n"));
        assert!(added.contains("%20"), "spaces in the path are encoded");
        assert_eq!(without_kde(&added), XBEL);
        assert_eq!(drop_kde(&added, |h| h == uri), XBEL);
        // Twice is still one.
        assert_eq!(with_kde(&without_kde(&added), &uri, "x").matches("<bookmark ").count(), 2);
    }

    #[test]
    fn gtk_bookmarks_get_the_phone_and_lose_only_it() {
        let mine = "file:///home/u/Projects Projects\nsftp://host/ Server\n";
        let uri = uri(&crate::phonefiles::mounts_dir().join("1").join("P"));
        let added = with_gtk(&without_gtk(mine), &uri, "P (Vyoma Desktop)");
        assert!(added.ends_with(&format!("{uri} P (Vyoma Desktop)\n")));
        assert_eq!(without_gtk(&added), mine);
        assert_eq!(without_gtk("file:///home/u/x x"), "file:///home/u/x x\n");
    }

    #[test]
    fn a_missing_file_is_only_made_when_asked() {
        let dir = std::env::temp_dir().join(format!("vyoma-places-{}", std::process::id()));
        let file = dir.join("bookmarks");
        edit(&file, false, |_| "x\n".into());
        assert!(!file.exists());
        edit(&file, true, |_| "x\n".into());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "x\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
