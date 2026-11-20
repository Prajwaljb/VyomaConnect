//! The window's requests (the Linux app's D-Bus methods, as JSON over the Mac's socket and
//! Windows' pipe), answered once for the PC sides that serve them that way: pairing, ringing,
//! sending files, the pairing code, run commands, Activity, the per-phone switches and the
//! Messages page. What only one computer has (its checks, its wallpaper, its firewall) and how it
//! shows things (notifications, a phone picker, opening a page) is the [Side]'s.

use std::collections::HashMap;
use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc};
use vyoma_core::activity::Activity;
use vyoma_core::adb::ClipboardSeen;
use vyoma_core::device::Command;
use vyoma_core::identity::Identity;
use vyoma_core::invite::{self, Invite};
use vyoma_core::lan::Lan;
use vyoma_core::link::packet;
use vyoma_core::messages::{self, Store};
use vyoma_core::permits::{Permit, Permits};
use vyoma_core::release::REPO;
use vyoma_core::share::{self, SHARE};
use vyoma_core::updates::Updates;

use crate::calls::DIAL;
use crate::commands::{Commands, RUNCOMMAND};
use crate::stock::FIND;
use crate::texting::Texting;

/// A phone as the Mac's and Windows' loops keep it.
#[derive(Clone, Debug, Default)]
pub struct Device {
    pub name: String,
    pub paired: bool,
    pub linked: bool,
    pub incoming: Vec<String>,
    pub cert: Vec<u8>,
    /// The address of its newest link, where its payloads are fetched.
    pub remote: Option<IpAddr>,
    /// Its battery: charge, charging, and whether it said it's low.
    pub battery: Option<(i64, bool, bool)>,
    /// The code of a pairing in progress, and whether it's the phone asking (so Accept and Reject).
    pub key: String,
    pub asking: bool,
}

impl Device {
    /// Whether it's paired, linked and takes packets of type [kind].
    pub fn takes(&self, kind: &str) -> bool {
        self.paired && self.linked && self.incoming.iter().any(|t| t == kind)
    }

    /// What the Messages page knows of it.
    pub fn texting(&self) -> Texting {
        Texting { linked: self.linked, shares: self.takes(messages::REQUEST), guarded: self.takes(messages::UNLOCK), dials: self.takes(DIAL) }
    }

    /// How it's reached while linked: loopback is the USB cable (adb's tunnel).
    pub fn via(&self) -> &'static str {
        match self.remote.filter(|_| self.linked) {
            None => "",
            Some(r) if r.is_loopback() => "cable",
            Some(_) => "wifi",
        }
    }
}

pub type Devices = HashMap<String, Device>;
pub type Answer = Result<Value, String>;

/// The requests after which the window reads again (the rest only read, or tell by their own events).
pub const CHANGING: &[&str] = &["Pair", "Accept", "Reject", "Unpair", "SetPermission", "FixCheck", "SaveCommand", "RemoveCommand", "SetUpdateChecks"];

/// What one computer does for the window.
pub trait Side: Send + Sync + Sized + 'static {
    /// This computer as the window's errors name it: "the Mac", "Windows".
    const OS: &'static str;
    /// The per-phone switches it has, in the window's order.
    const PERMITS: &'static [Permit];
    /// Whether a call made from Messages has its sound here (Linux's hands-free); else it stays on the phone.
    const CALLS_HERE: bool = false;

    /// This computer's name, as the pairing code carries it.
    fn name() -> impl Future<Output = String> + Send;
    fn notify(title: &str, body: &str) -> impl Future<Output = ()> + Send;
    /// Opens a web page.
    fn open(url: &str);
    /// A switch's name and line, as this computer words it.
    fn permit_words(p: Permit) -> (&'static str, &'static str) {
        (p.name(), p.detail())
    }
    /// Several phones could take [paths]: the person picks one, and the side sends to it later
    /// (its own `SendFiles` request, with notifications).
    fn pick(w: &Window<Self>, phones: Vec<(String, String)>, paths: Vec<String>) -> Answer;
    /// The requests only this computer answers; None: not here.
    fn answer(w: &Window<Self>, method: &str, args: &Value, devices: &mut Devices) -> impl Future<Output = Option<Answer>> + Send;
}

pub struct Window<S: Side> {
    pub me: Arc<Identity>,
    pub lan: Arc<Lan>,
    pub commands: mpsc::UnboundedSender<Command>,
    pub changes: broadcast::Sender<Value>,
    /// Where Refresh announces this computer.
    pub everyone: Vec<SocketAddr>,
    pub run: Commands,
    pub texts: Arc<Store>,
    pub permits: Arc<Permits>,
    pub activity: Arc<Activity>,
    pub updates: Arc<Updates>,
    /// Phones whose background clipboard sync was seen granted.
    pub clip_seen: ClipboardSeen,
    pub side: S,
}

