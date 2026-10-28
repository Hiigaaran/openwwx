//! Encryption for connections to the common server (TLS).
//!
//! A server makes its own certificate the first time it runs, so it needs no
//! domain or certificate authority. Clients trust a server's certificate the
//! first time they connect and remember its fingerprint ([`Pin`]); if it
//! changes later, they refuse to connect, as someone may be in the middle.

use std::fs;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, ring, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{
    ClientConfig, ClientConnection, DigitallySignedStruct, ServerConfig, ServerConnection, SignatureScheme, StreamOwned,
};
use sha2::{Digest, Sha256};

/// The fingerprint of the server certificate a client trusts: `None` until
/// the first connection, which fills it in.
pub type Pin = Arc<Mutex<Option<String>>>;

/// A pin for a server not seen before, or one seen with `fingerprint`.
pub fn pin(fingerprint: Option<String>) -> Pin {
    Arc::new(Mutex::new(fingerprint))
}

/// A certificate's fingerprint: SHA-256, in hex.
pub fn fingerprint(cert: &[u8]) -> String {
    Sha256::digest(cert).iter().map(|b| format!("{b:02x}")).collect()
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(ring::default_provider())
}

/// The server's TLS setup, with the certificate kept in `dir` (made the
/// first time), and that certificate's fingerprint.
pub fn server_config(dir: &Path) -> io::Result<(Arc<ServerConfig>, String)> {
    let (cert_path, key_path) = (dir.join("cert.der"), dir.join("key.der"));
    if !cert_path.exists() || !key_path.exists() {
        fs::create_dir_all(dir)?;
        let made = rcgen::generate_simple_self_signed(vec!["openwwx-server".to_string()]).map_err(io::Error::other)?;
        fs::write(&key_path, made.signing_key.serialize_der())?;
        fs::write(&cert_path, made.cert.der().as_ref())?;
    }
    let cert = CertificateDer::from(fs::read(&cert_path)?);
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(fs::read(&key_path)?));
    let print = fingerprint(&cert);
    let config = ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(io::Error::other)?
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .map_err(io::Error::other)?;
    Ok((Arc::new(config), print))
}

/// Accepts a certificate the first time and then only the same one.
#[derive(Debug)]
struct Pinned {
    pin: Pin,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let seen = fingerprint(end_entity);
        let mut pin = self.pin.lock().map_err(|_| rustls::Error::General("pin lock".into()))?;
        match pin.as_ref() {
            Some(expected) if *expected != seen => Err(rustls::Error::General(CHANGED.into())),
            Some(_) => Ok(ServerCertVerified::assertion()),
            None => {
                *pin = Some(seen);
                Ok(ServerCertVerified::assertion())
            }
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

/// Shown when a server's certificate is not the one trusted before.
pub const CHANGED: &str = "the server's certificate changed since the last connection";

/// An encrypted stream.
pub type Tls<C> = StreamOwned<C, TcpStream>;

/// Opens an encrypted connection over `sock`, trusting the server as `pin`
/// says. Returns once the handshake is done.
pub fn client(sock: TcpStream, pin: &Pin) -> io::Result<Tls<ClientConnection>> {
    let provider = provider();
    let verifier = Arc::new(Pinned { pin: pin.clone(), provider: provider.clone() });
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(io::Error::other)?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    let name = ServerName::try_from("openwwx-server").map_err(io::Error::other)?;
    let conn = ClientConnection::new(Arc::new(config), name).map_err(io::Error::other)?;
    handshake(StreamOwned::new(conn, sock))
}

/// The server's side of [`client`].
pub fn server(sock: TcpStream, config: &Arc<ServerConfig>) -> io::Result<Tls<ServerConnection>> {
    let conn = ServerConnection::new(config.clone()).map_err(io::Error::other)?;
    handshake(StreamOwned::new(conn, sock))
}

fn handshake<C, D>(mut tls: StreamOwned<C, TcpStream>) -> io::Result<StreamOwned<C, TcpStream>>
where
    C: std::ops::DerefMut<Target = rustls::ConnectionCommon<D>>,
    D: rustls::SideData,
{
    tls.sock.set_read_timeout(Some(Duration::from_secs(10)))?;
    while tls.conn.is_handshaking() {
        tls.conn.complete_io(&mut tls.sock)?;
    }
    tls.sock.set_read_timeout(None)?;
    Ok(tls)
}

/// Lets [`crate::pump`] reach the socket under an encrypted stream.
pub trait Socket: Read + Write + Send + 'static {
    fn socket(&self) -> &TcpStream;
}

impl Socket for TcpStream {
    fn socket(&self) -> &TcpStream {
        self
    }
}

impl<C, D> Socket for StreamOwned<C, TcpStream>
where
    C: std::ops::DerefMut<Target = rustls::ConnectionCommon<D>> + Send + 'static,
    D: rustls::SideData,
{
    fn socket(&self) -> &TcpStream {
        &self.sock
    }
}
