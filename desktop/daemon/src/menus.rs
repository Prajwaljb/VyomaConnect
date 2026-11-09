//! "Send to <phone>" when a file or folder is right-clicked, in every file manager that can take
//! an entry: Dolphin (a service menu), Thunar (its custom actions, `uca.xml`), Nemo (actions),
//! Nautilus and Caja (scripts). One entry per paired phone, rewritten as phones pair, unpair or
//! are renamed; each runs `vyoma-desktop --send <id> <paths>`, which hands them to the running
//! app's `Send` (notifications say how it goes).
//!
//! Entries are only written for the file managers installed, and only ours are ever touched:
//! Dolphin's and Nemo's are files of our own name, scripts carry [MARKER], and Thunar's actions
//! have a `vyoma-send-` unique id; everything else in `uca.xml` is written back as it was. All
//! these file managers watch where their entries live, so a change shows at the next right-click.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub const SEND_ARG: &str = "--send";

/// A line in each script of ours, to know it from the user's own.
const MARKER: &str = "# vyoma-send";
const SERVICE_MENU: &str = "vyoma-send.desktop";
const UCA_ID: &str = "vyoma-send-";

#[derive(Clone, PartialEq)]
pub struct Phone {
    pub id: String,
    pub name: String,
}

/// Makes the menus offer exactly [phones] (none: takes our entries out).
pub fn update(phones: &[Phone]) {
    let Some(exe) = exe() else { return };
    // Ids go into commands unquoted: only ever the plain ones the protocol allows.
    let phones: Vec<&Phone> = phones.iter().filter(|p| !p.id.is_empty() && p.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')).collect();
    let installed = |name: &str| on_path(name);
    let result = |what: &str, r: Result<()>| {
        if let Err(e) = r {
            tracing::warn!("couldn't update {what}'s Send to menu: {e:#}");
        }
    };
    result("Dolphin", if installed("dolphin") && !phones.is_empty() { dolphin(&exe, &phones) } else { remove_file(&service_menu()) });
    result("Thunar", thunar(&exe, if installed("thunar") { &phones } else { &[] }));
    result("Nemo", nemo(&exe, if installed("nemo") { &phones } else { &[] }));
    // Nautilus: top-level items through its Python extension when that's installed, else a
    // script each (in its Scripts submenu, which is all it offers apps without it).
    let nautilus = installed("nautilus");
    let python = nautilus && vyoma_companion::setup::nautilus_python();
    result("Nautilus", nautilus_extension(&exe, if python { &phones } else { &[] }));
    result("Nautilus", scripts(&data_home().join("nautilus/scripts"), &exe, if nautilus && !python { &phones } else { &[] }));
    result("Caja", scripts(&config_home().join("caja/scripts"), &exe, if installed("caja") { &phones } else { &[] }));
    tracing::info!("Send to menus: {} phone(s)", phones.len());
}

/// Which file managers (and Nautilus's Python extensions) there are, to rewrite the menus when it changes.
pub fn installed() -> Vec<bool> {
    let mut found: Vec<bool> = ["dolphin", "thunar", "nemo", "nautilus", "caja"].iter().map(|n| on_path(n)).collect();
    found.push(vyoma_companion::setup::nautilus_python());
    found
}

/// Takes every entry of ours out (`install.sh --undo`).
pub fn remove_all() {
    let Some(exe) = exe() else { return };
    let _ = remove_file(&service_menu());
    let _ = thunar(&exe, &[]);
    let _ = nemo(&exe, &[]);
    let _ = scripts(&data_home().join("nautilus/scripts"), &exe, &[]);
    let _ = nautilus_extension(&exe, &[]);
    let _ = scripts(&config_home().join("caja/scripts"), &exe, &[]);
}

/// Nautilus's Python extension, and the phones it offers (read on each right-click, so a phone
/// paired later shows without restarting Nautilus, which loads extensions only as it starts).
fn nautilus_extension(exe: &Path, phones: &[&Phone]) -> Result<()> {
    let dir = data_home().join("nautilus-python/extensions");
    let list = dir.join("vyoma-phones.json");
    let code = dir.join("vyoma_send.py");
    if phones.is_empty() {
        let _ = remove_file(&list);
        let _ = remove_file(&code);
        return Ok(());
    }
    let named: Vec<serde_json::Value> = phones.iter().map(|p| serde_json::json!({ "id": p.id, "name": p.name })).collect();
    write_if_changed(&list, &serde_json::to_string(&named)?)?;
    let exe = serde_json::to_string(&exe.to_string_lossy())?;
    write_if_changed(&code, &NAUTILUS_PY.replace("@EXE@", &exe).replace("@SEND@", SEND_ARG))
}

const NAUTILUS_PY: &str = r#"# vyoma-send: "Send to <phone>" in Nautilus, written by Vyoma (vyoma-desktop); removed with it.
import json
import os
import subprocess

from gi.repository import GObject, Nautilus

EXE = @EXE@
PHONES = os.path.join(os.path.dirname(os.path.abspath(__file__)), "vyoma-phones.json")


class VyomaSend(GObject.GObject, Nautilus.MenuProvider):
    def get_file_items(self, *args):
        files = args[-1]
        paths = [f.get_location().get_path() for f in files]
        if not paths or None in paths:
            return []
        try:
            with open(PHONES) as f:
                phones = json.load(f)
        except (OSError, ValueError):
            return []
        items = []
        for phone in phones:
            item = Nautilus.MenuItem(name="Vyoma::send-" + phone["id"], label="Send to " + phone["name"], icon="vyoma")
            item.connect("activate", self._send, phone["id"], paths)
            items.append(item)
        return items

    def get_background_items(self, *args):
        return []

    def _send(self, _item, phone, paths):
        subprocess.Popen([EXE, "@SEND@", phone, *paths], start_new_session=True)
"#;

/// The menus' side: `--send <id> [paths]`. Nautilus and Caja give their scripts the selection in
/// the environment, as full paths, which beats the bare names they pass as arguments.
pub fn send(args: &[String]) -> Result<()> {
    let (id, rest) = args.split_first().context("usage: vyoma-desktop --send <device id> <files or folders>")?;
    let picked = ["NAUTILUS_SCRIPT_SELECTED_FILE_PATHS", "CAJA_SCRIPT_SELECTED_FILE_PATHS"]
        .iter()
        .find_map(|v| std::env::var(v).ok().filter(|s| !s.trim().is_empty()))
        .map(|s| s.lines().filter(|l| !l.is_empty()).map(str::to_owned).collect::<Vec<_>>());
    let here = std::env::current_dir().unwrap_or_default();
    let paths: Vec<String> = picked.unwrap_or_else(|| rest.to_vec()).iter().map(|p| here.join(p).to_string_lossy().into_owned()).collect();
    if paths.is_empty() {
        return Ok(());
    }
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let sent = runtime.block_on(async {
        let conn = zbus::Connection::session().await?;
        conn.call_method(Some(crate::BUS_NAME), crate::OBJECT_PATH, Some("dev.vyoma.Desktop1"), "Send", &(id.as_str(), paths)).await?;
        anyhow::Ok(())
    });
    if let Err(e) = sent {
        tracing::warn!("couldn't hand the files over: {e:#}");
        let _ = std::process::Command::new("notify-send")
            .args(["--app-name=Vyoma", "--icon=vyoma", "--hint=string:desktop-entry:vyoma", "Couldn't send", "Vyoma isn't running on this PC right now. Open Vyoma, then try again."])
            .status();
    }
    Ok(())
}

// ---- Dolphin ------------------------------------------------------------------------------

fn service_menu() -> PathBuf {
    data_home().join("kio/servicemenus").join(SERVICE_MENU)
}

fn dolphin(exe: &Path, phones: &[&Phone]) -> Result<()> {
    let actions: String = (0..phones.len()).map(|i| format!("send{i};")).collect();
    let mut text = format!("[Desktop Entry]\nType=Service\nMimeType=all/all;\nX-KDE-Protocols=file\nX-KDE-Priority=TopLevel\nActions={actions}\n");
    for (i, p) in phones.iter().enumerate() {
        text += &format!("\n[Desktop Action send{i}]\nName=Send to {}\nIcon=vyoma\nExec={} {SEND_ARG} {} %F\n", entry_value(&p.name), exec_arg(exe), p.id);
    }
    let file = service_menu();
    write(&file, &text)?;
    // KDE only runs service menus of the user's that are executable.
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755))?;
    Ok(())
}

