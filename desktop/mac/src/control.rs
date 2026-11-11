//! The window's way in (the Linux app's D-Bus interface, as a local socket): `control.sock` in
//! Vyoma's folder, which only this user can open. One JSON line per request and one per reply
//! (see the window's `socket.rs` for the shapes); a connection that asks for `Events` gets one
//! line per change instead, until it closes.
//!
//! The state lives in the daemon's loop, so each request goes there ([Request]) and its answer
//! comes back; a connection never touches it.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc, oneshot};

/// Requests and changes are small; a line past this isn't from the window.
const MAX_LINE: usize = 1 << 20;

pub struct Request {
    pub method: String,
    pub args: Value,
    pub reply: oneshot::Sender<Result<Value, String>>,
}

pub fn socket_path(dir: &Path) -> PathBuf {
    dir.join("control.sock")
}

/// Listens on [path] for as long as the daemon runs. Changes sent on [changes] reach every
/// connection that asked for events.
pub async fn serve(path: PathBuf, requests: mpsc::Sender<Request>, changes: broadcast::Sender<Value>) -> Result<()> {
    // A socket left from a run that crashed; a second daemon fails earlier, at the network ports.
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).with_context(|| format!("couldn't listen on {}", path.display()))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    loop {
        let (stream, _) = listener.accept().await?;
        let (requests, changes) = (requests.clone(), changes.subscribe());
        tokio::spawn(async move {
            if let Err(e) = connection(stream, requests, changes).await {
                tracing::debug!("window connection: {e:#}");
            }
        });
    }
}

async fn connection(stream: UnixStream, requests: mpsc::Sender<Request>, mut changes: broadcast::Receiver<Value>) -> Result<()> {
    let (read, mut write) = stream.into_split();
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

/// The [i]th argument as text, or the reply's error.
pub fn text(args: &Value, i: usize) -> Result<String, String> {
    args[i].as_str().map(str::to_owned).ok_or_else(|| "That request was missing something".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn requests_reach_the_loop_and_events_stream() {
        let dir = std::env::temp_dir().join(format!("vyoma-control-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = socket_path(&dir);
        let (requests, mut asked) = mpsc::channel(4);
        let (changes, _) = broadcast::channel(16);
        tokio::spawn(serve(path.clone(), requests, changes.clone()));
        while !path.exists() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "this user's alone");

        // The loop's side: answers Pair with an error, anything else with the method's name.
        tokio::spawn(async move {
            while let Some(r) = asked.recv().await {
                let _ = r.reply.send(if r.method == "Pair" { Err("Not nearby".into()) } else { Ok(json!(r.method)) });
            }
        });
        let ask = |line: Value| {
            let path = path.clone();
            async move {
                let mut s = UnixStream::connect(&path).await.unwrap();
                s.write_all(format!("{line}\n").as_bytes()).await.unwrap();
                let mut out = String::new();
                BufReader::new(s).read_line(&mut out).await.unwrap();
                serde_json::from_str::<Value>(&out).unwrap()
            }
        };
        assert_eq!(ask(json!({ "method": "Phones", "args": [] })).await, json!({ "ok": "Phones" }));
        assert_eq!(ask(json!({ "method": "Pair", "args": ["x"] })).await, json!({ "error": "Not nearby" }));

        let mut events = UnixStream::connect(&path).await.unwrap();
        events.write_all(b"{\"method\":\"Events\"}\n").await.unwrap();
        let mut events = BufReader::new(events);
        // Subscribed once the request is read; keep sending until it's heard.
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
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
