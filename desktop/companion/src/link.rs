//! One KDE Connect link to a phone, from the dialling side.
//!
//! We connect over TCP and send our identity in plaintext, then act as the TLS server (the side
//! that dials is the server in KDE Connect). TLS accepts any certificate; trust is an exact match
//! against the certificate kdeconnectd pinned for that phone. Protocol v8: identities are
//! exchanged again inside TLS, and only that copy counts.

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

pub async fn connect(addr: SocketAddr, me: &Identity, identity: &Value, phone_id: &str, pinned_cert: &[u8]) -> Result<Link> {
    let mut tcp = tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(addr)).await.context("connect timed out")??;
    tcp.set_nodelay(true)?;
    let mut hello = serde_json::to_vec(identity)?;
    hello.push(b'\n');
    tcp.write_all(&hello).await?;

    let acceptor = TlsAcceptor::from(Arc::new(server_config(me)?));
    let tls = tokio::time::timeout(Duration::from_secs(10), acceptor.accept(tcp)).await.context("TLS timed out")??;
    let peer = tls.get_ref().1.peer_certificates().and_then(|c| c.first()).context("phone sent no certificate")?;
    if peer.as_ref() != pinned_cert {
        bail!("certificate doesn't match the one KDE Connect trusts for {phone_id}");
    }

    let (read, write) = tokio::io::split(tls);
    let sender = Sender(Arc::new(Mutex::new(write)));
    let mut receiver = Receiver(BufReader::new(read).lines());
    sender.send(identity).await?;
    let phone_identity = tokio::time::timeout(Duration::from_secs(10), receiver.next())
        .await
        .context("no identity from phone")??
        .context("phone closed during handshake")?;
    if phone_identity["type"] != "kdeconnect.identity" || phone_identity["body"]["deviceId"] != phone_id {
        bail!("unexpected identity from phone");
    }
    Ok(Link { sender, receiver, phone_identity })
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
