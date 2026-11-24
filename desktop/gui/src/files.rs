//! The Files page (the Mac and Windows; Linux mounts the phone's files in the file manager): one
//! folder of the phone at a time, read from the agent (which talks to the phone's file server) off
//! the UI thread. A file opened is copied into Downloads first, then opened with its app.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{Datelike, Local, TimeZone};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::daemon::{self, FileEntry, Folder};
use crate::{MainWindow, PhoneFile, breakable, notify};

/// The page's id in `selected`.
pub const FILES: &str = "files";

/// Which phone and folder the page shows.
#[derive(Default)]
struct Here {
    id: String,
    phone: String,
    /// None: the phone's first top folder (as it's first opened).
    path: Option<String>,
    /// The folder's name, as the page shows it.
    name: String,
    parent: Option<String>,
    /// The phone's top folders, for the way back's name.
    roots: Vec<(String, String)>,
}

static HERE: Mutex<Option<Here>> = Mutex::new(None);

fn here<T>(f: impl FnOnce(&mut Here) -> T) -> Option<T> {
    HERE.lock().unwrap_or_else(|e| e.into_inner()).as_mut().map(f)
}

pub fn wire(ui: &MainWindow) {
    ui.on_phone_files_up({
        let ui = ui.as_weak();
        move || {
            let Some(win) = ui.upgrade() else { return };
            match here(|h| h.parent.clone()).flatten() {
                Some(up) => {
                    here(|h| h.path = Some(up));
                    win.set_files_state("loading".into());
                    read(&ui);
                }
                // At a top folder: back to the phone's page.
                None => {
                    let id = here(|h| h.id.clone()).unwrap_or_default();
                    win.set_selected(id.into());
                }
            }
        }
    });
    ui.on_reload_phone_files({
        let ui = ui.as_weak();
        move || {
            if let Some(win) = ui.upgrade() {
                win.set_files_state("loading".into());
            }
            read(&ui);
        }
    });
    ui.on_stop_phone_files({
        let ui = ui.as_weak();
        move || {
            let Some(id) = here(|h| h.id.clone()) else { return };
            if let Some(win) = ui.upgrade() {
                win.set_files_can_stop(false);
                win.set_files_busy("Stopping\u{2026}".into());
            }
            std::thread::spawn(move || {
                let _ = daemon::connect().and_then(|d| d.stop_phone_files(&id));
            });
        }
    });
    ui.on_open_phone_file({
        let ui = ui.as_weak();
        move |f| {
            let Some(win) = ui.upgrade() else { return };
            if f.folder {
                here(|h| h.path = Some(f.path.to_string()));
                win.set_files_state("loading".into());
                read(&ui);
                return;
            }
            let Some(id) = here(|h| h.id.clone()) else { return };
            let (name, path) = (f.name.to_string(), f.path.to_string());
            let ui = ui.clone();
            std::thread::spawn(move || {
                let watch = Copying::start(&ui, &id, &format!("Copying {name}\u{2026}"));
                let done = daemon::connect().and_then(|d| d.fetch_phone_file(&id, &path));
                drop(watch);
                match done {
                    Ok(local) => {
                        if let Err(e) = open_here(&local) {
                            notify(&ui, format!("{name} is in Downloads, but couldn't be opened: {e}"));
                        } else {
                            notify(&ui, format!("{name} is in Downloads"));
                        }
                    }
                    Err(e) if stopped(&e) => notify(&ui, format!("Stopped copying {name}")),
                    Err(e) => notify(&ui, format!("Couldn't copy {name}: {e:#}")),
                }
            });
        }
    });
    ui.on_add_phone_files({
        let ui = ui.as_weak();
        move || {
            let Some((id, dir, phone)) = here(|h| (h.id.clone(), h.path.clone(), h.phone.clone())) else { return };
            let Some(dir) = dir else { return };
            let ui = ui.clone();
            std::thread::spawn(move || {
                let Some(files) = rfd::FileDialog::new().set_title(format!("Add to {phone}")).pick_files() else { return };
                let paths: Vec<String> = files.iter().map(|p| p.to_string_lossy().into_owned()).collect();
                let n = paths.len();
                let watch = Copying::start(&ui, &id, &if n == 1 { "Copying 1 file\u{2026}".to_owned() } else { format!("Copying {n} files\u{2026}") });
                let done = daemon::connect().and_then(|d| d.put_phone_files(&id, &dir, paths));
                drop(watch);
                match done {
                    Ok(sent) => notify(&ui, if sent == 1 { format!("Copied 1 file to {phone}") } else { format!("Copied {sent} files to {phone}") }),
                    Err(e) if stopped(&e) => notify(&ui, format!("Stopped copying to {phone}")),
                    Err(e) => notify(&ui, format!("Couldn't copy to {phone}: {e:#}")),
                }
                read(&ui);
            });
        }
    });
    ui.on_copy_phone_folder({
        let ui = ui.as_weak();
        move || {
            let Some((id, path, name)) = here(|h| (h.id.clone(), h.path.clone(), h.name.clone())) else { return };
            let Some(path) = path else { return };
            let ui = ui.clone();
            std::thread::spawn(move || {
                let watch = Copying::start(&ui, &id, &format!("Copying {name}\u{2026}"));
                let done = daemon::connect().and_then(|d| d.fetch_phone_file(&id, &path));
                drop(watch);
                match done {
                    Ok(local) => {
                        let _ = open_here(&local);
                        notify(&ui, format!("{name} is in Downloads"));
                    }
                    Err(e) if stopped(&e) => notify(&ui, format!("Stopped copying {name}")),
                    Err(e) => notify(&ui, format!("Couldn't copy {name}: {e:#}")),
                }
            });
        }
    });
    ui.on_make_phone_folder({
        let ui = ui.as_weak();
        move |name| {
            let name = name.trim().to_owned();
            let Some((id, dir)) = here(|h| (h.id.clone(), h.path.clone())) else { return };
            let Some(dir) = dir else { return };
            if let Err(why) = folder_name(&name) {
                notify(&ui, why.to_owned());
                return;
            }
            let path = format!("{}/{name}", dir.trim_end_matches('/'));
            let ui = ui.clone();
            std::thread::spawn(move || {
                if let Err(e) = daemon::connect().and_then(|d| d.new_phone_folder(&id, &path)) {
                    notify(&ui, format!("Couldn't make {name}: {e:#}"));
                }
                read(&ui);
            });
        }
    });
    ui.on_delete_phone_file({
        let ui = ui.as_weak();
        move |f| {
            let Some(id) = here(|h| h.id.clone()) else { return };
            let (name, path) = (f.name.to_string(), f.path.to_string());
            busy(&ui, &format!("Deleting {name}\u{2026}"));
            let ui = ui.clone();
            std::thread::spawn(move || {
                let done = daemon::connect().and_then(|d| d.delete_phone_file(&id, &path));
                busy(&ui, "");
                if let Err(e) = done {
                    notify(&ui, format!("Couldn't delete {name}: {e:#}"));
                }
                read(&ui);
            });
        }
    });
}

