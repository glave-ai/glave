//! A fake OIDC identity provider on 127.0.0.1 for the HTTP tests: it serves a discovery
//! document and a JWKS, and signs tokens with keys made for the test run (ES256 and RS256).
//! Tokens are put together by hand, so tests can also make tokens no real issuer would.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::rsa::KeySize;
use aws_lc_rs::signature::{
    ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair as _, RSA_PKCS1_SHA256, RsaKeyPair,
};
use glave_ports::testing::ManualClock;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::task::{JoinHandle, JoinSet};

/// The audience every test server expects.
pub(crate) const AUDIENCE: &str = "glave-test";

/// The time every sign-in check sees, in seconds since 1970.
pub(crate) const NOW: i64 = 1_791_590_400;

/// A clock stopped at `NOW`.
pub(crate) fn fixed_clock() -> ManualClock {
    ManualClock::at_second(NOW)
}

/// Text as unpadded base64url, as JWTs and JWKs carry it.
pub(crate) fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..=chunk.len() {
            out.push(char::from(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize]));
        }
    }
    out
}

/// A signing key and its id.
pub(crate) enum Signer {
    Ec { kid: String, key: EcdsaKeyPair },
    Rsa { kid: String, key: RsaKeyPair },
}

impl Signer {
    pub(crate) fn es256(kid: &str) -> Self {
        let key = EcdsaKeyPair::generate(&ECDSA_P256_SHA256_FIXED_SIGNING).unwrap();
        Self::Ec {
            kid: kid.into(),
            key,
        }
    }

    pub(crate) fn rs256(kid: &str) -> Self {
        let key = RsaKeyPair::generate(KeySize::Rsa2048).unwrap();
        Self::Rsa {
            kid: kid.into(),
            key,
        }
    }

    /// The public key, as a JWKS lists it.
    pub(crate) fn jwk(&self) -> Value {
        match self {
            Self::Ec { kid, key } => {
                // An uncompressed point: 0x04, then x and y.
                let point = key.public_key().as_ref();
                json!({
                    "kty": "EC", "crv": "P-256", "use": "sig", "alg": "ES256", "kid": kid,
                    "x": base64url(&point[1..33]), "y": base64url(&point[33..65]),
                })
            }
            Self::Rsa { kid, key } => {
                let public = key.public_key();
                json!({
                    "kty": "RSA", "use": "sig", "alg": "RS256", "kid": kid,
                    "n": base64url(public.modulus().big_endian_without_leading_zero()),
                    "e": base64url(public.exponent().big_endian_without_leading_zero()),
                })
            }
        }
    }

    /// A token with `claims`, signed by this key and naming it.
    pub(crate) fn sign(&self, claims: &Value) -> String {
        let (alg, kid) = match self {
            Self::Ec { kid, .. } => ("ES256", kid),
            Self::Rsa { kid, .. } => ("RS256", kid),
        };
        self.sign_as(&json!({"alg": alg, "typ": "JWT", "kid": kid}), claims)
    }

    /// A token with any header, signed by this key.
    pub(crate) fn sign_as(&self, header: &Value, claims: &Value) -> String {
        let signed = format!(
            "{}.{}",
            base64url(header.to_string().as_bytes()),
            base64url(claims.to_string().as_bytes())
        );
        let rng = SystemRandom::new();
        let signature = match self {
            Self::Ec { key, .. } => key.sign(&rng, signed.as_bytes()).unwrap().as_ref().to_vec(),
            Self::Rsa { key, .. } => {
                let mut signature = vec![0; key.public_modulus_len()];
                key.sign(&RSA_PKCS1_SHA256, &rng, signed.as_bytes(), &mut signature)
                    .unwrap();
                signature
            }
        };
        format!("{signed}.{}", base64url(&signature))
    }
}

/// The identity provider: its URL is its issuer.
pub(crate) struct Issuer {
    pub url: String,
    published: Arc<Mutex<Vec<Value>>>,
    fetches: Arc<AtomicUsize>,
    serving: JoinHandle<()>,
}

impl Drop for Issuer {
    fn drop(&mut self) {
        self.serving.abort();
    }
}

impl Issuer {
    pub(crate) async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let published = Arc::new(Mutex::new(Vec::new()));
        let fetches = Arc::new(AtomicUsize::new(0));
        let shared = (url.clone(), Arc::clone(&published), Arc::clone(&fetches));
        let serving = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let (url, published, fetches) = shared.clone();
                let service = service_fn(move |request: Request<hyper::body::Incoming>| {
                    let body = match request.uri().path() {
                        "/.well-known/openid-configuration" => {
                            json!({"issuer": url, "jwks_uri": format!("{url}/jwks")})
                        }
                        "/jwks" => {
                            fetches.fetch_add(1, Ordering::SeqCst);
                            json!({"keys": *published.lock().unwrap()})
                        }
                        _ => json!({}),
                    };
                    async move { Ok::<_, std::convert::Infallible>(Response::new(body.to_string())) }
                });
                connections
                    .spawn(http1::Builder::new().serve_connection(TokioIo::new(stream), service));
            }
        });
        Self {
            url,
            published,
            fetches,
            serving,
        }
    }

    /// Lists `signer`'s key in the JWKS, beside any listed already.
    pub(crate) fn publish(&self, signer: &Signer) {
        self.published.lock().unwrap().push(signer.jwk());
    }

    /// How often the JWKS has been fetched.
    pub(crate) fn fetches(&self) -> usize {
        self.fetches.load(Ordering::SeqCst)
    }

    /// The claims of a good token for `subject`, valid for ten minutes around `NOW`.
    pub(crate) fn claims(&self, subject: &str) -> Value {
        json!({
            "iss": self.url, "sub": subject, "aud": AUDIENCE,
            "iat": NOW, "exp": NOW + 600, "sid": "session-1",
        })
    }
}
