//! The window's way in (the Linux app's D-Bus interface, as the Mac's socket): a named pipe,
//! `\\.\pipe\vyoma-<user>` (`VYOMA_PIPE` overrides), which only this user can open and never
//! from another computer. One JSON line per request and one per reply, as on the Mac (see the
//! window's `socket.rs` for the shapes); a connection that asks for `Events` gets one line per
//! change instead, until it closes.
//!
//! The state lives in the agent's loop, so each request goes there ([Request]) and its answer
//! comes back; a connection never touches it.

use std::sync::OnceLock;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tokio::sync::{broadcast, mpsc, oneshot};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
use windows_sys::Win32::Security::{GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use crate::win::{from_wide, wide};

/// Requests and changes are small; a line past this isn't from the window.
const MAX_LINE: usize = 1 << 20;

pub struct Request {
    pub method: String,
    pub args: Value,
    pub reply: oneshot::Sender<Result<Value, String>>,
}

/// The pipe's name; the window works it out the same way (`gui/src/socket.rs`).
pub fn pipe_name() -> String {
    if let Ok(p) = std::env::var("VYOMA_PIPE") {
        return p;
    }
    let user: String = std::env::var("USERNAME").unwrap_or_default().chars().filter(|c| c.is_alphanumeric() || "-_.".contains(*c)).collect();
    format!(r"\\.\pipe\vyoma-{user}")
}

/// Listens on [name] for as long as the agent runs. Changes sent on [changes] reach every
/// connection that asked for events. Fails at once if someone else made the pipe first.
pub async fn serve(name: String, requests: mpsc::Sender<Request>, changes: broadcast::Sender<Value>) -> Result<()> {
    let mut server = create(&name, true).with_context(|| format!("couldn't make {name} (is Vyoma already running?)"))?;
    loop {
        server.connect().await?;
        let connected = server;
        // The next one waits before this one's handled: a client always finds an instance.
        server = create(&name, false)?;
        let (requests, changes) = (requests.clone(), changes.subscribe());
        tokio::spawn(async move {
            if let Err(e) = connection(connected, requests, changes).await {
                tracing::debug!("window connection: {e:#}");
            }
        });
    }
}

fn create(name: &str, first: bool) -> Result<NamedPipeServer> {
    let attrs = this_user_only()?;
    let mut options = ServerOptions::new();
    options.first_pipe_instance(first).reject_remote_clients(true);
    // SAFETY: [attrs] points at a SECURITY_ATTRIBUTES kept for the life of the process.
    Ok(unsafe { options.create_with_security_attributes_raw(name, attrs as *mut std::ffi::c_void) }?)
}

/// A security descriptor that lets this user, and nobody else, open the pipe (a pipe's default
/// lets everyone read). Made once and kept.
fn this_user_only() -> Result<*const SECURITY_ATTRIBUTES> {
    static ATTRS: OnceLock<usize> = OnceLock::new();
    if let Some(&a) = ATTRS.get() {
        return Ok(a as *const SECURITY_ATTRIBUTES);
    }
    let sid = user_sid()?;
    let sddl = wide(&format!("D:P(A;;GA;;;{sid})"));
    let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: a NUL-terminated SDDL string; the descriptor is LocalAlloc'd and kept for good.
    if unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut sd, std::ptr::null_mut()) } == 0 {
        bail!("no security descriptor: {}", std::io::Error::last_os_error());
    }
    let attrs = Box::leak(Box::new(SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd, bInheritHandle: 0 }));
    let a = *ATTRS.get_or_init(|| attrs as *const SECURITY_ATTRIBUTES as usize);
    Ok(a as *const SECURITY_ATTRIBUTES)
}

/// This process's user, as "S-1-5-21-…".
pub(crate) fn user_sid() -> Result<String> {
    // SAFETY: the token is opened for query and closed; the buffer is sized by the first call;
    // the string form is LocalAlloc'd and freed after copying.
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            bail!("no process token: {}", std::io::Error::last_os_error());
        }
        let mut len = 0u32;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut len);
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        let ok = GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len);
        CloseHandle(token);
        if ok == 0 {
            bail!("no user in the token: {}", std::io::Error::last_os_error());
        }
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut s: *mut u16 = std::ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut s) == 0 {
            bail!("no SID string: {}", std::io::Error::last_os_error());
        }
        let sid = from_wide(s);
        LocalFree(s.cast());
        Ok(sid)
    }
}

