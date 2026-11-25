//! The daemon over a local socket (macOS, where there's no session bus): `vyoma-mac` listens on
//! `~/Library/Application Support/Vyoma/control.sock` (or `VYOMA_SOCKET`), readable by this user
//! alone. On Windows the same lines go over the agent's named pipe, `\\.\pipe\vyoma-<user>`
//! (or `VYOMA_PIPE`), which only this user can open. One JSON line per request, `{"method": "Phones", "args": [...]}`, and one per reply,
//! `{"ok": value}` or `{"error": "words for the person"}`. The methods are the D-Bus interface's
//! names (`Pair`, `SendFiles`, `PairingCode`…), with JSON where D-Bus carries JSON in a string,
//! except `Phones`: every device's details at once, each with its `id` and `permits`.
//! `{"method": "Events"}` turns a connection into a stream of `{"event": "Changed"}`,
//! `{"event": "FilesSent", "id", "count", "error"}`, `ActivityChanged`, `MessagesChanged`,
//! `{"event": "SetupDone", "ok"}` and `{"event": "FirewallDone", "ok"}`.

use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream as Stream;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

use crate::daemon::{ActivityEntry, CommandData, Daemon, Event, Folder, Found, Progress, Health, Inbox, People, Permission, PhoneData, ThreadData};

const NOT_RUNNING: &str = "Vyoma isn't running here right now. It restarts by itself; if it doesn't, use Start Vyoma";
/// A call the daemon takes longer than this over has hung.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// Copying the phone's files: a long video over Wi-Fi takes a while.
const COPY_TIMEOUT: Duration = Duration::from_secs(3600);

#[cfg(windows)]
type Stream = std::fs::File;

/// A connection to the daemon at [path].
fn open(path: &std::path::Path) -> std::io::Result<Stream> {
    #[cfg(unix)]
    {
        Stream::connect(path)
    }
    #[cfg(windows)]
    {
        // Every instance of the pipe taken (231, ERROR_PIPE_BUSY): the agent makes the next at
        // once, so a moment's wait finds it.
        for _ in 0..50 {
            match std::fs::OpenOptions::new().read(true).write(true).open(path) {
                Err(e) if e.raw_os_error() == Some(231) => std::thread::sleep(Duration::from_millis(20)),
                other => return other,
            }
        }
        std::fs::OpenOptions::new().read(true).write(true).open(path)
    }
}

/// A call that takes longer than [CALL_TIMEOUT] has hung (Windows pipes opened as files have no
/// read timeout: there the agent's own answers bound it).
fn time_limit(stream: &Stream, limit: Duration) -> std::io::Result<()> {
    #[cfg(unix)]
    return stream.set_read_timeout(Some(limit));
    #[cfg(windows)]
    {
        let _ = (stream, limit);
        Ok(())
    }
}

/// Where the agent listens: its named pipe, the window works it out as the agent does
/// (`windows/src/control.rs`).
#[cfg(windows)]
pub fn path() -> PathBuf {
    if let Some(p) = std::env::var_os("VYOMA_PIPE") {
        return PathBuf::from(p);
    }
    let user: String = std::env::var("USERNAME").unwrap_or_default().chars().filter(|c| c.is_alphanumeric() || "-_.".contains(*c)).collect();
    PathBuf::from(format!(r"\\.\pipe\vyoma-{user}"))
}

