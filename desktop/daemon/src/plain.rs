//! Errors as people read them in a notification: one plain sentence and what to do next, never the
//! chain of internals (tool names, exit codes, D-Bus errors). The caller logs the raw error.

use anyhow::Error;

/// The whole chain, lowercased, to look for known causes in.
fn chain(e: &Error) -> String {
    format!("{e:#}").to_lowercase()
}

/// A send that stopped, or files that couldn't be read for it.
pub fn send(e: &Error) -> String {
    let all = chain(e);
    if let Some(cause) = e.chain().map(ToString::to_string).find(|c| c.contains("zip it and send that instead")) {
        return capitalised(&cause) + ".";
    }
    if all.contains("can't read") || all.contains("isn't a file or a folder") || all.contains("no file name") {
        return "This PC couldn't read one of the files. Check that it's still there, then try again.".into();
    }
    if all.contains("stopping") {
        return "Vyoma was closing. Open it again, then send once more.".into();
    }
    "The phone stopped taking the files. Check that it's still connected, then try again.".into()
}

/// A file from the phone that didn't arrive.
pub fn receive(e: &Error) -> String {
    let all = chain(e);
    if all.contains("couldn't make") || all.contains("no space") || all.contains("permission denied") {
        return "This PC couldn't save it in Downloads. Check there's free space, then send it again.".into();
    }
    "The transfer stopped part way. Send it again from the phone.".into()
}

/// The phone's files that didn't open.
pub fn mount(e: &Error) -> String {
    let all = chain(e);
    if let Some(cause) = e.chain().map(ToString::to_string).find(|c| c.starts_with("Phone files need")) {
        return cause + ".";
    }
    if all.contains("permission denied") || all.contains("host key") {
        return "The phone didn't let this PC in. Open Vyoma on the phone, then try again.".into();
    }
    if all.contains("refused") || all.contains("timed out") || all.contains("no route") || all.contains("20 s") {
        return "The phone didn't answer. Check that it's on the same Wi-Fi or plugged in, then try again.".into();
    }
    // 203: systemd couldn't start sshfs at all.
    if all.contains("command not found") || all.contains("status: 203") || all.contains("systemd-run isn't there") {
        return "This PC is missing an add-on for phone files. Open Vyoma, go to This PC and choose Install.".into();
    }
    "The phone's files didn't open. Try again in a moment.".into()
}

/// The call's sound that stayed on the phone.
pub fn call_audio(e: &Error) -> String {
    let all = chain(e);
    if all.contains("isn't paired with the phone over bluetooth") {
        return "This PC isn't paired with the phone over Bluetooth. Pair them in Bluetooth settings to hear calls here.".into();
    }
    if all.contains("which bluetooth device") {
        return "More than one phone is paired with this PC over Bluetooth. Remove the ones you don't use in Bluetooth settings.".into();
    }
    if all.contains("system bus") || all.contains("bluetooth isn't running") {
        return "Bluetooth is off on this PC. Turn it on to hear calls here.".into();
    }
    if all.contains("call mode") {
        return "This PC couldn't switch its sound to the call. Talk on the phone this time.".into();
    }
    "The phone didn't connect to this PC over Bluetooth. Check Bluetooth is on on both, then try the next call.".into()
}

/// The reply box that didn't open.
pub fn reply(e: &Error) -> String {
    if chain(e).contains("zenity") {
        return "This desktop can't show a reply box. Open Vyoma, go to This PC and choose Install, or reply on the phone.".into();
    }
    "The reply box didn't open. Reply on the phone this time.".into()
}

fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Context, anyhow};

    #[test]
    fn says_it_plainly() {
        let e = Err::<(), _>(anyhow!("exit status: 1: read: Connection reset by peer")).context("sshfs ended").unwrap_err();
        let m = mount(&e);
        assert!(!m.contains("sshfs") && !m.contains("status") && m.ends_with('.'), "{m}");
        let e = anyhow!("Phone files need Wi-Fi or the USB cable (it's on Bluetooth only)");
        assert_eq!(mount(&e), "Phone files need Wi-Fi or the USB cable (it's on Bluetooth only).");
        let e = Err::<(), _>(anyhow!("org.bluez.Error.Failed")).context("the phone didn't take this PC as its headset").unwrap_err();
        assert!(!call_audio(&e).contains("bluez"));
        let e = anyhow!("photos has more than 2000 files: zip it and send that instead");
        assert_eq!(send(&e), "Photos has more than 2000 files: zip it and send that instead.");
        let e = Err::<(), _>(anyhow!("stopping")).context("a.txt wasn't sent").unwrap_err();
        assert!(send(&e).starts_with("Vyoma was closing"));
    }
}