/// Opens phone [id]'s files at its first top folder.
pub fn show(ui: &slint::Weak<MainWindow>, id: &str) {
    let Some(win) = ui.upgrade() else { return };
    start(&win, id);
    read(ui);
}

/// The page, opened on phone [id] and loading (snapshots fill it themselves).
pub fn start(win: &MainWindow, id: &str) {
    let phone = win.get_current().name.to_string();
    *HERE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Here { id: id.to_owned(), phone: phone.clone(), ..Default::default() });
    win.set_files_phone(phone.clone().into());
    win.set_files_title("Files".into());
    win.set_files_back(phone.into());
    win.set_files_list(ModelRc::default());
    win.set_files_busy("".into());
    win.set_files_state("loading".into());
    win.set_selected(FILES.into());
}

/// A copy under way: the page's card shows [what], then the phone's progress, asked for every
/// 400 ms, until this is dropped (the copy's answer came).
struct Copying {
    running: Arc<AtomicBool>,
    ui: slint::Weak<MainWindow>,
}

impl Copying {
    fn start(ui: &slint::Weak<MainWindow>, id: &str, what: &str) -> Copying {
        let running = Arc::new(AtomicBool::new(true));
        let (ui2, what) = (ui.clone(), what.to_owned());
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(win) = ui2.upgrade() {
                win.set_files_busy(what.into());
                win.set_files_progress(-1.0);
                win.set_files_progress_text("".into());
                win.set_files_can_stop(true);
            }
        });
        let (ui2, id, on) = (ui.clone(), id.to_owned(), running.clone());
        std::thread::spawn(move || {
            while on.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(400));
                let Ok(Some(p)) = daemon::connect().and_then(|d| d.phone_files_progress(&id)) else { continue };
                let (ui, on) = (ui2.clone(), on.clone());
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(win) = ui.upgrade() else { return };
                    if !on.load(Ordering::Relaxed) {
                        return;
                    }
                    // Once Stop is pressed, "Stopping…" stays.
                    if win.get_files_can_stop() {
                        win.set_files_busy(format!("Copying {}\u{2026}", breakable(&p.name)).into());
                    }
                    win.set_files_progress(if p.total > 0 { p.done as f32 / p.total as f32 } else { -1.0 });
                    win.set_files_progress_text(if p.total > 0 { format!("{} of {}", size(p.done), size(p.total)) } else { size(p.done) }.into());
                });
            }
        });
        Copying { running, ui: ui.clone() }
    }
}