// Linux reaches its daemon over D-Bus; this builds there for its tests.
#[cfg(not(windows))]
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub fn path() -> PathBuf {
    if let Some(p) = std::env::var_os("VYOMA_SOCKET") {
        return PathBuf::from(p);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join("Library/Application Support/Vyoma/control.sock")
}

pub struct SocketDaemon {
    path: PathBuf,
    events: Mutex<Option<BufReader<Stream>>>,
}

impl SocketDaemon {
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    pub fn connect() -> Result<Self> {
        Self::at(path())
    }

    pub fn at(path: PathBuf) -> Result<Self> {
        // Only to know it's there now; each call opens its own connection.
        open(&path).map_err(|_| anyhow!(NOT_RUNNING))?;
        Ok(Self { path, events: Mutex::new(None) })
    }

    fn call(&self, method: &str, args: Value) -> Result<Value> {
        self.call_within(method, args, CALL_TIMEOUT)
    }

    fn call_within(&self, method: &str, args: Value, limit: Duration) -> Result<Value> {
        let mut stream = open(&self.path).map_err(|_| anyhow!(NOT_RUNNING))?;
        time_limit(&stream, limit)?;
        writeln!(stream, "{}", json!({ "method": method, "args": args }))?;
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).map_err(|_| anyhow!("Vyoma didn't answer in time. Try again"))?;
        reply(&line)
    }

    fn json<T>(&self, method: &str, args: Value, read: impl Fn(&Value) -> T) -> Result<T> {
        Ok(read(&self.call(method, args)?))
    }

    fn unit(&self, method: &str, args: Value) -> Result<()> {
        self.call(method, args).map(drop)
    }
}

/// A reply line: its value, or the daemon's words for what went wrong.
fn reply(line: &str) -> Result<Value> {
    if line.trim().is_empty() {
        bail!("Vyoma is restarting. Try again in a moment");
    }
    let mut v: Value = serde_json::from_str(line).context("Vyoma answered something this window can't read")?;
    if let Some(e) = v["error"].as_str() {
        bail!("{e}");
    }
    Ok(v["ok"].take())
}

fn event(v: &Value) -> Option<Event> {
    Some(match v["event"].as_str()? {
        "Changed" => Event::Changed,
        "FilesSent" => Event::Sent { id: v["id"].as_str()?.to_owned(), count: v["count"].as_u64().unwrap_or(0) as u32, error: v["error"].as_str().unwrap_or("").to_owned() },
        "ActivityChanged" => Event::Activity,
        "MessagesChanged" => Event::Messages,
        "SetupDone" => Event::SetupDone { ok: v["ok"].as_bool().unwrap_or(false) },
        "FirewallDone" => Event::FirewallDone { ok: v["ok"].as_bool().unwrap_or(false) },
        _ => return None,
    })
}

