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
/// How long the phone has to fetch a payload, and to prove who it is once connected.
const FETCH_WAIT: Duration = Duration::from_secs(15);
const PROOF_LIMIT: Duration = Duration::from_secs(5);
const MAX_PROOFS: usize = 4;

pub async fn offer(sender: &Sender, me: &Identity, pinned: &[u8], packet: Value, size: u64, source: impl AsyncRead + Unpin) -> Result<()> {
    offer_sized(sender, me, pinned, packet, size as i64, source).await
}

/// An open-ended payload (`payloadSize` -1): streams until [source] ends or the phone hangs up.
pub async fn offer_stream(sender: &Sender, me: &Identity, pinned: &[u8], packet: Value, source: impl AsyncRead + Unpin) -> Result<()> {
    offer_sized(sender, me, pinned, packet, -1, source).await
}

async fn offer_sized(sender: &Sender, me: &Identity, pinned: &[u8], packet: Value, size: i64, source: impl AsyncRead + Unpin) -> Result<()> {
    let mut offer = Offer::open().await?;
    sender.send(&offer.announce(packet, size)?).await?;
    offer.serve(me, pinned, source).await
}

/// A payload being offered, for a caller that sends the announcement its own way (the device hub):
/// [Offer::announce] fills in the packet, then [Offer::serve] waits for the device and streams.
pub struct Offer {
    listener: TcpListener,
    size: i64,
}

impl Offer {
    pub async fn open() -> Result<Self> {
        Ok(Self { listener: bind().await?, size: 0 })
    }

    /// [packet] with the payload's size (-1: open-ended) and where to fetch it.
    pub fn announce(&mut self, mut packet: Value, size: i64) -> Result<Value> {
        self.size = size;
        packet["payloadSize"] = json!(size);
        packet["payloadTransferInfo"] = json!({ "port": self.listener.local_addr()?.port() });
        Ok(packet)
    }

    pub async fn serve(self, me: &Identity, pinned: &[u8], mut source: impl AsyncRead + Unpin) -> Result<()> {
        let mut tls = accept_phone(&self.listener, me, pinned).await?;
        if self.size < 0 {
            live(tls.get_ref().0);
        }
        tokio::io::copy(&mut source, &mut tls).await?;
        tls.shutdown().await?;
        Ok(())
    }
}

/// The first connection to prove it's the paired phone. Anyone on the network can reach this port,
/// so one that's wrong or stalls is dropped and the wait goes on. Proofs run side by side, each with
/// a time limit, so a few idle connections can't hold up the real one.
async fn accept_phone(listener: &TcpListener, me: &Identity, pinned: &[u8]) -> Result<tokio_rustls::server::TlsStream<TcpStream>> {
    let acceptor = TlsAcceptor::from(Arc::new(crate::link::server_config(me)?));
    let mut proofs = tokio::task::JoinSet::new();
    let wait = async {
        loop {
            tokio::select! {
                accepted = listener.accept(), if proofs.len() < MAX_PROOFS => {
                    let (acceptor, pinned, (tcp, _)) = (acceptor.clone(), pinned.to_vec(), accepted?);
                    proofs.spawn(tokio::time::timeout(PROOF_LIMIT, async move {
                        tcp.set_nodelay(true)?;
                        let tls = acceptor.accept(tcp).await?;
                        let peer = tls.get_ref().1.peer_certificates().and_then(|c| c.first()).context("phone sent no certificate")?;
                        if peer.as_ref() != pinned {
                            bail!("not the paired phone");
                        }
                        Ok(tls)
                    }));
                }
                Some(done) = proofs.join_next() => match done {
                    Ok(Ok(Ok(tls))) => return Ok(tls),
                    Ok(Ok(Err(e))) => tracing::info!("payload port: dropped a connection: {e:#}"),
                    _ => tracing::info!("payload port: dropped a connection that stalled"),
                },
            }
        }
    };
    tokio::time::timeout(FETCH_WAIT, wait).await.context("phone never fetched the payload")?
}