impl Drop for Copying {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        let ui = self.ui.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(win) = ui.upgrade() {
                win.set_files_busy("".into());
                win.set_files_progress(-1.0);
                win.set_files_progress_text("".into());
                win.set_files_can_stop(false);
            }
        });
    }
}

/// The person stopped it (the engine's `phonefiles::STOPPED`).
fn stopped(e: &anyhow::Error) -> bool {
    e.to_string() == "Stopped"
}

fn busy(ui: &slint::Weak<MainWindow>, what: &str) {
    let (ui, what) = (ui.clone(), what.to_owned());
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(win) = ui.upgrade() {
            win.set_files_busy(what.into());
        }
    });
}

/// Reads the folder the page shows, if it's showing.
pub fn read(ui: &slint::Weak<MainWindow>) {
    let ui = ui.clone();
    let _ = slint::invoke_from_event_loop(move || {
        let Some(win) = ui.upgrade() else { return };
        if win.get_selected() != FILES {
            return;
        }
        let Some((id, path)) = here(|h| (h.id.clone(), h.path.clone())) else { return };
        std::thread::spawn(move || {
            let read = daemon::connect().and_then(|d| d.phone_files(&id, path.as_deref()));
            let _ = slint::invoke_from_event_loop(move || {
                let Some(win) = ui.upgrade() else { return };
                // Only if it's still this folder that's shown.
                if win.get_selected() != FILES || here(|h| (h.id.clone(), h.path.clone())) != Some((id, path)) {
                    return;
                }
                match read {
                    Ok(folder) => show_folder(&win, &folder),
                    Err(e) => {
                        win.set_files_error(format!("{e:#}").into());
                        win.set_files_state("error".into());
                    }
                }
            });
        });
    });
}

pub fn show_folder(win: &MainWindow, folder: &Folder) {
    let phone = here(|h| {
        h.path = Some(folder.path.clone());
        h.name = folder.name.clone();
        h.parent = folder.parent.clone();
        h.roots = folder.roots.clone();
        h.phone.clone()
    })
    .unwrap_or_default();
    let back = match &folder.parent {
        Some(up) => folder.roots.iter().find(|r| &r.0 == up).map_or_else(|| leaf(up).to_owned(), |r| r.1.clone()),
        None => phone,
    };
    win.set_files_title(breakable(&folder.name).into());
    win.set_files_back(back.into());
    let now = Local::now().timestamp();
    let rows: Vec<PhoneFile> = folder.entries.iter().map(|e| row(&folder.path, e, now)).collect();
    win.set_files_list(ModelRc::new(VecModel::from(rows)));
    win.set_files_state("ready".into());
}