impl Daemon for SocketDaemon {
    fn phones(&self) -> Result<Vec<PhoneData>> {
        let list = self.call("Phones", json!([]))?;
        Ok(list
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|d| {
                        let mut phone = PhoneData::from_details(d["id"].as_str()?, d);
                        phone.permits = Permission::list(&d["permits"]);
                        Some(phone)
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    fn act(&self, method: &str, id: &str) -> Result<()> {
        self.unit(method, json!([id]))
    }

    fn send_files(&self, id: &str, paths: Vec<String>) -> Result<()> {
        self.unit("SendFiles", json!([id, paths]))
    }

    fn search(&self) -> Result<()> {
        self.unit("Refresh", json!([]))
    }

    fn pairing_code(&self) -> Result<String> {
        self.json("PairingCode", json!([]), |v| v.as_str().unwrap_or("").to_owned())
    }

    fn end_pairing_code(&self) -> Result<()> {
        self.unit("EndPairingCode", json!([]))
    }

    fn health(&self) -> Result<Health> {
        self.json("Health", json!([]), Health::from_json)
    }

    fn set_permission(&self, id: &str, key: &str, on: bool) -> Result<()> {
        self.unit("SetPermission", json!([id, key, on]))
    }

    fn allow_clipboard(&self, id: &str) -> Result<()> {
        self.unit("AllowClipboardSync", json!([id]))
    }

    fn commands(&self) -> Result<Vec<CommandData>> {
        self.json("Commands", json!([]), CommandData::list)
    }

    fn save_command(&self, key: &str, name: &str, command: &str, fingerprint: bool, every_time: bool) -> Result<String> {
        self.json("SaveCommand", json!([key, name, command, fingerprint, every_time]), |v| v.as_str().unwrap_or("").to_owned())
    }

    fn remove_command(&self, key: &str) -> Result<()> {
        self.unit("RemoveCommand", json!([key]))
    }

    fn install(&self) -> Result<()> {
        self.unit("Install", json!([]))
    }

    fn open_firewall(&self) -> Result<()> {
        self.unit("OpenFirewall", json!([]))
    }

    fn fix_check(&self, name: &str) -> Result<()> {
        self.unit("FixCheck", json!([name]))
    }

    fn update(&self) -> Result<()> {
        self.unit("Update", json!([]))
    }

    fn set_update_checks(&self, on: bool) -> Result<()> {
        self.unit("SetUpdateChecks", json!([on]))
    }

    fn activity(&self) -> Result<Vec<ActivityEntry>> {
        self.json("Activity", json!([]), ActivityEntry::list)
    }

    fn clear_activity(&self) -> Result<()> {
        self.unit("ClearActivity", json!([]))
    }

    fn wallpaper(&self) -> Result<crate::look::Wallpaper> {
        let v = self.call("Wallpaper", json!([]))?;
        let colors = v["colors"].as_array().context("no colours")?.iter().map(|c| {
            let b = |i: usize| c[i].as_u64().unwrap_or(0).min(255) as u8;
            [b(0), b(1), b(2)]
        });
        Ok(crate::look::Wallpaper { cols: v["cols"].as_u64().unwrap_or(0) as usize, rows: v["rows"].as_u64().unwrap_or(0) as usize, colors: colors.collect() })
    }

    fn conversations(&self, id: &str) -> Result<Inbox> {
        self.json("Conversations", json!([id]), Inbox::from_json)
    }

    fn thread(&self, id: &str, thread: i64) -> Result<ThreadData> {
        self.json("Thread", json!([id, thread]), ThreadData::from_json)
    }

    fn send_text(&self, id: &str, thread: i64, text: &str) -> Result<()> {
        self.unit("TextThread", json!([id, thread, text]))
    }

    fn unlock_texts(&self, id: &str) -> Result<()> {
        self.unit("UnlockTexts", json!([id]))
    }

    fn contacts(&self, id: &str) -> Result<People> {
        self.json("Contacts", json!([id]), People::from_json)
    }

    fn search_texts(&self, id: &str, query: &str) -> Result<Found> {
        self.json("SearchTexts", json!([id, query]), Found::from_json)
    }

    fn thread_for(&self, id: &str, number: &str) -> Result<i64> {
        self.json("ThreadFor", json!([id, number]), |v| v.as_i64().unwrap_or(0))
    }

    fn call_number(&self, id: &str, number: &str) -> Result<()> {
        self.unit("Call", json!([id, number]))
    }

    fn phone_files(&self, id: &str, path: Option<&str>) -> Result<Folder> {
        // The first look asks the phone to start its server and signs in: longer than a plain call.
        Ok(Folder::from_json(&self.call_within("PhoneFiles", json!([id, path]), Duration::from_secs(60))?))
    }

    fn fetch_phone_file(&self, id: &str, path: &str) -> Result<String> {
        Ok(self.call_within("FetchPhoneFile", json!([id, path]), COPY_TIMEOUT)?.as_str().unwrap_or_default().to_owned())
    }

    fn put_phone_files(&self, id: &str, dir: &str, paths: Vec<String>) -> Result<u64> {
        Ok(self.call_within("PutPhoneFiles", json!([id, dir, paths]), COPY_TIMEOUT)?.as_u64().unwrap_or(0))
    }

    fn new_phone_folder(&self, id: &str, path: &str) -> Result<()> {
        self.call_within("NewPhoneFolder", json!([id, path]), Duration::from_secs(60)).map(drop)
    }

    fn delete_phone_file(&self, id: &str, path: &str) -> Result<()> {
        self.call_within("DeletePhoneFile", json!([id, path]), COPY_TIMEOUT).map(drop)
    }

    fn rename_phone_file(&self, id: &str, path: &str, name: &str) -> Result<String> {
        Ok(self.call_within("RenamePhoneFile", json!([id, path, name]), Duration::from_secs(60))?.as_str().unwrap_or_default().to_owned())
    }

    fn phone_files_progress(&self, id: &str) -> Result<Option<Progress>> {
        let p = self.call("PhoneFilesProgress", json!([id]))?;
        Ok(p.is_object().then(|| Progress {
            name: p["name"].as_str().unwrap_or_default().to_owned(),
            done: p["done"].as_u64().unwrap_or(0),
            total: p["total"].as_u64().unwrap_or(0),
        }))
    }

    fn stop_phone_files(&self, id: &str) -> Result<()> {
        self.call("StopPhoneFiles", json!([id])).map(drop)
    }

    fn next_event(&self) -> Result<Event> {
        let mut events = self.events.lock().unwrap();
        if events.is_none() {
            let mut stream = open(&self.path).map_err(|_| anyhow!(NOT_RUNNING))?;
            writeln!(stream, "{}", json!({ "method": "Events", "args": [] }))?;
            *events = Some(BufReader::new(stream));
        }
        let reader = events.as_mut().unwrap();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line)? == 0 {
                *events = None;
                bail!("the daemon went away");
            }
            if let Some(e) = serde_json::from_str::<Value>(&line).ok().as_ref().and_then(event) {
                return Ok(e);
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    /// A daemon that answers each line with [answer] and streams two events.
    fn fake(answer: fn(&Value) -> Value) -> (PathBuf, std::thread::JoinHandle<()>) {
        let path = std::env::temp_dir().join(format!("vyoma-socket-{}-{}.sock", std::process::id(), rand_suffix()));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            for stream in listener.incoming().take(4) {
                let mut stream = stream.unwrap();
                let mut line = String::new();
                BufReader::new(stream.try_clone().unwrap()).read_line(&mut line).unwrap();
                if line.is_empty() {
                    continue; // the probe in connect()
                }
                let req: Value = serde_json::from_str(&line).unwrap();
                if req["method"] == "Events" {
                    writeln!(stream, "{}", json!({ "event": "Changed" })).unwrap();
                    writeln!(stream, "{}", json!({ "event": "FilesSent", "id": "p", "count": 2, "error": "" })).unwrap();
                } else {
                    writeln!(stream, "{}", answer(&req)).unwrap();
                }
            }
        });
        (path, server)
    }

    fn rand_suffix() -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos() as u64
    }

    #[test]
    fn talks_to_the_daemon_over_its_socket() {
        let (path, server) = fake(|req| match req["method"].as_str().unwrap() {
            "Phones" => json!({ "ok": [{ "id": "p", "name": "Pixel", "paired": true, "linked": true, "via": "wifi", "battery": { "charge": 64, "charging": false }, "can": { "share": true }, "permits": [] }] }),
            "Pair" => json!({ "error": "That phone isn't nearby now" }),
            _ => json!({ "ok": null }),
        });
        let d = SocketDaemon::at(path.clone()).unwrap();
        let phones = d.phones().unwrap();
        assert_eq!((phones[0].id.as_str(), phones[0].battery, phones[0].can_share, phones[0].via.as_str()), ("p", Some(64), true, "wifi"));
        assert_eq!(d.act("Pair", "p").unwrap_err().to_string(), "That phone isn't nearby now", "the daemon's own words");
        assert!(matches!(d.next_event().unwrap(), Event::Changed));
        assert!(matches!(d.next_event().unwrap(), Event::Sent { count: 2, .. }));
        server.join().unwrap();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn no_daemon_says_so() {
        let missing = std::env::temp_dir().join("vyoma-no-such.sock");
        assert_eq!(SocketDaemon::at(missing).err().unwrap().to_string(), NOT_RUNNING);
        assert!(reply("").is_err(), "a closed connection");
        assert_eq!(reply(r#"{"ok": "x"}"#).unwrap(), json!("x"));
    }
}