/// Argument [i] of a request, as text.
pub fn text(args: &Value, i: usize) -> Result<String, String> {
    args[i].as_str().map(str::to_owned).ok_or_else(|| "That request was missing something".to_owned())
}

/// A JSON list of strings (the files of a send).
pub fn strings(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|p| p.as_str().map(str::to_owned)).collect()).unwrap_or_default()
}

pub fn url_encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// A phone as the window reads it (the Linux app's `Details`). [linked]: how many paired phones are linked.
pub fn details<S: Side>(id: &str, d: &Device, permits: &Permits, clip_seen: &ClipboardSeen, linked: usize) -> Value {
    let via = d.via();
    let mut v = json!({
        "id": id, "name": d.name, "paired": d.paired, "linked": d.linked, "key": d.key, "asking": d.asking,
        "via": via,
        "can": { "share": d.takes(SHARE), "ring": d.takes(FIND), "browse": false, "text": d.takes(messages::REQUEST) },
        // The cable's one-time Allow for background clipboard sync (the window's Clipboard card).
        "clipboardSync": clip_seen.state(id, d.paired && d.linked && (via == "cable" || linked == 1)),
        "permits": S::PERMITS.iter().map(|&p| {
            let (name, detail) = S::permit_words(p);
            json!({ "key": p.key(), "name": name, "detail": detail, "on": permits.allows(id, p) })
        }).collect::<Vec<_>>(),
    });
    if let Some((charge, charging, low)) = d.battery {
        v["battery"] = json!({ "charge": charge, "charging": charging, "low": low });
    }
    v
}


impl<S: Side> Window<S> {
    fn command(&self, c: Command) -> Answer {
        self.commands.send(c).map(|_| Value::Null).map_err(|_| "Vyoma is stopping".to_owned())
    }

    /// Sends what phone [id] needs from [ask] (texts and contacts not loaded yet).
    fn ask(&self, id: &str, ask: Vec<Value>) -> Result<(), String> {
        ask.into_iter().try_for_each(|p| self.command(Command::Send(id.to_owned(), p)).map(drop))
    }

    /// The not-here answer.
    pub fn not_here(&self) -> String {
        format!("Not on {} yet", S::OS)
    }

