//! One KDE Connect link to a phone, from the dialling side.
//!
//! We connect over TCP and send our identity in plaintext, then act as the TLS server (the side
//! that dials is the server in KDE Connect). TLS accepts any certificate; trust is an exact match
//! against the certificate kdeconnectd pinned for that phone. Protocol v8: identities are
//! exchanged again inside TLS, and only that copy counts.

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
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::server::TlsStream;

use crate::identity::Identity;

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

pub fn packet(kind: &str, body: Value) -> Value {
    json!({ "id": now_ms(), "type": kind, "body": body })
}

type Stream = TlsStream<TcpStream>;

/// The sending half, shared by every task that talks to the phone.
#[derive(Clone)]
pub struct Sender(Arc<Mutex<WriteHalf<Stream>>>);

impl Sender {
    pub async fn send(&self, packet: &Value) -> Result<()> {
        let mut line = serde_json::to_vec(packet)?;
        line.push(b'\n');
        let mut w = self.0.lock().await;
        w.write_all(&line).await?;
        w.flush().await?;
        Ok(())
    }
}

pub struct Receiver(tokio::io::Lines<BufReader<ReadHalf<Stream>>>);

impl Receiver {
    /// Next packet, or None when the phone hangs up.
    pub async fn next(&mut self) -> Result<Option<Value>> {
        loop {
            match self.0.next_line().await? {
                None => return Ok(None),
                Some(line) if line.trim().is_empty() => continue,
                Some(line) => return Ok(Some(serde_json::from_str(&line).context("bad packet")?)),
            }
        }
    }
}

pub struct Link {
    pub sender: Sender,
    pub receiver: Receiver,
    pub phone_identity: Value,
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
    set(libc::IPPROTO_TCP, libc::TCP_KEEPIDLE, 15);
    set(libc::IPPROTO_TCP, libc::TCP_KEEPINTVL, 5);
    set(libc::IPPROTO_TCP, libc::TCP_KEEPCNT, 3);
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
    hello
}

/// Like [connect], to whichever of the [trusted] phones (id to certificate) answers at [addr];
/// for an address found without knowing who's there (a USB tethering link).
pub async fn connect_trusted(addr: SocketAddr, me: &Identity, identity: &Value, trusted: &HashMap<String, Vec<u8>>) -> Result<(String, Link)> {
    let mut tcp = tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(addr)).await.context("connect timed out")??;
    tcp.set_nodelay(true)?;
    keepalive(&tcp);
    let mut hello = serde_json::to_vec(&public_hello(identity))?;
    hello.push(b'\n');
    tcp.write_all(&hello).await?;

    let acceptor = TlsAcceptor::from(Arc::new(server_config(me)?));
    let tls = tokio::time::timeout(Duration::from_secs(10), acceptor.accept(tcp)).await.context("TLS timed out")??;
    let peer = tls.get_ref().1.peer_certificates().and_then(|c| c.first()).context("phone sent no certificate")?;
    let Some(phone_id) = trusted.iter().find(|(_, cert)| peer.as_ref() == cert.as_slice()).map(|(id, _)| id.clone()) else {
        bail!("certificate isn't one KDE Connect trusts");
    };

    let (read, write) = tokio::io::split(tls);
    let sender = Sender(Arc::new(Mutex::new(write)));
    let mut receiver = Receiver(BufReader::new(read).lines());
    sender.send(identity).await?;
    let phone_identity = tokio::time::timeout(Duration::from_secs(10), receiver.next())
        .await
        .context("no identity from phone")??
        .context("phone closed during handshake")?;
    if phone_identity["type"] != "kdeconnect.identity" || phone_identity["body"]["deviceId"] != phone_id.as_str() {
        bail!("unexpected identity from phone");
    }
    Ok((phone_id, Link { sender, receiver, phone_identity }))
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
        assert_eq!(hello["body"]["deviceId"], "abc");
        assert_eq!(hello["body"]["protocolVersion"], 8);
        let text = hello.to_string();
        for private in ["my-pc", "vyoma.files.get", "vyoma.stats", "host-id"] {
            assert!(!text.contains(private), "{private}");
        }
    }
}
