//! One issuer's signing keys: found through its OIDC discovery document, fetched from its
//! JWKS, and cached. A token naming a key not in the cache fetches them again, at most once
//! per `REFETCH`, so a stream of tokens with made-up key ids cannot flood the issuer.

use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use jiff::{SignedDuration, Timestamp};
use jsonwebtoken::jwk::{AlgorithmParameters, EllipticCurve, Jwk, KeyAlgorithm, PublicKeyUse};
use jsonwebtoken::{Algorithm, DecodingKey};
use serde::Deserialize;
use serde_json::Value;

use super::SignInError;

/// The least time between two fetches of one issuer's keys.
const REFETCH: SignedDuration = SignedDuration::from_secs(30);
/// Keys kept per issuer; an issuer publishes a few at a time.
const MOST_KEYS: usize = 32;
/// The largest discovery document or key set read, in bytes.
const MOST_BYTES: usize = 256 * 1024;
/// How long one fetch may take.
const TIMEOUT: Duration = Duration::from_secs(10);

/// The HTTP client every issuer's keys are fetched with: it follows no redirects.
pub(super) fn client() -> Result<reqwest::Client, SignInError> {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| SignInError::Client(e.to_string()))
}

/// Whether keys may be fetched from `url`: `https`, or `http` to this computer only.
pub(super) fn allowed(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    match parsed.scheme() {
        "https" => parsed.host_str().is_some(),
        "http" => matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "[::1]")),
        _ => false,
    }
}

/// An issuer and the keys it signs with.
pub(super) struct Keys {
    issuer: String,
    http: reqwest::Client,
    cache: Mutex<Cache>,
}

#[derive(Default)]
struct Cache {
    jwks_uri: Option<String>,
    keys: Vec<Key>,
    fetched: Option<Timestamp>,
}

#[derive(Clone)]
struct Key {
    id: Option<String>,
    alg: Algorithm,
    checks: DecodingKey,
}

#[derive(Deserialize)]
struct Discovery {
    issuer: String,
    jwks_uri: String,
}

#[derive(Deserialize)]
struct KeySet {
    keys: Vec<Value>,
}

impl Keys {
    pub(super) fn new(issuer: String, http: reqwest::Client) -> Self {
        Self {
            issuer,
            http,
            cache: Mutex::new(Cache::default()),
        }
    }

    pub(super) fn issuer(&self) -> &str {
        &self.issuer
    }

    /// The key that checks a token signed with `alg` by the key `id`, fetching the issuer's
    /// keys when it is not known yet.
    pub(super) async fn key(
        &self,
        id: Option<&str>,
        alg: Algorithm,
        now: Timestamp,
    ) -> Result<DecodingKey, SignInError> {
        if let Some(key) = self.find(id, alg) {
            return Ok(key);
        }
        let jwks_uri = {
            let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
            if cache
                .fetched
                .is_some_and(|at| now.duration_since(at) < REFETCH)
            {
                return Err(self.unknown());
            }
            cache.fetched = Some(now);
            cache.jwks_uri.clone()
        };
        let jwks_uri = match jwks_uri {
            Some(uri) => uri,
            None => self.discover().await?,
        };
        let keys = self.fetch(&jwks_uri).await?;
        {
            let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
            cache.jwks_uri = Some(jwks_uri);
            cache.keys = keys;
        }
        self.find(id, alg).ok_or_else(|| self.unknown())
    }

    fn unknown(&self) -> SignInError {
        SignInError::UnknownKey {
            issuer: self.issuer.clone(),
        }
    }

    /// The cached key for `id` and `alg`. A token without a key id may use the issuer's one
    /// key for that algorithm, when it has only one.
    fn find(&self, id: Option<&str>, alg: Algorithm) -> Option<DecodingKey> {
        let cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        let mut usable = cache.keys.iter().filter(|key| key.alg == alg);
        match id {
            Some(id) => usable
                .find(|key| key.id.as_deref() == Some(id))
                .map(|key| key.checks.clone()),
            None => match (usable.next(), usable.next()) {
                (Some(only), None) => Some(only.checks.clone()),
                _ => None,
            },
        }
    }

    /// Where the issuer publishes its keys, from its discovery document.
    async fn discover(&self) -> Result<String, SignInError> {
        let url = format!(
            "{}/.well-known/openid-configuration",
            self.issuer.trim_end_matches('/')
        );
        let discovery: Discovery = self.get(&url).await?;
        if discovery.issuer != self.issuer {
            return Err(self.failed(format!(
                "its discovery document names the issuer {:?}",
                discovery.issuer
            )));
        }
        if !allowed(&discovery.jwks_uri) {
            return Err(self.failed(format!(
                "its keys are at {:?}, which is not https",
                discovery.jwks_uri
            )));
        }
        Ok(discovery.jwks_uri)
    }

    /// The keys at `jwks_uri` that sign with RS256 or ES256. Others are skipped.
    async fn fetch(&self, jwks_uri: &str) -> Result<Vec<Key>, SignInError> {
        let set: KeySet = self.get(jwks_uri).await?;
        Ok(set
            .keys
            .into_iter()
            .filter_map(|value| serde_json::from_value::<Jwk>(value).ok())
            .filter_map(|jwk| usable(&jwk))
            .take(MOST_KEYS)
            .collect())
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T, SignInError> {
        let mut response = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| self.failed(e.to_string()))?;
        if !response.status().is_success() {
            return Err(self.failed(format!("{url} answered {}", response.status())));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| self.failed(e.to_string()))?
        {
            if body.len().saturating_add(chunk.len()) > MOST_BYTES {
                return Err(self.failed(format!("{url} sent more than {MOST_BYTES} bytes")));
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body).map_err(|e| self.failed(format!("{url}: {e}")))
    }

    fn failed(&self, reason: String) -> SignInError {
        SignInError::Keys {
            issuer: self.issuer.clone(),
            reason,
        }
    }
}

/// `jwk` as a key that checks RS256 or ES256 signatures, if it is one.
fn usable(jwk: &Jwk) -> Option<Key> {
    if jwk
        .common
        .public_key_use
        .as_ref()
        .is_some_and(|used| *used != PublicKeyUse::Signature)
    {
        return None;
    }
    let alg = match &jwk.algorithm {
        AlgorithmParameters::RSA(_) => Algorithm::RS256,
        AlgorithmParameters::EllipticCurve(ec) if ec.curve == EllipticCurve::P256 => {
            Algorithm::ES256
        }
        _ => return None,
    };
    let named = jwk.common.key_algorithm;
    let matches = match named {
        None => true,
        Some(KeyAlgorithm::RS256) => alg == Algorithm::RS256,
        Some(KeyAlgorithm::ES256) => alg == Algorithm::ES256,
        Some(_) => false,
    };
    if !matches {
        return None;
    }
    Some(Key {
        id: jwk.common.key_id.clone(),
        alg,
        checks: DecodingKey::from_jwk(jwk).ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_come_over_https_or_from_this_computer() {
        assert!(allowed("https://accounts.example.com"));
        assert!(allowed("http://127.0.0.1:8080"));
        assert!(allowed("http://localhost/realms/glave"));
        assert!(allowed("http://[::1]:9000"));
        assert!(!allowed("http://accounts.example.com"));
        assert!(!allowed("http://127.0.0.1.example.com"));
        assert!(!allowed("http://localhost@example.com"));
        assert!(!allowed("ftp://127.0.0.1"));
        assert!(!allowed("not a url"));
    }
}
