//! Windows' own notifications (toasts) under Vyoma's name and icon, with buttons and a reply box:
//! the pairing question (Accept / Reject), the phone's notifications (Reply, their actions; taken
//! down when the phone dismisses them, and dismissed on the phone when closed here), and the choice
//! of phone for Explorer's Send to.
//!
//! A program that isn't from the Store gets toasts by registering a name for itself under the
//! user's own registry (`AppUserModelId\dev.vyoma.Vyoma`: shown name, icon) and a COM class that
//! Windows calls when a button is pressed ([Callback], registered while the agent runs; if it isn't
//! running Windows starts it with `-Embedding`). No administrator, no Start menu shortcut.
//! Where toasts don't work (Wine, an old Windows) [show] says so and the caller falls back to the
//! notification area's balloons or a dialog.
//! Built and tested on CI and under Wine only; not yet run on a real Windows PC.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result};
use tokio::sync::mpsc;
use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::TypedEventHandler;
use windows::UI::Notifications::{NotificationSetting, ToastDismissalReason, ToastDismissedEventArgs, ToastNotification, ToastNotificationManager, ToastNotifier};
use windows::Win32::Foundation::{CLASS_E_NOAGGREGATION, E_POINTER};
use windows::Win32::System::Com::{CLSCTX_LOCAL_SERVER, COINIT_MULTITHREADED, CoInitializeEx, CoRegisterClassObject, IClassFactory, IClassFactory_Impl, REGCLS_MULTIPLEUSE};
use windows::Win32::UI::Notifications::{INotificationActivationCallback, INotificationActivationCallback_Impl, NOTIFICATION_USER_INPUT_DATA};
use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
use windows::core::{BOOL, GUID, HSTRING, IUnknown, Interface, PCWSTR, Ref, implement};

use crate::win;

/// Vyoma's name for Windows' notifications.
pub const AUMID: &str = "dev.vyoma.Vyoma";
/// The class Windows calls when a toast's button is pressed.
const CLSID: GUID = GUID::from_u128(0x9750d8b4_70fb_4276_bb4d_a7d118943759);
const CLSID_TEXT: &str = "{9750D8B4-70FB-4276-BB4D-A7D118943759}";
/// The program's own icon, as a picture Windows can show beside each toast.
const ICON: &[u8] = include_bytes!("../dist/vyoma.png");
/// The reply box's id in a toast, and the most buttons Windows shows.
const REPLY: &str = "reply";
const MAX_BUTTONS: usize = 5;
/// Toasts kept here so their Dismissed event still reaches us; the oldest go past this.
const KEPT: usize = 64;

fn aumid_key() -> String {
    format!(r"Software\Classes\AppUserModelId\{AUMID}")
}

fn clsid_key() -> String {
    format!(r"Software\Classes\CLSID\{CLSID_TEXT}")
}

/// What the person did with one of ours: pressed a button or the toast itself (its arguments, and
/// what they typed in the reply box), or closed it.
#[derive(Debug, PartialEq)]
pub enum Heard {
    Pressed { args: String, reply: Option<String> },
    Closed { args: String },
}

/// Whether a toast went up.
#[derive(Debug, PartialEq)]
pub enum Shown {
    Yes,
    /// The person turned Vyoma's notifications off in Windows' settings (or Focus blocks them).
    Off,
    /// No toasts here: say it another way.
    Unavailable,
}

pub struct Button {
    pub label: String,
    pub args: String,
    /// Sends what's in the reply box with it.
    pub with_reply: bool,
}

/// One toast. [tag] and [group] name it, to replace or take it down later.
#[derive(Default)]
pub struct Toast {
    pub tag: String,
    pub group: String,
    pub title: String,
    pub body: String,
    /// The small line under the text (which app, which phone).
    pub attribution: String,
    /// Pressing the toast itself.
    pub launch: String,
    /// A reply box with this hint.
    pub reply: Option<String>,
    pub buttons: Vec<Button>,
    /// Stays on screen until answered (the pairing question).
    pub urgent: bool,
    /// A phone call ringing: Windows' call look (shown expanded, on screen until answered).
    pub ringing: bool,
}

pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // XML can't carry most control characters at all.
            c if (c as u32) < 0x20 && !matches!(c, '\n' | '\t') => {}
            c => out.push(c),
        }
    }
    out
}

