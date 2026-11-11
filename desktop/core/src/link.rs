//! One KDE Connect link to a device.
//!
//! Whoever dials sends its identity in plaintext, then is the TLS server; whoever accepts reads
//! that line and is the TLS client. TLS accepts any certificate: trust is an exact match against
//! one pinned at pairing, and a device not paired yet must at least show a certificate whose
//! common name is the id it claims. Protocol v8: identities are exchanged again inside TLS, and
//! only that copy counts.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{DigitallySignedStruct, DistinguishedName, SignatureScheme};
use rustls_pki_types::{CertificateDer, ServerName, UnixTime};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_rustls::{TlsAcceptor, TlsConnector, TlsStream};

use crate::cert;
use crate::identity::Identity;

/// The most one packet may be from a device not paired yet, and from one that is.
pub const UNPAIRED_PACKET_BYTES: usize = 64 * 1024;
pub const MAX_PACKET_BYTES: usize = 32 * 1024 * 1024;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

pub fn packet(kind: &str, body: Value) -> Value {
    json!({ "id": now_ms(), "type": kind, "body": body })
}

type Stream = TlsStream<TcpStream>;

/// The sending half, shared by every task that talks to the phone.
#[derive(Clone)]
pub struct Sender(Out);

#[derive(Clone)]
enum Out {
    Stream(Arc<Mutex<WriteHalf<Stream>>>),
    /// Into a queue whose owner sends it on (the desktop daemon's device hub, which picks the link).
    Queue(tokio::sync::mpsc::UnboundedSender<Value>),
}

impl Sender {
    /// A sender whose packets go into [queue] instead of onto a link.
    pub fn queue(queue: tokio::sync::mpsc::UnboundedSender<Value>) -> Self {
        Self(Out::Queue(queue))
    }

    pub async fn send(&self, packet: &Value) -> Result<()> {
        match &self.0 {
            Out::Stream(w) => {
                let mut line = serde_json::to_vec(packet)?;
                line.push(b'\n');
                let mut w = w.lock().await;
                w.write_all(&line).await?;
                w.flush().await?;
            }
            Out::Queue(q) => q.send(packet.clone()).map_err(|_| anyhow::anyhow!("the link is closed"))?,
        }
        Ok(())
    }
}

pub struct Receiver {
    reader: BufReader<ReadHalf<Stream>>,
    /// The longest line taken; a longer one ends the link.
    pub limit: usize,
}

impl Receiver {
    fn new(read: ReadHalf<Stream>, limit: usize) -> Self {
        Self { reader: BufReader::new(read), limit }
    }

    /// Next packet, or None when the phone hangs up.
    pub async fn next(&mut self) -> Result<Option<Value>> {
        loop {
            let mut line = Vec::new();
            let n = (&mut self.reader).take(self.limit as u64 + 1).read_until(b'\n', &mut line).await?;
            if n == 0 {
                return Ok(None);
            }
            if line.last() != Some(&b'\n') && line.len() > self.limit {
                bail!("packet over {} bytes", self.limit);
            }
            if line.trim_ascii().is_empty() {
                continue;
            }
            return Ok(Some(serde_json::from_slice(&line).context("bad packet")?));
        }
    }
}

pub struct Link {
    pub sender: Sender,
    pub receiver: Receiver,
    pub phone_identity: Value,
    /// The certificate the device showed (DER).
    pub peer_cert: Vec<u8>,
    /// Where it is: payloads it offers are fetched from here.
    pub remote: SocketAddr,
}

/// Notices a phone that went away without a word (app reinstalled or killed, Wi-Fi dropped, its
/// FIN lost on the way): probes after 15 s idle, every 5 s, 3 misses; unacknowledged data gives up
/// after 30 s. Without it a half-open link looked alive for good, so the companion never redialled
/// and the dashboard lost its gauges until a restart.
fn keepalive(tcp: &TcpStream) {
    use std::os::fd::AsRawFd;
    let fd = tcp.as_raw_fd();
    let set = |level: libc::c_int, name: libc::c_int, value: libc::c_int| {
        // SAFETY: a plain setsockopt on a socket we own, with an int-sized value.
        unsafe { libc::setsockopt(fd, level, name, (&value as *const libc::c_int).cast(), size_of::<libc::c_int>() as libc::socklen_t) };
    };
    set(libc::SOL_SOCKET, libc::SO_KEEPALIVE, 1);
    // The idle time before the first probe: TCP_KEEPIDLE on Linux, TCP_KEEPALIVE on macOS.
    #[cfg(target_os = "macos")]
    set(libc::IPPROTO_TCP, libc::TCP_KEEPALIVE, 15);
    #[cfg(not(target_os = "macos"))]
    set(libc::IPPROTO_TCP, libc::TCP_KEEPIDLE, 15);
    set(libc::IPPROTO_TCP, libc::TCP_KEEPINTVL, 5);
    set(libc::IPPROTO_TCP, libc::TCP_KEEPCNT, 3);
    // Unsent data gives up after 30 s (Linux only; macOS has no such option, the probes above stand).
    #[cfg(target_os = "linux")]
    set(libc::IPPROTO_TCP, libc::TCP_USER_TIMEOUT, 30_000);
}