// ---- Thunar -------------------------------------------------------------------------------

fn thunar(exe: &Path, phones: &[&Phone]) -> Result<()> {
    let file = config_home().join("Thunar/uca.xml");
    let before = match std::fs::read_to_string(&file) {
        Ok(s) => s,
        Err(_) if phones.is_empty() => return Ok(()),
        // Thunar uses the system's when the user has none: start from it, so its actions stay.
        Err(_) => std::fs::read_to_string("/etc/xdg/Thunar/uca.xml").unwrap_or_else(|_| "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<actions>\n</actions>\n".into()),
    };
    let ours: String = phones
        .iter()
        .map(|p| {
            format!(
                "<action>\n\t<icon>vyoma</icon>\n\t<name>Send to {}</name>\n\t<submenu></submenu>\n\t<unique-id>{UCA_ID}{}</unique-id>\n\t<command>{} {SEND_ARG} {} %F</command>\n\t<description>Send with Vyoma</description>\n\t<range></range>\n\t<patterns>*</patterns>\n\t<directories/>\n\t<audio-files/>\n\t<image-files/>\n\t<other-files/>\n\t<text-files/>\n\t<video-files/>\n</action>\n",
                xml(&p.name),
                p.id,
                xml(&shell_arg(exe)),
                p.id
            )
        })
        .collect();
    let Some(after) = with_actions(&before, &ours) else {
        anyhow::bail!("{} isn't as Thunar writes it, so it's left alone", file.display());
    };
    if after != before {
        write(&file, &after)?;
    }
    Ok(())
}