impl Toast {
    pub fn xml(&self) -> String {
        let mut x = format!("<toast launch=\"{}\" activationType=\"foreground\"", escape(&self.launch));
        if self.ringing {
            x.push_str(" scenario=\"incomingCall\"");
        } else if self.urgent {
            x.push_str(" scenario=\"reminder\"");
        }
        x.push_str("><visual><binding template=\"ToastGeneric\">");
        for t in [&self.title, &self.body].into_iter().filter(|t| !t.is_empty()) {
            x += &format!("<text>{}</text>", escape(t));
        }
        if !self.attribution.is_empty() {
            x += &format!("<text placement=\"attribution\">{}</text>", escape(&self.attribution));
        }
        x.push_str("</binding></visual>");
        if self.reply.is_some() || !self.buttons.is_empty() {
            x.push_str("<actions>");
            if let Some(hint) = &self.reply {
                x += &format!("<input id=\"{REPLY}\" type=\"text\" placeHolderContent=\"{}\"/>", escape(hint));
            }
            for b in self.buttons.iter().take(MAX_BUTTONS) {
                x += &format!("<action content=\"{}\" arguments=\"{}\" activationType=\"background\"", escape(&b.label), escape(&b.args));
                if b.with_reply && self.reply.is_some() {
                    x += &format!(" hint-inputId=\"{REPLY}\"");
                }
                x.push_str("/>");
            }
            x.push_str("</actions>");
        }
        if self.urgent || self.ringing {
            // A reminder rings like an alarm and a call loops Windows' ringtone unless told
            // otherwise: one chime (the phone is ringing already).
            x.push_str("<audio src=\"ms-winsoundevent:Notification.Default\"/>");
        }
        x.push_str("</toast>");
        x
    }
}

/// `k=v&k=v` with both sides percent-encoded: a button's arguments.
pub fn args(pairs: &[(&str, &str)]) -> String {
    let enc = |s: &str| s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect::<String>();
    pairs.iter().map(|(k, v)| format!("{}={}", enc(k), enc(v))).collect::<Vec<_>>().join("&")
}

pub fn parse_args(s: &str) -> HashMap<String, String> {
    let dec = |s: &str| {
        let b = s.as_bytes();
        let mut out = Vec::with_capacity(b.len());
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'%'
                && let Some(v) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(v);
                i += 3;
                continue;
            }
            out.push(b[i]);
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    };
    s.split('&').filter(|p| !p.is_empty()).map(|p| p.split_once('=').map_or((dec(p), String::new()), |(k, v)| (dec(k), dec(v)))).collect()
}

/// A short stable name for a tag or group (Windows allows 64 characters): FNV-1a in hex.
pub fn short(s: &str) -> String {
    format!("{:016x}", s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3)))
}

static SINK: OnceLock<mpsc::UnboundedSender<Heard>> = OnceLock::new();
static NOTIFIER: OnceLock<ToastNotifier> = OnceLock::new();
static SHOWING: Mutex<Vec<((String, String), ToastNotification)>> = Mutex::new(Vec::new());

/// Registers Vyoma for toasts and listens for their buttons; false if toasts won't work here.
/// Tests (`VYOMA_DIR` set) leave the registry alone unless `VYOMA_TOASTS=1`.
pub fn start(heard: mpsc::UnboundedSender<Heard>) -> bool {
    if std::env::var_os("VYOMA_DIR").is_some() && std::env::var("VYOMA_TOASTS").as_deref() != Ok("1") {
        return false;
    }
    match setup(heard) {
        Ok(()) => true,
        Err(e) => {
            tracing::info!("no toasts here, balloons instead: {e:#}");
            false
        }
    }
}

fn setup(heard: mpsc::UnboundedSender<Heard>) -> Result<()> {
    register()?;
    // SAFETY: a NUL-terminated id for this process, before any window.
    unsafe { SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(AUMID)) }.context("no app id")?;
    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID)).context("no toast notifier")?;
    let _ = SINK.set(heard);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new().name("vyoma-toasts".into()).spawn(move || {
        // SAFETY: this thread joins the multithreaded apartment and registers our class there for
        // the life of the process; Windows calls it on its own threads.
        let registered = unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let factory: IClassFactory = Factory.into();
            CoRegisterClassObject(&CLSID, &factory, CLSCTX_LOCAL_SERVER, REGCLS_MULTIPLEUSE)
        };
        let ok = registered.is_ok();
        let _ = tx.send(registered.map(|_| ()));
        if ok {
            loop {
                std::thread::park();
            }
        }
    })?;
    rx.recv().context("the toasts' thread stopped")?.context("Windows wouldn't take the button handler")?;
    let _ = NOTIFIER.set(notifier);
    Ok(())
}

