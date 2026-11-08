//! The key a server signs its pushes with (VAPID, RFC 8292): a P-256 key pair made from the
//! operating system's randomness, whose private half its caller keeps sealed. The public half
//! is what a browser subscribes with (`applicationServerKey`), so its push service takes pushes
//! for that subscription only when they are signed with the private half.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use glave_event::Secret;
use p256::ecdsa::signature::Signer as _;
use p256::ecdsa::{Signature, SigningKey};
use thiserror::Error;
use zeroize::Zeroizing;

/// How many times a key is drawn before giving up: 32 random bytes fail only as a number past
/// the curve's order, about once in 2^32 draws.
const DRAWS: usize = 4;

/// What a JWT's header says of every token: signed with ES256.
const HEADER: &[u8] = br#"{"typ":"JWT","alg":"ES256"}"#;

/// Why no key could be made or read.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum KeyError {
    #[error("the operating system gave no randomness to make a key with")]
    NoRandomness,
    #[error("the kept key is not a P-256 private key in base64url")]
    Unreadable,
}

/// A server's VAPID key pair.
pub struct VapidKey(SigningKey);

impl VapidKey {
    /// A new key, from the operating system's randomness.
    pub fn make() -> Result<Self, KeyError> {
        for _ in 0..DRAWS {
            let mut bytes = Zeroizing::new([0u8; 32]);
            getrandom::fill(bytes.as_mut()).map_err(|_| KeyError::NoRandomness)?;
            if let Ok(key) = SigningKey::from_slice(bytes.as_ref()) {
                return Ok(Self(key));
            }
        }
        Err(KeyError::NoRandomness)
    }

    /// The key [`Self::secret`] wrote.
    pub fn read(secret: &Secret) -> Result<Self, KeyError> {
        let bytes = Zeroizing::new(
            URL_SAFE_NO_PAD
                .decode(secret.expose())
                .map_err(|_| KeyError::Unreadable)?,
        );
        SigningKey::from_slice(&bytes)
            .map(Self)
            .map_err(|_| KeyError::Unreadable)
    }

    /// Its private half, as base64url, to keep sealed.
    pub fn secret(&self) -> Secret {
        Secret::new(URL_SAFE_NO_PAD.encode(self.0.to_bytes()))
    }

    /// Its public half as a browser subscribes with it: the uncompressed point, in base64url.
    pub fn public(&self) -> String {
        let point = self.0.verifying_key().to_encoded_point(false);
        URL_SAFE_NO_PAD.encode(point.as_bytes())
    }

    /// The `Authorization` a push to `audience` (the push service's origin) carries until
    /// `expires` (seconds since 1970): a token signed with this key that names `contact`, and
    /// the key it is checked with.
    pub(crate) fn authorization(&self, audience: &str, expires: i64, contact: &str) -> String {
        let claims = serde_json::json!({ "aud": audience, "exp": expires, "sub": contact });
        let signed = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(HEADER),
            URL_SAFE_NO_PAD.encode(claims.to_string())
        );
        let signature: Signature = self.0.sign(signed.as_bytes());
        let token = format!("{signed}.{}", URL_SAFE_NO_PAD.encode(signature.to_bytes()));
        format!("vapid t={token}, k={}", self.public())
    }
}

#[cfg(test)]
mod tests {
    use p256::ecdsa::VerifyingKey;
    use p256::ecdsa::signature::Verifier as _;

    use super::*;

    #[test]
    fn a_kept_key_reads_back_as_the_same_key() {
        let key = VapidKey::make().unwrap();
        let again = VapidKey::read(&key.secret()).unwrap();
        assert_eq!(again.public(), key.public());
        assert_ne!(VapidKey::make().unwrap().public(), key.public());
        // A browser's applicationServerKey: 65 bytes, uncompressed.
        assert_eq!(URL_SAFE_NO_PAD.decode(key.public()).unwrap().len(), 65);
        assert_eq!(
            VapidKey::read(&Secret::new("not a key")).err(),
            Some(KeyError::Unreadable)
        );
    }

    #[test]
    fn its_token_is_signed_for_the_push_service_and_names_who_to_contact() {
        let key = VapidKey::make().unwrap();
        let header = key.authorization("https://push.example.net", 1_700_000_000, "mailto:a@b.c");
        let (token, public) = header
            .strip_prefix("vapid t=")
            .and_then(|rest| rest.split_once(", k="))
            .unwrap();
        assert_eq!(public, key.public());
        let parts: Vec<&str> = token.split('.').collect();
        let [head, claims, signature] = parts.as_slice() else {
            panic!("a JWT has three parts: {token}");
        };
        let head: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(head).unwrap()).unwrap();
        assert_eq!(head["alg"], "ES256");
        let claims: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(claims).unwrap()).unwrap();
        assert_eq!(claims["aud"], "https://push.example.net");
        assert_eq!(claims["exp"], 1_700_000_000);
        assert_eq!(claims["sub"], "mailto:a@b.c");
        let verifying =
            VerifyingKey::from_sec1_bytes(&URL_SAFE_NO_PAD.decode(public).unwrap()).unwrap();
        let signature = Signature::from_slice(&URL_SAFE_NO_PAD.decode(signature).unwrap()).unwrap();
        let signed = format!("{}.{}", parts[0], parts[1]);
        verifying.verify(signed.as_bytes(), &signature).unwrap();
    }
}