/// Opens the payload the phone announced in `packet`; reads end after `payloadSize` bytes.
pub async fn fetch(me: &Identity, pinned: &[u8], phone: IpAddr, packet: &Value) -> Result<impl AsyncRead + Unpin + use<>> {
    let size = packet["payloadSize"].as_u64().context("payload without a size")?;
    Ok(open(me, pinned, phone, packet).await?.take(size))
}

/// Opens a live payload the phone announced (`payloadSize` -1, the phone's camera): it runs until
/// the phone hangs up.
pub async fn fetch_stream(me: &Identity, pinned: &[u8], phone: IpAddr, packet: &Value) -> Result<impl AsyncRead + Unpin + use<>> {
    open(me, pinned, phone, packet).await
}

async fn open(me: &Identity, pinned: &[u8], phone: IpAddr, packet: &Value) -> Result<Either> {
    let port = packet["payloadTransferInfo"]["port"].as_u64().context("payload without a port")? as u16;
    // With the cable in, the phone also names its port's adb forward here: far quicker than Wi-Fi.
    // Another plugged-in phone could sit behind it; the certificate check below sends that one away.
    if let Some(usb) = packet["payloadTransferInfo"]["usbPort"].as_u64() {
        match connect(me, pinned, IpAddr::from([127, 0, 0, 1]), usb as u16).await {
            Ok(tls) => {
                tracing::info!("payload on port {port} over USB");
                return Ok(Either::Usb(tls));
            }
            Err(e) => tracing::info!("payload on port {port} over USB failed, using Wi-Fi: {e:#}"),
        }
    }
    let lan = connect(me, pinned, phone, port).await?;
    // A link from loopback is the adb tunnel: the cable, not the network.
    tracing::info!("payload on port {port} over {}", if phone.is_loopback() { "USB" } else { "Wi-Fi" });
    Ok(Either::Lan(lan))
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

#[cfg(test)]
mod tests {
    use super::*;
    use rustls_pki_types::{PrivateKeyDer, ServerName};

    fn identity() -> Identity {
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap().self_signed(&key).unwrap();
        Identity { device_id: "test".into(), cert: cert.der().clone(), key: PrivateKeyDer::try_from(key.serialize_der()).unwrap() }
    }

    async fn dial(who: &Identity, port: u16) -> Result<()> {
        let tcp = TcpStream::connect(("127.0.0.1", port)).await?;
        let connector = TlsConnector::from(Arc::new(crate::link::client_config(who)?));
        let mut tls = connector.connect(ServerName::IpAddress(IpAddr::from([127, 0, 0, 1]).into()), tcp).await?;
        // With TLS 1.3 the client finishes first; one byte back proves it was let in.
        let mut byte = [0];
        tls.read_exact(&mut byte).await?;
        Ok(())
    }

    #[tokio::test]
    async fn strangers_at_the_port_dont_end_the_transfer() {
        let (me, phone, stranger) = (identity(), identity(), identity());
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let pinned = phone.cert.to_vec();

        // One gives up at once, one shows the wrong certificate, the rest sit there saying nothing.
        drop(TcpStream::connect(("127.0.0.1", port)).await.unwrap());
        let wrong = tokio::spawn(async move { dial(&stranger, port).await });
        let mut idle = Vec::new();
        for _ in 0..MAX_PROOFS {
            idle.push(TcpStream::connect(("127.0.0.1", port)).await.unwrap());
        }
        let real = tokio::spawn(async move { dial(&phone, port).await });

        let mut tls = accept_phone(&listener, &me, &pinned).await.expect("the phone gets through");
        tls.write_all(&[1]).await.unwrap();
        tls.flush().await.unwrap();
        real.await.unwrap().unwrap();
        assert!(wrong.await.unwrap().is_err());
    }

    #[tokio::test]
    async fn nobody_but_the_phone_is_served() {
        let (me, stranger) = (identity(), identity());
        let pinned = identity().cert.to_vec();
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let dialled = tokio::spawn(async move { dial(&stranger, port).await });
        let waited = tokio::time::timeout(Duration::from_millis(800), accept_phone(&listener, &me, &pinned)).await;
        assert!(waited.is_err(), "still waiting for the phone");
        assert!(dialled.await.unwrap().is_err());
    }
}
