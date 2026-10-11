//! TLS between a harness and a computer on another machine. Each worker gets a self-signed
//! certificate the harness side makes for it; the harness trusts exactly that certificate (its
//! SHA-256 fingerprint is pinned), so no certificate authority is involved, and nothing else
//! can stand in for the worker. TLS 1.3 only, on aws-lc-rs, the crypto rustls already uses.

use std::fmt;
use std::net::SocketAddr;
use std::sync::Arc;

use glave_ports::{ComputerError, digest};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms, aws_lc_rs};
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, ServerConfig, SignatureScheme};
use tokio::net::TcpStream;
use tokio_rustls::{TlsAcceptor, TlsConnector};

use crate::remote::RemoteComputers;

/// The name in every worker's certificate. Only the pinned fingerprint is checked.
const NAME: &str = "glave-worker";
/// How a failure to connect at all begins, so a caller waiting for a computer to start can
/// tell it from a refusal.
pub(crate) const NOT_LISTENING: &str = "could not connect to";
/// How a failed handshake begins.
pub(crate) const NO_HANDSHAKE: &str = "the TLS handshake with";

/// The SHA-256 of a certificate (its DER bytes), as 64 hex digits.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Fingerprint(String);

impl Fingerprint {
    fn of(certificate: &[u8]) -> Self {
        Self(digest(certificate).as_str().to_owned())
    }

    /// A fingerprint written as 64 hex digits, colons allowed between pairs.
    pub fn parse(text: &str) -> Result<Self, ComputerError> {
        let hex: String = text
            .chars()
            .filter(|c| *c != ':')
            .collect::<String>()
            .to_ascii_lowercase();
        if hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            Ok(Self(hex))
        } else {
            Err(ComputerError::Refused(format!(
                "{text:?} is not a SHA-256 fingerprint"
            )))
        }
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fingerprint({})", self.0)
    }
}

/// A computer's TLS identity: its certificate and private key, as PEM, and the fingerprint a
/// client pins.
#[derive(Clone)]
pub struct Identity {
    pub certificate: String,
    pub key: String,
    pub fingerprint: Fingerprint,
}

impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Identity")
            .field("fingerprint", &self.fingerprint)
            .field("key", &"<hidden>")
            .finish_non_exhaustive()
    }
}

fn failed(what: &str) -> impl Fn(rcgen::Error) -> ComputerError + '_ {
    move |error| ComputerError::Failed(format!("could not {what}: {error}"))
}

impl Identity {
    /// A new self-signed identity, with a fresh P-256 key.
    pub fn mint() -> Result<Self, ComputerError> {
        let key = rcgen::KeyPair::generate().map_err(failed("make a key"))?;
        let params = rcgen::CertificateParams::new(vec![NAME.to_owned()])
            .map_err(failed("describe a certificate"))?;
        let certificate = params
            .self_signed(&key)
            .map_err(failed("sign a certificate"))?;
        Ok(Self {
            fingerprint: Fingerprint::of(certificate.der()),
            certificate: certificate.pem(),
            key: key.serialize_pem(),
        })
    }

    /// What `glave computer serve --tls` accepts connections with.
    pub fn acceptor(certificate: &str, key: &str) -> Result<TlsAcceptor, ComputerError> {
        let refused = |what: &'static str| {
            move |error: rustls::pki_types::pem::Error| {
                ComputerError::Refused(format!("the {what} is not PEM: {error}"))
            }
        };
        let chain = vec![
            CertificateDer::from_pem_slice(certificate.as_bytes())
                .map_err(refused("certificate"))?,
        ];
        let key = PrivateKeyDer::from_pem_slice(key.as_bytes()).map_err(refused("key"))?;
        let config = ServerConfig::builder_with_provider(provider())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|error| tls_failed(&error))?
            .with_no_client_auth()
            .with_single_cert(chain, key)
            .map_err(|error| tls_failed(&error))?;
        Ok(TlsAcceptor::from(Arc::new(config)))
    }
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(aws_lc_rs::default_provider())
}

fn tls_failed(error: &rustls::Error) -> ComputerError {
    ComputerError::Failed(format!("TLS: {error}"))
}

/// Trusts one certificate: the one whose fingerprint was pinned. Handshake signatures are
/// still checked, so only the holder of its key can answer.
#[derive(Debug)]
struct Pinned {
    fingerprint: Fingerprint,
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if Fingerprint::of(end_entity) == self.fingerprint {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(
                "the computer's certificate is not the one pinned for it".into(),
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// Reaches the computer served with TLS at `address`, trusting only the certificate with
/// `fingerprint`, and presents `token`. Checks it speaks this protocol before returning.
pub async fn connect_tls(
    address: SocketAddr,
    fingerprint: &Fingerprint,
    token: &str,
) -> Result<RemoteComputers, ComputerError> {
    let provider = provider();
    let pinned = Pinned {
        fingerprint: fingerprint.clone(),
        algorithms: provider.signature_verification_algorithms,
    };
    let config = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|error| tls_failed(&error))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(pinned))
        .with_no_client_auth();
    let stream = TcpStream::connect(address).await.map_err(|error| {
        ComputerError::Unreachable(format!("{NOT_LISTENING} {address}: {error}"))
    })?;
    let name = ServerName::try_from(NAME)
        .map_err(|error| ComputerError::Failed(format!("{NAME} is not a server name: {error}")))?;
    let stream = TlsConnector::from(Arc::new(config))
        .connect(name, stream)
        .await
        .map_err(|error| {
            ComputerError::Unreachable(format!("{NO_HANDSHAKE} {address} failed: {error}"))
        })?;
    let (read, write) = tokio::io::split(stream);
    let remote = RemoteComputers::connect(read, write);
    remote.authenticate(token).await?;
    remote.check_protocol().await?;
    Ok(remote)
}