/// [uca] with our actions taken out and [ours] put in before the end; None if it has no end.
fn with_actions(uca: &str, ours: &str) -> Option<String> {
    let mut kept = String::with_capacity(uca.len());
    let mut rest = uca;
    while let Some(start) = rest.find("<action>") {
        let len = rest[start..].find("</action>")? + "</action>".len();
        let action = &rest[start..start + len];
        kept += &rest[..start];
        if action.contains(&format!("<unique-id>{UCA_ID}")) {
            // And the line it was on.
            rest = rest[start + len..].strip_prefix('\n').unwrap_or(&rest[start + len..]);
        } else {
            kept += action;
            rest = &rest[start + len..];
        }
    }
    kept += rest;
    let end = kept.rfind("</actions>")?;
    Some(format!("{}{ours}{}", &kept[..end], &kept[end..]))
}

// ---- Nemo ---------------------------------------------------------------------------------

fn nemo(exe: &Path, phones: &[&Phone]) -> Result<()> {
    let dir = data_home().join("nemo/actions");
    let mine = |name: &str| name.starts_with("vyoma-send-") && name.ends_with(".nemo_action");
    let wanted: Vec<String> = phones.iter().map(|p| format!("vyoma-send-{}.nemo_action", p.id)).collect();
    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if mine(&name) && !wanted.contains(&name) {
            remove_file(&entry.path())?;
        }
    }
    for (p, file) in phones.iter().zip(&wanted) {
        // % starts one of Nemo's own tokens in a name.
        let name = entry_value(&p.name).replace('%', "");
        let text = format!("[Nemo Action]\nName=Send to {name}\nComment=Send with Vyoma\nExec={} {SEND_ARG} {} %F\nIcon-Name=vyoma\nSelection=notnone\nExtensions=any;\n", exec_arg(exe), p.id);
        write_if_changed(&dir.join(file), &text)?;
    }
    Ok(())
}

// ---- Nautilus, Caja -----------------------------------------------------------------------

/// Scripts in [dir], named "Send to <phone>": the file manager shows them under Scripts.
fn scripts(dir: &Path, exe: &Path, phones: &[&Phone]) -> Result<()> {
    let wanted: Vec<(String, String)> = phones
        .iter()
        .map(|p| {
            let file = format!("Send to {}", p.name.replace(['/', '\n', '\r'], " ").trim_start_matches('.').trim());
            (file, format!("#!/bin/sh\n{MARKER} {}\nexec {} {SEND_ARG} {} \"$@\"\n", p.id, shell_arg(exe), p.id))
        })
        .collect();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_ours(&entry.path()) && !wanted.iter().any(|(f, _)| *f == name) {
            remove_file(&entry.path())?;
        }
    }
    for (file, text) in &wanted {
        let path = dir.join(file);
        // One of the user's own with that name stays theirs.
        if path.exists() && !is_ours(&path) {
            continue;
        }
        write_if_changed(&path, text)?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn is_ours(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && m.len() < 4096) && std::fs::read_to_string(path).is_ok_and(|s| s.lines().any(|l| l.starts_with(MARKER)))
}