/// Vyoma's name and icon for toasts, and the class for their buttons, in the user's registry.
fn register() -> Result<()> {
    let exe = std::env::current_exe()?;
    let icon = win::data_dir().join("vyoma.png");
    if std::fs::read(&icon).ok().as_deref() != Some(ICON) {
        std::fs::create_dir_all(win::data_dir())?;
        std::fs::write(&icon, ICON)?;
    }
    let key = aumid_key();
    win::set_user_value(&key, Some("DisplayName"), "Vyoma")?;
    win::set_user_value(&key, Some("IconUri"), &icon.to_string_lossy())?;
    win::set_user_value(&key, Some("CustomActivator"), CLSID_TEXT)?;
    win::set_user_value(&format!(r"{}\LocalServer32", clsid_key()), None, &format!("\"{}\"", exe.display()))?;
    Ok(())
}

/// `--uninstall`: takes the registration out again (toasts already shown go with it).
pub fn unregister() {
    if let Ok(history) = ToastNotificationManager::History() {
        let _ = history.ClearWithId(&HSTRING::from(AUMID));
    }
    win::delete_user_key(&aumid_key());
    win::delete_user_key(&clsid_key());
}

pub fn show(t: &Toast) -> Shown {
    let Some(notifier) = NOTIFIER.get() else { return Shown::Unavailable };
    if notifier.Setting().is_ok_and(|s| s != NotificationSetting::Enabled) {
        return Shown::Off;
    }
    match put_up(notifier, t) {
        Ok(()) => Shown::Yes,
        Err(e) => {
            tracing::info!("toast not shown: {e}");
            Shown::Unavailable
        }
    }
}

fn put_up(notifier: &ToastNotifier, t: &Toast) -> windows::core::Result<()> {
    let doc = XmlDocument::new()?;
    doc.LoadXml(&HSTRING::from(t.xml()))?;
    let toast = ToastNotification::CreateToastNotification(&doc)?;
    if !t.tag.is_empty() {
        toast.SetTag(&HSTRING::from(&t.tag))?;
    }
    if !t.group.is_empty() {
        toast.SetGroup(&HSTRING::from(&t.group))?;
    }
    if let Some(sink) = SINK.get() {
        let (sink, launch) = (sink.clone(), t.launch.clone());
        toast.Dismissed(&TypedEventHandler::new(move |_: Ref<ToastNotification>, e: Ref<ToastDismissedEventArgs>| {
            // Closed by the person; not the banner timing out into the notification centre.
            if e.as_ref().is_some_and(|e| e.Reason().is_ok_and(|r| r == ToastDismissalReason::UserCanceled)) {
                let _ = sink.send(Heard::Closed { args: launch.clone() });
            }
            Ok(())
        }))?;
    }
    notifier.Show(&toast)?;
    let mut showing = SHOWING.lock().unwrap();
    let name = (t.tag.clone(), t.group.clone());
    showing.retain(|(n, _)| *n != name);
    showing.push((name, toast));
    let extra = showing.len().saturating_sub(KEPT);
    showing.drain(..extra);
    Ok(())
}

/// Takes one of ours down (the phone dismissed it, the pairing was answered elsewhere).
pub fn remove(tag: &str, group: &str) {
    SHOWING.lock().unwrap().retain(|((t, g), _)| !(t == tag && g == group));
    if NOTIFIER.get().is_none() {
        return;
    }
    if let Ok(history) = ToastNotificationManager::History() {
        let _ = history.RemoveGroupedTagWithId(&HSTRING::from(tag), &HSTRING::from(group), &HSTRING::from(AUMID));
    }
}

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(&self, outer: Ref<IUnknown>, iid: *const GUID, object: *mut *mut core::ffi::c_void) -> windows::core::Result<()> {
        if object.is_null() || iid.is_null() {
            return Err(E_POINTER.into());
        }
        // SAFETY: checked non-null above; COM's contract for CreateInstance.
        unsafe { *object = std::ptr::null_mut() };
        if !outer.is_null() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let callback: INotificationActivationCallback = Callback.into();
        // SAFETY: as above.
        unsafe { callback.query(iid, object).ok() }
    }

    fn LockServer(&self, _: BOOL) -> windows::core::Result<()> {
        Ok(())
    }
}