    /// Answers the window's [method] with [args].
    pub async fn answer(&self, method: &str, args: &Value, devices: &mut Devices) -> Answer {
        let texting = |devices: &Devices, id: &str| devices.get(id).map(Device::texting).unwrap_or_default();
        match method {
            "Phones" => {
                let linked = devices.values().filter(|d| d.paired && d.linked).count();
                Ok(Value::Array(devices.iter().map(|(id, d)| details::<S>(id, d, &self.permits, &self.clip_seen, linked)).collect()))
            }
            "Pair" => self.command(Command::Pair(text(args, 0)?)),
            "Accept" => self.command(Command::Accept(text(args, 0)?)),
            "Reject" => self.command(Command::Reject(text(args, 0)?)),
            "Unpair" => self.command(Command::Unpair(text(args, 0)?)),
            "Ring" => {
                let id = text(args, 0)?;
                if !devices.get(&id).is_some_and(|d| d.takes(FIND)) {
                    return Err("That phone can't be rung right now".into());
                }
                self.command(Command::Send(id, packet(FIND, json!({}))))
            }
            "SendFiles" => {
                // From the file manager's menu: nobody's looking at the window, so notifications say how it went.
                let tell = args[2].as_bool().unwrap_or(false);
                self.send(text(args, 0)?, strings(&args[1]), devices, tell).await
            }
            "SendAny" => {
                let paths = strings(&args[0]);
                if paths.is_empty() {
                    return Ok(Value::Null);
                }
                let all: Vec<_> = devices.iter().map(|(id, d)| (id.clone(), d.name.clone(), d.paired, d.linked)).collect();
                match share::pick(&all) {
                    share::Pick::One(id) => self.send(id, paths, devices, true).await,
                    share::Pick::NonePaired => {
                        S::notify("No phone to send to", "Pair your phone in Vyoma first, then try again.").await;
                        Ok(Value::Null)
                    }
                    share::Pick::Ask(phones) => S::pick(self, phones, paths),
                }
            }
            "Refresh" => self.lan.announce(&self.everyone).await.map(|_| Value::Null).map_err(|e| format!("{e:#}")),
            "PairingCode" => {
                // No address, no code: a phone couldn't reach this computer. The window says so (the error's word).
                let addresses: Vec<String> = invite::local_addresses().iter().map(|a| a.to_string()).collect();
                if addresses.is_empty() {
                    return Err("offline".into());
                }
                let code = Invite::new(std::time::Instant::now()).ok_or("no randomness")?;
                let secret = invite::hex(code.secret());
                self.command(Command::Invite(Some(code)))?;
                Ok(json!(format!(
                    "https://github.com/{REPO}/releases/latest#pair?v=1&id={}&name={}&fp={}&s={secret}&a={}&p={}",
                    self.me.device_id,
                    url_encode(&S::name().await),
                    invite::fingerprint(&self.me.cert),
                    addresses.join(","),
                    self.lan.tcp_port()
                )))
            }
            "EndPairingCode" => self.command(Command::Invite(None)),
            "AllowClipboardSync" => {
                let done = vyoma_core::adb::allow_clipboard().await.map_err(|e| format!("{e:#}"))?;
                tracing::info!("background clipboard sync allowed for {done} app(s)");
                Ok(Value::Null)
            }
            "SetUpdateChecks" => {
                self.updates.set_checks(args[0].as_bool().ok_or("on or off?")?);
                Ok(Value::Null)
            }
            // The release's page, where the download is.
            "Update" => {
                S::open(&self.updates.page().ok_or_else(|| format!("No newer Vyoma for {} yet", S::OS))?);
                Ok(Value::Null)
            }
            "Commands" => Ok(Value::Object(self.run.all().into_iter().collect())),
            "SaveCommand" => {
                let flag = |i: usize| args[i].as_bool().unwrap_or(false);
                let key = self.run.save_one(&text(args, 0)?, &text(args, 1)?, &text(args, 2)?, flag(3), flag(4)).map_err(|e| format!("{e:#}"))?;
                self.commands_changed(devices);
                Ok(json!(key))
            }
            "RemoveCommand" => {
                self.run.remove(&text(args, 0)?).map_err(|e| format!("{e:#}"))?;
                self.commands_changed(devices);
                Ok(Value::Null)
            }
            "Activity" => Ok(Value::Array(self.activity.recent())),
            "ClearActivity" => {
                self.activity.clear();
                Ok(Value::Null)
            }
            "SetPermission" => {
                let (id, key) = (text(args, 0)?, text(args, 1)?);
                let on = args[2].as_bool().ok_or("on or off?")?;
                let p = Permit::from_key(&key).filter(|p| S::PERMITS.contains(p)).ok_or_else(|| format!("no permission {key}"))?;
                self.permits.set(&id, p, on).map_err(|e| format!("{e:#}"))?;
                tracing::info!("{id}: {key} {}", if on { "allowed" } else { "turned off" });
                Ok(Value::Null)
            }
            "Conversations" => {
                let id = text(args, 0)?;
                let (v, ask) = texting(devices, &id).conversations(&self.texts, &id, S::CALLS_HERE)?;
                self.ask(&id, ask)?;
                Ok(v)
            }
            "Contacts" | "SearchTexts" => {
                let id = text(args, 0)?;
                let t = texting(devices, &id);
                let (json, ask) = if method == "Contacts" { t.contacts(&self.texts, &id)? } else { t.search(&self.texts, &id, &text(args, 1)?)? };
                self.ask(&id, ask)?;
                serde_json::from_str(&json).map_err(|e| e.to_string())
            }
            "ThreadFor" => {
                let id = text(args, 0)?;
                Ok(json!(texting(devices, &id).thread_for(&self.texts, &id, &text(args, 1)?)?))
            }
            "Call" => {
                let id = text(args, 0)?;
                let number = texting(devices, &id).dial(&self.texts, &id, &text(args, 1)?)?;
                tracing::info!("{id}: asking the phone to make a call");
                self.command(Command::Send(id, crate::calls::dial_packet(&number)))
            }
            "Thread" => {
                let id = text(args, 0)?;
                let thread = args[1].as_i64().ok_or("which conversation?")?;
                let (json, ask) = texting(devices, &id).thread(&self.texts, &id, thread);
                self.ask(&id, ask)?;
                serde_json::from_str(&json).map_err(|e| e.to_string())
            }
            "TextThread" => {
                let id = text(args, 0)?;
                let thread = args[1].as_i64().ok_or("which conversation?")?;
                let p = texting(devices, &id).send(&self.texts, &id, thread, &text(args, 2)?)?;
                tracing::info!("{id}: sending a text");
                self.command(Command::Send(id, p))
            }
            "UnlockTexts" => {
                let id = text(args, 0)?;
                if let Some(p) = texting(devices, &id).unlock(&self.texts, &id)? {
                    tracing::info!("{id}: asking to show the texts");
                    self.command(Command::Send(id, p))?;
                }
                Ok(Value::Null)
            }
            _ => match S::answer(self, method, args, devices).await {
                Some(answer) => answer,
                None => Err(self.not_here()),
            },
        }
    }