// ---- shared -------------------------------------------------------------------------------

/// This binary, where the menus will find it: the installed one, even after an update replaced it.
fn exe() -> Option<PathBuf> {
    let exe = std::fs::read_link("/proc/self/exe").or_else(|_| std::env::current_exe()).ok()?;
    let s = exe.to_string_lossy();
    Some(PathBuf::from(s.strip_suffix(" (deleted)").unwrap_or(&s)))
}

fn on_path(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(name).is_file()))
}

pub fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".local/share"))
}

fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".config"))
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

/// A name as a desktop entry value: one line, backslashes escaped.
fn entry_value(s: &str) -> String {
    s.replace(['\n', '\r'], " ").replace('\\', "\\\\")
}

/// A path as a desktop entry Exec argument (the spec's quoting, then its value escaping).
fn exec_arg(path: &Path) -> String {
    let s = path.to_string_lossy();
    if s.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+".contains(c)) {
        return s.into_owned();
    }
    let quoted: String = s.chars().flat_map(|c| if "\"`$\\".contains(c) { vec!['\\', c] } else { vec![c] }).collect();
    format!("\"{}\"", quoted.replace('\\', "\\\\"))
}

/// A path for a shell command line.
fn shell_arg(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace(['\n', '\r'], " ")
}

fn write_if_changed(file: &Path, text: &str) -> Result<()> {
    if std::fs::read_to_string(file).is_ok_and(|s| s == text) {
        return Ok(());
    }
    write(file, text)
}

/// Whole or not at all: a file manager reading meanwhile sees the old file or the new.
fn write(file: &Path, text: &str) -> Result<()> {
    let dir = file.parent().context("no folder")?;
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{}.vyoma", file.file_name().and_then(|n| n.to_str()).unwrap_or("menu")));
    std::fs::write(&tmp, text)?;
    if let Ok(meta) = std::fs::metadata(file) {
        std::fs::set_permissions(&tmp, meta.permissions())?;
    }
    std::fs::rename(&tmp, file).with_context(|| format!("can't write {}", file.display()))
}

fn remove_file(file: &Path) -> Result<()> {
    match std::fs::remove_file(file) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e).with_context(|| format!("can't remove {}", file.display())),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<actions>\n<action>\n\t<name>Open Terminal Here</name>\n\t<unique-id>1-1</unique-id>\n</action>\n</actions>\n";

    #[test]
    fn thunar_keeps_the_users_actions() {
        let ours = format!("<action>\n\t<unique-id>{UCA_ID}abc</unique-id>\n</action>\n");
        let added = with_actions(USER, &ours).unwrap();
        assert!(added.contains("Open Terminal Here") && added.contains(&ours));
        assert!(added.trim_end().ends_with("</actions>"));
        // Again: replaced, not doubled.
        let again = with_actions(&added, &ours).unwrap();
        assert_eq!(again, added);
        // Gone: back as it was.
        assert_eq!(with_actions(&added, "").unwrap(), USER);
    }

    #[test]
    fn a_broken_uca_file_is_left_alone() {
        assert!(with_actions("<actions>\n<action>\n", "x").is_none());
        assert!(with_actions("garbage", "x").is_none());
    }

    #[test]
    fn paths_are_quoted_for_each_place() {
        assert_eq!(exec_arg(Path::new("/home/a/.local/bin/vyoma-desktop")), "/home/a/.local/bin/vyoma-desktop");
        assert_eq!(exec_arg(Path::new("/home/a b/x")), "\"/home/a b/x\"");
        assert_eq!(exec_arg(Path::new("/a$b")), "\"/a\\\\$b\"");
        assert_eq!(shell_arg(Path::new("/it's")), "'/it'\\''s'");
        assert_eq!(xml("A&B <x>"), "A&amp;B &lt;x&gt;");
    }
}