pub async fn connect(addr: SocketAddr, me: &Identity, identity: &Value, phone_id: &str, pinned_cert: &[u8]) -> Result<Link> {
    let trusted = HashMap::from([(phone_id.to_string(), pinned_cert.to_vec())]);
    Ok(connect_trusted(addr, me, identity, &trusted).await?.1)
}

/// The copy of our identity that goes out in the clear, before anyone's proved who they are (for a
/// tethered address, to whatever answers there). Peers take the real one from inside TLS, so this
/// says only what starts the handshake, not this PC's name or what it can do.
fn public_hello(identity: &Value) -> Value {
    let mut hello = identity.clone();
    let body = &mut hello["body"];
    body["deviceName"] = "Vyoma companion".into();
    body["incomingCapabilities"] = serde_json::json!([]);
    body["outgoingCapabilities"] = serde_json::json!([]);
    if let Some(body) = body.as_object_mut() {
        body.remove("vyomaHost");
    }
    // Which PC stays private, but the phone must know a call through an adb forward isn't another
    // KDE Connect app on it (both come from loopback). The name above says as much already.
    body["vyomaCompanion"] = true.into();
    hello
}

/// What answered isn't the phone that was asked for. Known apart from other failures because trying
/// again won't change it: the address now belongs to something else (that phone was reinstalled, say).
#[derive(Debug)]
pub struct NotTrusted;

impl std::fmt::Display for NotTrusted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("certificate isn't one KDE Connect trusts")
    }
}

impl std::error::Error for NotTrusted {}

/// Like [connect], to whichever of the [trusted] phones (id to certificate) answers at [addr];
/// for an address found without knowing who's there (a USB tethering link).
pub async fn connect_trusted(addr: SocketAddr, me: &Identity, identity: &Value, trusted: &HashMap<String, Vec<u8>>) -> Result<(String, Link)> {
    let tls = dial_tls(addr, me, &public_hello(identity)).await?;
    let peer = peer_cert(&tls)?;
    let Some(phone_id) = trusted.iter().find(|(_, cert)| peer == **cert).map(|(id, _)| id.clone()) else {
        return Err(NotTrusted.into());
    };
    let link = finish(tls, identity, &phone_id, MAX_PACKET_BYTES).await?;
    Ok((phone_id, link))
}

/// Dials [addr], says [hello] in the clear and becomes the TLS server.
async fn dial_tls(addr: SocketAddr, me: &Identity, hello: &Value) -> Result<Stream> {
    let mut tcp = tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(addr)).await.context("connect timed out")??;
    tcp.set_nodelay(true)?;
    keepalive(&tcp);
    let mut line = serde_json::to_vec(hello)?;
    line.push(b'\n');
    tcp.write_all(&line).await?;
    let acceptor = TlsAcceptor::from(Arc::new(server_config(me)?));
    Ok(tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(tcp)).await.context("TLS timed out")??.into())
}

fn peer_cert(tls: &Stream) -> Result<Vec<u8>> {
    Ok(tls.get_ref().1.peer_certificates().and_then(|c| c.first()).context("peer sent no certificate")?.to_vec())
}

/// The v8 exchange inside TLS: our identity out, theirs in, and it must be [peer_id]'s.
async fn finish(tls: Stream, identity: &Value, peer_id: &str, limit: usize) -> Result<Link> {
    let peer_cert = peer_cert(&tls)?;
    let remote = tls.get_ref().0.peer_addr()?;
    let (read, write) = tokio::io::split(tls);
    let sender = Sender(Out::Stream(Arc::new(Mutex::new(write))));
    let mut receiver = Receiver::new(read, UNPAIRED_PACKET_BYTES);
    sender.send(identity).await?;
    let phone_identity = tokio::time::timeout(HANDSHAKE_TIMEOUT, receiver.next())
        .await
        .context("no identity from peer")??
        .context("peer closed during handshake")?;
    if phone_identity["type"] != "kdeconnect.identity" || phone_identity["body"]["deviceId"] != peer_id {
        bail!("unexpected identity from peer");
    }
    receiver.limit = limit;
    Ok(Link { sender, receiver, phone_identity, peer_cert, remote })
}

