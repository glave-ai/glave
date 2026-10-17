//! The signed JWT a service account proves itself with (RS256, RFC 7515), from the PEM key in
//! its key file.

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{RSA_PKCS1_SHA256, RsaKeyPair};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use glave_ports::auth::AuthError;
use serde::Serialize;

/// The lines a PEM private key's base64 sits between.
const BEGIN: &str = "-----BEGIN PRIVATE KEY-----";
const END: &str = "-----END PRIVATE KEY-----";

/// A service account's private key, ready to sign. Never shown.
pub(crate) struct SigningKey(RsaKeyPair);

impl SigningKey {
    /// The key in a key file's `private_key`: PKCS #8 in PEM. The error never shows the key.
    pub(crate) fn from_pem(pem: &str) -> Result<Self, AuthError> {
        let unreadable = || {
            AuthError::refused(
                "the service account key file's private_key is not an RSA key in PKCS #8 PEM",
            )
        };
        let body: String = pem
            .lines()
            .map(str::trim)
            .skip_while(|line| *line != BEGIN)
            .skip(1)
            .take_while(|line| *line != END)
            .collect();
        let der = STANDARD.decode(body).map_err(|_| unreadable())?;
        RsaKeyPair::from_pkcs8(&der)
            .map(Self)
            .map_err(|_| unreadable())
    }

    /// `header.payload.signature`, each part base64url without padding, signed with RS256.
    pub(crate) fn sign(
        &self,
        header: &impl Serialize,
        claims: &impl Serialize,
    ) -> Result<String, AuthError> {
        let input = format!("{}.{}", segment(header)?, segment(claims)?);
        let mut signature = vec![0; self.0.public_modulus_len()];
        self.0
            .sign(
                &RSA_PKCS1_SHA256,
                &SystemRandom::new(),
                input.as_bytes(),
                &mut signature,
            )
            .map_err(|_| AuthError::refused("could not sign the service account assertion"))?;
        Ok(format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature)))
    }
}

/// One part of a JWT: its JSON, base64url without padding.
fn segment(value: &impl Serialize) -> Result<String, AuthError> {
    serde_json::to_vec(value)
        .map(|json| URL_SAFE_NO_PAD.encode(json))
        .map_err(|_| AuthError::refused("could not encode the service account assertion"))
}

/// The JWT header Google asks for: RS256, and the key's id when the file names it.
#[derive(Serialize)]
pub(crate) struct Header<'a> {
    pub alg: &'static str,
    pub typ: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kid: Option<&'a str>,
}

/// The claims of an assertion exchanged for an access token: who, for what, to whom, and when.
#[derive(Serialize)]
pub(crate) struct Claims<'a> {
    pub iss: &'a str,
    pub scope: &'a str,
    pub aud: &'a str,
    pub iat: i64,
    pub exp: i64,
}

#[cfg(test)]
mod tests {
    use serde_json::value::RawValue;

    use super::*;

    /// The RSA key of RFC 7515, Appendix A.2, as PKCS #8 PEM: a published test key, kept as a
    /// file because the RFC's signature, checked byte for byte below, is made with exactly it.
    /// The one private key `cargo xtask secrets` allows in the source.
    const RFC_7515_KEY: &str = include_str!("jwt/rfc7515-a2.pem");

    #[test]
    fn signing_matches_rfc_7515_s_rs256_example() {
        // RS256 (RSASSA-PKCS1-v1_5) is deterministic, so the RFC's signature is the only right
        // answer. Its header and payload are sent as the RFC's exact bytes.
        let key = SigningKey::from_pem(RFC_7515_KEY).unwrap();
        let header = RawValue::from_string(r#"{"alg":"RS256"}"#.to_owned()).unwrap();
        let payload = RawValue::from_string(
            "{\"iss\":\"joe\",\r\n \"exp\":1300819380,\r\n \"http://example.com/is_root\":true}"
                .to_owned(),
        )
        .unwrap();
        let jws = key.sign(&header, &payload).unwrap();
        assert_eq!(
            jws,
            "eyJhbGciOiJSUzI1NiJ9\
             .eyJpc3MiOiJqb2UiLA0KICJleHAiOjEzMDA4MTkzODAsDQogImh0dHA6Ly9leGFtcGxlLmNvbS9pc19yb290Ijp0cnVlfQ\
             .cC4hiUPoj9Eetdgtv3hF80EGrhuB__dzERat0XF9g2VtQgr9PJbu3XOiZj5RZmh7\
             AAuHIm4Bh-0Qc_lF5YKt_O8W2Fp5jujGbds9uJdbF9CUAr7t1dnZcAcQjbKBYNX4\
             BAynRFdiuB--f_nZLgrnbyTyWzO75vRK5h6xBArLIARNPvkSjtQBMHlb1L07Qe7K\
             0GarZRmB_eSN9383LcOLn6_dO--xi12jzDwusC-eOkHWEsqtFZESc6BfI7noOPqv\
             hJ1phCnvWh6IeYI2w9QOYEUipUTI8np6LbgGY9Fs98rqVt5AXLIhWkWywlVmtVrB\
             p0igcN_IoypGlUPQGe77Rw"
        );
    }

    #[test]
    fn google_s_header_and_claims_are_encoded_as_asked() {
        let key = SigningKey::from_pem(RFC_7515_KEY).unwrap();
        let jwt = key
            .sign(
                &Header {
                    alg: "RS256",
                    typ: "JWT",
                    kid: Some("key-1"),
                },
                &Claims {
                    iss: "glave@acme-ai.iam.gserviceaccount.com",
                    scope: "https://www.googleapis.com/auth/cloud-platform",
                    aud: "https://oauth2.googleapis.com/token",
                    iat: 1_000,
                    exp: 4_600,
                },
            )
            .unwrap();
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3);
        let decode = |part: &str| String::from_utf8(URL_SAFE_NO_PAD.decode(part).unwrap()).unwrap();
        assert_eq!(
            decode(parts[0]),
            r#"{"alg":"RS256","typ":"JWT","kid":"key-1"}"#
        );
        assert_eq!(
            decode(parts[1]),
            r#"{"iss":"glave@acme-ai.iam.gserviceaccount.com","scope":"https://www.googleapis.com/auth/cloud-platform","aud":"https://oauth2.googleapis.com/token","iat":1000,"exp":4600}"#
        );
    }

    #[test]
    fn a_key_that_is_not_one_is_refused_without_showing_it() {
        for pem in [
            String::new(),
            format!("{BEGIN}\nbm90IGEga2V5\n{END}\n"),
            format!("{BEGIN}\nsecret!!\n{END}\n"),
        ] {
            let Err(error) = SigningKey::from_pem(&pem) else {
                panic!("{pem:?} is not a key");
            };
            assert!(!error.to_string().contains("secret"), "{error}");
            assert!(!error.is_transient());
        }
    }
}