    /// Sends [paths] (files and folders) to [id]; with [tell], notifications say how it went.
    pub async fn send(&self, id: String, paths: Vec<String>, devices: &Devices, tell: bool) -> Answer {
        let Some(d) = devices.get(&id).filter(|d| d.takes(SHARE)) else {
            let name = devices.get(&id).map_or("The phone", |d| d.name.as_str());
            let why = format!("{name} isn't connected. Open Vyoma on it, then try again.");
            if tell {
                S::notify("Couldn't send", &why).await;
            }
            return Err(why);
        };
        let files = match share::expand(&paths) {
            Ok(f) => f,
            Err(e) => {
                if tell {
                    S::notify("Couldn't send", &format!("{e:#}")).await;
                }
                return Err(format!("{e:#}"));
            }
        };
        let what = share::summary(&files);
        let (me, cert, commands, changes, name) = (self.me.clone(), d.cert.clone(), self.commands.clone(), self.changes.clone(), d.name.clone());
        tokio::spawn(async move {
            let count = files.len();
            let error = match share::send_files(&commands, &id, &me, &cert, files).await {
                Ok(()) => String::new(),
                Err(e) => format!("{e:#}"),
            };
            if tell {
                if error.is_empty() {
                    S::notify(&format!("Sent to {name}"), &what).await;
                } else {
                    S::notify(&format!("Couldn't send to {name}"), &error).await;
                }
            }
            let _ = changes.send(json!({ "event": "FilesSent", "id": id, "count": count, "error": error }));
        });
        Ok(Value::Null)
    }

    /// Phones showing the list get the new one.
    fn commands_changed(&self, devices: &Devices) {
        for (id, d) in devices {
            if d.takes(RUNCOMMAND) {
                let _ = self.commands.send(Command::Send(id.clone(), self.run.list_packet()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Here;

    impl Side for Here {
        const OS: &'static str = "here";
        const PERMITS: &'static [Permit] = &[Permit::Control, Permit::Files];
        async fn name() -> String {
            "Desk".into()
        }
        async fn notify(_: &str, _: &str) {}
        fn open(_: &str) {}
        fn pick(_: &Window<Self>, _: Vec<(String, String)>, _: Vec<String>) -> Answer {
            Ok(Value::Null)
        }
        async fn answer(_: &Window<Self>, _: &str, _: &Value, _: &mut Devices) -> Option<Answer> {
            None
        }
    }

    #[test]
    fn a_phone_reads_the_same_on_every_side() {
        let dir = std::env::temp_dir().join(format!("vyoma-engine-window-{}", std::process::id()));
        let seen = ClipboardSeen::load(&dir);
        let permits = Permits::default();
        permits.set("p", Permit::Files, false).unwrap();
        let mut d = Device { name: "Pixel".into(), paired: true, linked: true, incoming: vec![SHARE.into()], battery: Some((64, false, false)), remote: Some("127.0.0.1".parse().unwrap()), ..Default::default() };
        let v = details::<Here>("p", &d, &permits, &seen, 1);
        assert_eq!((v["via"].as_str(), v["can"]["share"].as_bool(), v["can"]["ring"].as_bool()), (Some("cable"), Some(true), Some(false)));
        assert_eq!(v["battery"]["charge"], 64);
        let switches: Vec<_> = v["permits"].as_array().unwrap().iter().map(|p| (p["key"].as_str().unwrap().to_owned(), p["on"].as_bool().unwrap())).collect();
        assert_eq!(switches, [("control".to_owned(), true), ("files".to_owned(), false)], "only this side's switches");
        d.linked = false;
        assert_eq!((d.via(), d.takes(SHARE)), ("", false));
        assert_eq!(url_encode("Sam's PC"), "Sam%27s%20PC");
        assert_eq!(text(&json!(["a", 1]), 1).unwrap_err(), "That request was missing something");
        assert_eq!(strings(&json!(["a", 2, "b"])), ["a", "b"]);
    }
}