async fn connection(stream: impl AsyncRead + AsyncWrite, requests: mpsc::Sender<Request>, mut changes: broadcast::Receiver<Value>) -> Result<()> {
    let (read, mut write) = tokio::io::split(stream);
    let mut lines = BufReader::new(read).lines();
    while let Some(line) = lines.next_line().await? {
        if line.len() > MAX_LINE {
            break;
        }
        let Ok(req) = serde_json::from_str::<Value>(&line) else {
            write.write_all(format!("{}\n", json!({ "error": "That wasn't a request" })).as_bytes()).await?;
            continue;
        };
        let method = req["method"].as_str().unwrap_or("").to_owned();
        if method == "Events" {
            loop {
                match changes.recv().await {
                    Ok(change) => write.write_all(format!("{change}\n").as_bytes()).await?,
                    // Fell behind: what it missed, the window rereads on the next Changed.
                    Err(broadcast::error::RecvError::Lagged(_)) => write.write_all(format!("{}\n", json!({ "event": "Changed" })).as_bytes()).await?,
                    Err(broadcast::error::RecvError::Closed) => return Ok(()),
                }
            }
        }
        let (reply, answer) = oneshot::channel();
        requests.send(Request { method, args: req["args"].clone(), reply }).await.context("stopping")?;
        let line = match answer.await {
            Ok(Ok(v)) => json!({ "ok": v }),
            Ok(Err(e)) => json!({ "error": e }),
            Err(_) => json!({ "error": "Vyoma is restarting. Try again in a moment" }),
        };
        write.write_all(format!("{line}\n").as_bytes()).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;
    use tokio::net::windows::named_pipe::ClientOptions;

    #[test]
    fn the_pipe_is_named_for_its_user() {
        let name = pipe_name();
        assert!(name.starts_with(r"\\.\pipe\vyoma-"), "{name}");
        assert!(!name["\\\\.\\pipe\\".len()..].contains('\\'));
    }

    #[test]
    fn only_this_user_may_open_it() {
        let sid = user_sid().unwrap();
        assert!(sid.starts_with("S-1-"), "{sid}");
        assert!(!this_user_only().unwrap().is_null());
    }

    #[tokio::test]
    async fn requests_reach_the_loop_and_events_stream() {
        let name = format!(r"\\.\pipe\vyoma-test-{}", std::process::id());
        let (requests, mut asked) = mpsc::channel(4);
        let (changes, _) = broadcast::channel(16);
        tokio::spawn(serve(name.clone(), requests, changes.clone()));
        // The loop's side: answers Pair with an error, anything else with the method's name.
        tokio::spawn(async move {
            while let Some(r) = asked.recv().await {
                let _ = r.reply.send(if r.method == "Pair" { Err("Not nearby".into()) } else { Ok(json!(r.method)) });
            }
        });
        let open = || async {
            for _ in 0..100 {
                if let Ok(c) = ClientOptions::new().open(&name) {
                    return c;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            panic!("no pipe");
        };
        let ask = |line: Value| async move {
            let mut c = open().await;
            c.write_all(format!("{line}\n").as_bytes()).await.unwrap();
            let mut out = Vec::new();
            let mut byte = [0u8; 1];
            while c.read(&mut byte).await.unwrap() == 1 && byte[0] != b'\n' {
                out.push(byte[0]);
            }
            serde_json::from_slice::<Value>(&out).unwrap()
        };
        assert_eq!(ask(json!({ "method": "Phones", "args": [] })).await, json!({ "ok": "Phones" }));
        assert_eq!(ask(json!({ "method": "Pair", "args": ["x"] })).await, json!({ "error": "Not nearby" }));

        let mut events = BufReader::new(open().await);
        events.get_mut().write_all(b"{\"method\":\"Events\"}\n").await.unwrap();
        let mut line = String::new();
        let read = async {
            events.read_line(&mut line).await.unwrap();
        };
        let send = async {
            loop {
                let _ = changes.send(json!({ "event": "Changed" }));
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        };
        tokio::select! { _ = read => {}, _ = send => {} }
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap(), json!({ "event": "Changed" }));
    }
}
