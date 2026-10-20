//! What a TLS handshake for the name is answered with: the certificate in hand, or, when the
//! authority asks with ALPN `acme-tls/1`, the challenge's certificate (RFC 8737). A handshake
//! for another name, or before a certificate is in hand, gets nothing and fails.

use std::fmt;
use std::sync::{Arc, PoisonError, RwLock};

use rustls::crypto::aws_lc_rs;
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls::{ServerConfig, version};

use crate::https::HttpsError;

/// The protocol the authority's challenge is asked under (RFC 8737, section 6.2).
const ACME_TLS: &[u8] = b"acme-tls/1";

/// The protocol HTTPS is served with: HTTP/1.1, as the session API's server speaks.
const HTTP_1_1: &[u8] = b"http/1.1";

/// The certificates the name is served with.
pub(crate) struct Resolver {
    name: String,
    current: RwLock<Option<Arc<CertifiedKey>>>,
    challenge: RwLock<Option<Arc<CertifiedKey>>>,
}

impl Resolver {
    pub(crate) fn new(name: String) -> Self {
        Self {
            name,
            current: RwLock::new(None),
            challenge: RwLock::new(None),
        }
    }

    /// Serves `key` from now on.
    pub(crate) fn serve(&self, key: CertifiedKey) {
        *self.current.write().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(key));
    }

    /// Answers the authority's challenge with `key` until [`Resolver::answered`].
    pub(crate) fn challenge(&self, key: CertifiedKey) {
        *self
            .challenge
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(key));
    }

    /// The challenge is over.
    pub(crate) fn answered(&self) {
        *self
            .challenge
            .write()
            .unwrap_or_else(PoisonError::into_inner) = None;
    }

    /// Whether a certificate is in hand.
    pub(crate) fn ready(&self) -> bool {
        self.current
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }
}

impl ResolvesServerCert for Resolver {
    fn resolve(&self, hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        let named = hello
            .server_name()
            .is_none_or(|name| name.eq_ignore_ascii_case(&self.name));
        if !named {
            return None;
        }
        let challenged = hello
            .alpn()
            .is_some_and(|mut asked| asked.any(|protocol| protocol == ACME_TLS));
        let kept = if challenged {
            &self.challenge
        } else {
            &self.current
        };
        kept.read().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

impl fmt::Debug for Resolver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Resolver")
            .field("name", &self.name)
            .field("ready", &self.ready())
            .finish_non_exhaustive()
    }
}

/// What HTTPS is served with: TLS 1.3 on aws-lc-rs, the crypto rustls already uses, each
/// handshake answered by `resolver`.
pub(crate) fn config(resolver: Arc<Resolver>) -> Result<Arc<ServerConfig>, HttpsError> {
    let provider = Arc::new(aws_lc_rs::default_provider());
    let mut config = ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&version::TLS13])
        .map_err(|error| HttpsError::Make(error.to_string()))?
        .with_no_client_auth()
        .with_cert_resolver(resolver);
    config.alpn_protocols = vec![HTTP_1_1.to_vec(), ACME_TLS.to_vec()];
    Ok(Arc::new(config))
}

/// The certificate whose chain and private key are `chain` and `key`, as PEM, ready to serve.
pub(crate) fn certified(chain: &str, key: &str) -> Result<CertifiedKey, HttpsError> {
    let unread = |what: &str, error: &dyn fmt::Display| {
        HttpsError::Make(format!("the {what} is not PEM: {error}"))
    };
    let chain = CertificateDer::pem_slice_iter(chain.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| unread("certificate", &error))?;
    if chain.is_empty() {
        return Err(HttpsError::Make(
            "the certificate holds no certificate".into(),
        ));
    }
    let key =
        PrivateKeyDer::from_pem_slice(key.as_bytes()).map_err(|error| unread("key", &error))?;
    signed(chain, &key)
}

/// The challenge's certificate for `name`: self-signed, carrying `digest`, the SHA-256 of the
/// authority's key authorization, in its `acmeIdentifier` extension (RFC 8737, section 3).
pub(crate) fn challenge_key(name: &str, digest: &[u8]) -> Result<CertifiedKey, HttpsError> {
    let failed = |error: rcgen::Error| HttpsError::Make(error.to_string());
    let key = rcgen::KeyPair::generate().map_err(failed)?;
    let mut params = rcgen::CertificateParams::new(vec![name.to_owned()]).map_err(failed)?;
    params.custom_extensions = vec![rcgen::CustomExtension::new_acme_identifier(digest)];
    let certificate = params.self_signed(&key).map_err(failed)?;
    let private = PrivateKeyDer::try_from(key.serialize_der())
        .map_err(|error| HttpsError::Make(error.to_owned()))?;
    signed(vec![certificate.der().clone()], &private)
}

/// `chain` signed for with `key`.
fn signed(
    chain: Vec<CertificateDer<'static>>,
    key: &PrivateKeyDer<'_>,
) -> Result<CertifiedKey, HttpsError> {
    let signing = aws_lc_rs::sign::any_supported_type(key)
        .map_err(|error| HttpsError::Make(format!("the key cannot sign: {error}")))?;
    Ok(CertifiedKey::new(chain, signing))
}

#[cfg(test)]
mod tests;