/// A device not paired yet must show a certificate made out to the id it claims.
fn check_name(tls: &Stream, id: &str) -> Result<()> {
    if cert::common_name(&peer_cert(tls)?).as_deref() != Some(id) {
        bail!("certificate isn't {id}'s");
    }
    Ok(())
}

/// Dials a device that announced itself as [peer_id] (discovery), with our full identity in the
/// clear as KDE Connect does (the broadcast says as much). Its certificate must name it, and the
/// link reads at most [UNPAIRED_PACKET_BYTES] a packet until the caller raises `receiver.limit`.
pub async fn dial(addr: SocketAddr, me: &Identity, identity: &Value, peer_id: &str) -> Result<Link> {
    let tls = dial_tls(addr, me, identity).await?;
    check_name(&tls, peer_id)?;
    finish(tls, identity, peer_id, UNPAIRED_PACKET_BYTES).await
}

/// The identity line a dialler sends before TLS, read a byte at a time so nothing of the TLS
/// handshake behind it is taken, and at most [UNPAIRED_PACKET_BYTES].
pub async fn read_hello(tcp: &mut TcpStream) -> Result<Value> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        if tcp.read(&mut byte).await? == 0 {
            bail!("closed before its identity");
        }
        if byte[0] == b'\n' {
            break;
        }
        line.push(byte[0]);
        if line.len() > UNPAIRED_PACKET_BYTES {
            bail!("identity too long");
        }
    }
    let hello: Value = serde_json::from_slice(&line).context("bad identity")?;
    if hello["type"] != "kdeconnect.identity" {
        bail!("expected an identity");
    }
    Ok(hello)
}

/// Takes a link the device dialled, once [read_hello] gave [peer_id]: we're the TLS client.
pub async fn accept(tcp: TcpStream, me: &Identity, identity: &Value, peer_id: &str) -> Result<Link> {
    tcp.set_nodelay(true)?;
    keepalive(&tcp);
    let connector = TlsConnector::from(Arc::new(client_config(me)?));
    // No name to check (trust is the pin), and an id isn't always a valid DNS name: the address
    // stands in, which also keeps SNI out of the hello.
    let name = ServerName::IpAddress(tcp.peer_addr()?.ip().into());
    let tls: Stream = tokio::time::timeout(HANDSHAKE_TIMEOUT, connector.connect(name, tcp)).await.context("TLS timed out")??.into();
    check_name(&tls, peer_id)?;
    finish(tls, identity, peer_id, UNPAIRED_PACKET_BYTES).await
}

pub fn server_config(me: &Identity) -> Result<rustls::ServerConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    Ok(rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()?
        .with_client_cert_verifier(Arc::new(AnyClientCert(provider)))
        .with_single_cert(vec![me.cert.clone()], me.key.clone_key())?)
}

/// Requires a client certificate but accepts any well-signed one: pinning happens after the handshake.
#[derive(Debug)]
struct AnyClientCert(Arc<CryptoProvider>);

impl ClientCertVerifier for AnyClientCert {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(&self, _: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: UnixTime) -> Result<ClientCertVerified, rustls::Error> {
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// For connecting to a payload the phone serves (it's the TLS server then). Like [server_config],
/// any well-signed certificate passes the handshake and the caller pins it afterwards.
pub fn client_config(me: &Identity) -> Result<rustls::ClientConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    Ok(rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AnyServerCert(provider)))
        .with_client_auth_cert(vec![me.cert.clone()], me.key.clone_key())?)
}

#[derive(Debug)]
struct AnyServerCert(Arc<CryptoProvider>);

impl ServerCertVerifier for AnyServerCert {
    fn verify_server_cert(&self, _: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: &ServerName<'_>, _: &[u8], _: UnixTime) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clear_copy_of_the_identity_says_little() {
        let full = packet(
            "kdeconnect.identity",
            serde_json::json!({
                "deviceId": "abc", "deviceName": "my-pc companion", "deviceType": "desktop", "protocolVersion": 8,
                "incomingCapabilities": ["vyoma.files.get"], "outgoingCapabilities": ["vyoma.stats"], "vyomaHost": "host-id",
            }),
        );
        let hello = public_hello(&full);
        assert_eq!(hello["body"]["vyomaCompanion"], true);
        assert_eq!(hello["body"]["deviceId"], "abc");
        assert_eq!(hello["body"]["protocolVersion"], 8);
        let text = hello.to_string();
        for private in ["my-pc", "vyoma.files.get", "vyoma.stats", "host-id"] {
            assert!(!text.contains(private), "{private}");
        }
    }
}