/// Windows' call when a toast or one of its buttons is pressed.
#[implement(INotificationActivationCallback)]
struct Callback;

impl INotificationActivationCallback_Impl for Callback_Impl {
    fn Activate(&self, _app: &PCWSTR, args: &PCWSTR, data: *const NOTIFICATION_USER_INPUT_DATA, count: u32) -> windows::core::Result<()> {
        // SAFETY: Windows passes NUL-terminated strings and [count] entries at [data].
        let (args, inputs) = unsafe {
            let args = if args.is_null() { String::new() } else { args.to_string().unwrap_or_default() };
            let inputs: Vec<(String, String)> = if data.is_null() {
                Vec::new()
            } else {
                std::slice::from_raw_parts(data, count as usize).iter().map(|d| (d.Key.to_string().unwrap_or_default(), d.Value.to_string().unwrap_or_default())).collect()
            };
            (args, inputs)
        };
        let reply = inputs.into_iter().find(|(k, _)| k == REPLY).map(|(_, v)| v);
        if let Some(sink) = SINK.get() {
            let _ = sink.send(Heard::Pressed { args, reply });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_carry_their_arguments_safely() {
        let a = args(&[("do", "pair"), ("id", "a&b=c"), ("key", "1A 2B")]);
        assert!(!a.contains(' ') && a.matches('&').count() == 2, "{a}");
        let back = parse_args(&a);
        assert_eq!((back["do"].as_str(), back["id"].as_str(), back["key"].as_str()), ("pair", "a&b=c", "1A 2B"));
        assert_eq!(parse_args("open")["open"], "");
        assert_eq!(parse_args("x=%zz")["x"], "%zz", "broken escapes stay as they are");
        assert_eq!(short("phone-1").len(), 16);
        assert_ne!(short("phone-1"), short("phone-2"));
    }

    #[test]
    fn a_phones_text_cant_break_the_toast() {
        let t = Toast {
            title: "<b>Mum</b> & \"Dad\"".into(),
            body: "line\u{7}one".into(),
            attribution: "WhatsApp \u{b7} Pixel".into(),
            launch: args(&[("do", "open")]),
            reply: Some("Reply to Mum".into()),
            buttons: vec![Button { label: "Send".into(), args: args(&[("do", "reply")]), with_reply: true }, Button { label: "Mark as read".into(), args: "x".into(), with_reply: false }],
            ..Default::default()
        };
        let x = t.xml();
        assert!(x.contains("<text>&lt;b&gt;Mum&lt;/b&gt; &amp; &quot;Dad&quot;</text>"), "{x}");
        assert!(x.contains("<text>lineone</text>"), "control characters dropped");
        assert!(x.contains("hint-inputId=\"reply\"") && x.matches("<action ").count() == 2);
        assert!(!x.contains("scenario"));
        let many = Toast { buttons: (0..9).map(|i| Button { label: i.to_string(), args: String::new(), with_reply: false }).collect(), urgent: true, ..Default::default() };
        let x = many.xml();
        assert_eq!(x.matches("<action ").count(), MAX_BUTTONS, "Windows shows five at most");
        assert!(x.contains("scenario=\"reminder\""));
    }

    /// Windows parses what we make. Wine has no WinRT XML at all: skipped there, so this one counts
    /// on CI's Windows.
    #[test]
    fn windows_reads_the_xml() {
        let t = Toast { title: "Pair with Pixel?".into(), body: "Code 1A2B".into(), reply: Some("hint".into()), buttons: vec![Button { label: "Accept".into(), args: "a=1&b=2".into(), with_reply: false }], urgent: true, ..Default::default() };
        let doc = match XmlDocument::new() {
            Ok(d) => d,
            Err(e) if e.code() == windows::Win32::Foundation::REGDB_E_CLASSNOTREG => return eprintln!("no WinRT XML here (Wine)"),
            Err(e) => panic!("{e}"),
        };
        doc.LoadXml(&HSTRING::from(t.xml())).unwrap();
        assert_eq!(doc.GetElementsByTagName(&HSTRING::from("action")).unwrap().Length().unwrap(), 1);
    }
}
