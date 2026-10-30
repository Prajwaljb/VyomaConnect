//! PC screenshot to phone. Spectacle (always on Plasma) captures in the background with no
//! prompt or notification; the image travels as a KDE Connect payload on its own TLS socket,
//! which only the phone this link belongs to may open.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

use crate::identity::Identity;
use crate::link::{self, Sender};

const PAYLOAD_PORTS: std::ops::RangeInclusive<u16> = 1739..=1764;

/// Captures the whole desktop and offers it to the phone. On failure the phone gets an error
/// packet instead, so it never waits on a spinner.
pub async fn capture_and_send(sender: &Sender, me: &Identity, pinned: &[u8]) -> Result<()> {
    let image = match capture().await {
        Ok(bytes) => bytes,
        Err(e) => {
            sender.send(&link::packet("vyoma.screenshot", json!({ "error": format!("{e:#}") }))).await?;
            return Err(e);
        }
    };
    let listener = bind_payload_port().await?;
    let port = listener.local_addr()?.port();
    let mut announce: Value = link::packet("vyoma.screenshot", json!({ "format": "webp" }));
    announce["payloadSize"] = json!(image.len());
    announce["payloadTransferInfo"] = json!({ "port": port });
    sender.send(&announce).await?;

    // The phone connects back for the bytes. As on the link, the side that accepts is the TLS server.
    let (tcp, _) = tokio::time::timeout(Duration::from_secs(15), listener.accept()).await.context("phone never fetched the screenshot")??;
    tcp.set_nodelay(true)?;
    let acceptor = TlsAcceptor::from(Arc::new(link::server_config(me)?));
    let mut tls = tokio::time::timeout(Duration::from_secs(10), acceptor.accept(tcp)).await.context("payload TLS timed out")??;
    let peer = tls.get_ref().1.peer_certificates().and_then(|c| c.first()).context("phone sent no certificate")?;
    if peer.as_ref() != pinned {
        bail!("payload peer isn't the paired phone");
    }
    tls.write_all(&image).await?;
    tls.shutdown().await?;
    Ok(())
}

async fn capture() -> Result<Vec<u8>> {
    // Private to this user (XDG_RUNTIME_DIR is 0700), and deleted as soon as it's read.
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    // WebP: about a quarter of the PNG, and still crisp on text at Spectacle's default quality.
    let path = dir.join(format!("vyoma-shot-{}.webp", std::process::id()));
    let status = tokio::time::timeout(
        Duration::from_secs(15),
        tokio::process::Command::new("spectacle")
            .args(["--background", "--nonotify", "--fullscreen", "--output"])
            .arg(&path)
            .kill_on_drop(true)
            .status(),
    )
    .await
    .context("Spectacle took too long")?
    .context("Spectacle isn't installed")?;
    let bytes = tokio::fs::read(&path).await;
    let _ = tokio::fs::remove_file(&path).await;
    if !status.success() {
        bail!("Spectacle couldn't capture the screen");
    }
    bytes.context("Spectacle saved no image")
}

async fn bind_payload_port() -> Result<TcpListener> {
    for port in PAYLOAD_PORTS {
        // kdeconnectd uses the same range for its own transfers; take the first free one.
        if let Ok(l) = TcpListener::bind(("0.0.0.0", port)).await {
            return Ok(l);
        }
    }
    bail!("no free payload port")
}
