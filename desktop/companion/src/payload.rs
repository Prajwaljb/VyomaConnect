//! KDE Connect payloads from our side: announce a packet with `payloadSize` and a port, wait for
//! the phone to connect back, prove it's the paired phone (pinned certificate), then stream.
//! As on the link, the side that accepts is the TLS server.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

use crate::identity::Identity;
use crate::link::Sender;

const PAYLOAD_PORTS: std::ops::RangeInclusive<u16> = 1739..=1764;

pub async fn offer(sender: &Sender, me: &Identity, pinned: &[u8], mut packet: Value, size: u64, mut source: impl AsyncRead + Unpin) -> Result<()> {
    let listener = bind().await?;
    packet["payloadSize"] = json!(size);
    packet["payloadTransferInfo"] = json!({ "port": listener.local_addr()?.port() });
    sender.send(&packet).await?;

    let (tcp, _) = tokio::time::timeout(Duration::from_secs(15), listener.accept()).await.context("phone never fetched the payload")??;
    tcp.set_nodelay(true)?;
    let acceptor = TlsAcceptor::from(Arc::new(crate::link::server_config(me)?));
    let mut tls = tokio::time::timeout(Duration::from_secs(10), acceptor.accept(tcp)).await.context("payload TLS timed out")??;
    let peer = tls.get_ref().1.peer_certificates().and_then(|c| c.first()).context("phone sent no certificate")?;
    if peer.as_ref() != pinned {
        bail!("payload peer isn't the paired phone");
    }
    tokio::io::copy(&mut source, &mut tls).await?;
    tls.shutdown().await?;
    Ok(())
}

async fn bind() -> Result<TcpListener> {
    for port in PAYLOAD_PORTS {
        // kdeconnectd uses the same range for its own transfers; take the first free one.
        if let Ok(l) = TcpListener::bind(("0.0.0.0", port)).await {
            return Ok(l);
        }
    }
    bail!("no free payload port")
}
