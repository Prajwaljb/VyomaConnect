//! KDE Connect payloads. Sending: announce a packet with `payloadSize` and a port, wait for the
//! phone to connect back, prove it's the paired phone (pinned certificate), then stream.
//! Receiving: connect to the port the phone announced. As on the link, the side that accepts is
//! the TLS server.

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{TlsAcceptor, TlsConnector};

use crate::identity::Identity;
use crate::link::Sender;

const PAYLOAD_PORTS: std::ops::RangeInclusive<u16> = 1739..=1764;

pub async fn offer(sender: &Sender, me: &Identity, pinned: &[u8], packet: Value, size: u64, source: impl AsyncRead + Unpin) -> Result<()> {
    offer_sized(sender, me, pinned, packet, size as i64, source).await
}

/// An open-ended payload (`payloadSize` -1): streams until [source] ends or the phone hangs up.
pub async fn offer_stream(sender: &Sender, me: &Identity, pinned: &[u8], packet: Value, source: impl AsyncRead + Unpin) -> Result<()> {
    offer_sized(sender, me, pinned, packet, -1, source).await
}

async fn offer_sized(sender: &Sender, me: &Identity, pinned: &[u8], mut packet: Value, size: i64, mut source: impl AsyncRead + Unpin) -> Result<()> {
    let listener = bind().await?;
    packet["payloadSize"] = json!(size);
    packet["payloadTransferInfo"] = json!({ "port": listener.local_addr()?.port() });
    sender.send(&packet).await?;

    let (tcp, _) = tokio::time::timeout(Duration::from_secs(15), listener.accept()).await.context("phone never fetched the payload")??;
    tcp.set_nodelay(true)?;
    if size < 0 {
        live(&tcp);
    }
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

/// Opens the payload the phone announced in `packet`; reads end after `payloadSize` bytes.
pub async fn fetch(me: &Identity, pinned: &[u8], phone: IpAddr, packet: &Value) -> Result<impl AsyncRead + Unpin + use<>> {
    let port = packet["payloadTransferInfo"]["port"].as_u64().context("payload without a port")? as u16;
    let size = packet["payloadSize"].as_u64().context("payload without a size")?;
    // With the cable in, the phone also names its port's adb forward here: far quicker than Wi-Fi.
    // Another plugged-in phone could sit behind it; the certificate check below sends that one away.
    if let Some(usb) = packet["payloadTransferInfo"]["usbPort"].as_u64() {
        match connect(me, pinned, IpAddr::from([127, 0, 0, 1]), usb as u16).await {
            Ok(tls) => return Ok(Either::Usb(tls).take(size)),
            Err(e) => tracing::debug!("payload over USB failed, using Wi-Fi: {e:#}"),
        }
    }
    Ok(Either::Lan(connect(me, pinned, phone, port).await?).take(size))
}

type Tls = tokio_rustls::client::TlsStream<TcpStream>;

/// The same stream type whichever way it came (`impl` can't name two).
enum Either {
    Usb(Tls),
    Lan(Tls),
}

impl AsyncRead for Either {
    fn poll_read(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>, buf: &mut tokio::io::ReadBuf<'_>) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            Either::Usb(s) | Either::Lan(s) => std::pin::Pin::new(s).poll_read(cx, buf),
        }
    }
}

async fn connect(me: &Identity, pinned: &[u8], phone: IpAddr, port: u16) -> Result<Tls> {
    let tcp = tokio::time::timeout(Duration::from_secs(5), TcpStream::connect((phone, port))).await.context("couldn't reach the phone")??;
    tcp.set_nodelay(true)?;
    let connector = TlsConnector::from(Arc::new(crate::link::client_config(me)?));
    let name = rustls_pki_types::ServerName::IpAddress(phone.into());
    let tls = tokio::time::timeout(Duration::from_secs(10), connector.connect(name, tcp)).await.context("payload TLS timed out")??;
    let peer = tls.get_ref().1.peer_certificates().and_then(|c| c.first()).context("phone sent no certificate")?;
    if peer.as_ref() != pinned {
        bail!("payload peer isn't the paired phone");
    }
    Ok(tls)
}

/// A live stream must not queue: the kernel would otherwise take megabytes of unsent video when
/// the link slows (Wi-Fi hiccups), seconds behind. With little unsent allowed, a slow link stalls
/// the writer instead, back up to the encoder, which then drops frames rather than delaying them.
fn live(tcp: &TcpStream) {
    use std::os::fd::AsRawFd;
    let lowat: libc::c_int = 16 * 1024;
    // SAFETY: a plain setsockopt on a socket we own, with an int-sized value.
    unsafe {
        libc::setsockopt(tcp.as_raw_fd(), libc::IPPROTO_TCP, libc::TCP_NOTSENT_LOWAT, (&lowat as *const libc::c_int).cast(), size_of::<libc::c_int>() as libc::socklen_t);
    }
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