fn row(dir: &str, e: &FileEntry, now: i64) -> PhoneFile {
    PhoneFile {
        name: breakable(&e.name).into(),
        path: format!("{}/{}", dir.trim_end_matches('/'), e.name).into(),
        folder: e.folder,
        detail: if e.folder { when(e.modified, now) } else { [size(e.size), when(e.modified, now)].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" \u{b7} ") }.into(),
        kind: kind(&e.name).into(),
    }
}

/// Which glyph a file gets, by its extension.
fn kind(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "heif" | "bmp" | "dng" | "avif" => "image",
        "mp4" | "mkv" | "mov" | "webm" | "3gp" | "avi" | "m4v" => "video",
        _ => "file",
    }
}

/// "812 bytes", "4.2 KB", "18 MB", "1.3 GB" (powers of 1000, as phones and Finder count).
fn size(bytes: u64) -> String {
    let b = bytes as f64;
    match bytes {
        0..1000 => format!("{bytes} bytes"),
        1000..1_000_000 => format!("{:.1} KB", b / 1e3).replace(".0 ", " "),
        1_000_000..1_000_000_000 => if b < 1e7 { format!("{:.1} MB", b / 1e6) } else { format!("{:.0} MB", b / 1e6) },
        _ => format!("{:.1} GB", b / 1e9),
    }
}

/// "14:32" today, "3 Nov" this year, "3 Nov 2025" before; "" unknown.
fn when(secs: i64, now: i64) -> String {
    if secs <= 0 {
        return String::new();
    }
    let (Some(at), Some(today)) = (Local.timestamp_opt(secs, 0).single(), Local.timestamp_opt(now, 0).single()) else { return String::new() };
    if at.date_naive() == today.date_naive() {
        at.format("%H:%M").to_string()
    } else if at.year() == today.year() {
        at.format("%-d %b").to_string()
    } else {
        at.format("%-d %b %Y").to_string()
    }
}

/// Why [name] can't be a folder's name, if it can't.
fn folder_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() || name == "." || name == ".." {
        return Err("Give the folder a name");
    }
    if name.contains(['/', '\\']) || name.chars().any(char::is_control) {
        return Err("A folder's name can't have / or \\ in it");
    }
    Ok(())
}

fn leaf(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().filter(|l| !l.is_empty()).unwrap_or(path)
}

/// Opens a file here with its app (the path as one argument, never through a shell).
fn open_here(path: &str) -> std::io::Result<()> {
    let mut cmd = if cfg!(windows) {
        let mut c = std::process::Command::new("explorer");
        c.arg(path);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg(path);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(path);
        c
    };
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_kinds_and_names() {
        assert_eq!(size(812), "812 bytes");
        assert_eq!(size(4_200), "4.2 KB");
        assert_eq!(size(1_000), "1 KB");
        assert_eq!(size(2_400_000), "2.4 MB");
        assert_eq!(size(18_300_000), "18 MB");
        assert_eq!(size(1_300_000_000), "1.3 GB");
        assert_eq!((kind("IMG_1.JPG"), kind("clip.mp4"), kind("notes.txt"), kind("README")), ("image", "video", "file", "file"));
        assert!(folder_name("Trips 2026").is_ok());
        assert!(folder_name("a/b").is_err() && folder_name("..").is_err() && folder_name("").is_err());
        assert_eq!(leaf("/DCIM/Camera"), "Camera");
        assert_eq!(when(0, 1_800_000_000), "");
    }

    #[test]
    fn rows_join_their_folder() {
        let e = FileEntry { name: "a.jpg".into(), folder: false, size: 2_400_000, modified: 0 };
        let r = row("/DCIM/", &e, 1_800_000_000);
        assert_eq!((r.path.as_str(), r.detail.as_str(), r.kind.as_str()), ("/DCIM/a.jpg", "2.4 MB", "image"));
    }
}
