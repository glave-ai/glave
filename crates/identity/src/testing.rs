//! A passkey authenticator in software, for tests: it answers the options glave hands a browser
//! as a platform authenticator would, with a P-256 key of its own, attestation `none`, the
//! person verified, and a counter that goes up at each use. The browser's part (the client
//! data, with the page's origin) it plays too. Its JSON is WebAuthn's, buffers in base64url.

use aws_lc_rs::digest::{SHA256, digest};
use aws_lc_rs::error::Unspecified;
use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair as _};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};

/// The flags an answer carries: the person present and verified; with `ATTESTED`, a new
/// credential's data follows.
const PRESENT: u8 = 0x01;
const VERIFIED: u8 = 0x04;
const ATTESTED: u8 = 0x40;

/// One authenticator holding one passkey, for the page at `origin`.
pub struct Authenticator {
    origin: String,
    key: EcdsaKeyPair,
    id: [u8; 16],
    counter: u32,
    random: SystemRandom,
}

impl Authenticator {
    /// An authenticator for the page at `origin`, such as `https://glave.test`, with a new key.
    pub fn new(origin: &str) -> Result<Self, Unspecified> {
        let random = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &random)?;
        let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref())?;
        let named = digest(&SHA256, pkcs8.as_ref());
        let id = named
            .as_ref()
            .get(..16)
            .and_then(|start| <[u8; 16]>::try_from(start).ok())
            .ok_or(Unspecified)?;
        Ok(Self {
            origin: origin.to_owned(),
            key,
            id,
            counter: 0,
            random,
        })
    }

    /// Its answers come from the page at `origin` from now on, as when another page asks it
    /// for the same passkey.
    pub fn on_page(&mut self, origin: &str) {
        origin.clone_into(&mut self.origin);
    }

    /// The answer to `options`, the options to make a passkey (`{"publicKey": …}`): a new
    /// credential, as `navigator.credentials.create` gives it.
    pub fn create(&mut self, options: &Value) -> Result<Value, Unspecified> {
        let options = options.get("publicKey").unwrap_or(&Value::Null);
        let client = self.client("webauthn.create", options);
        let public = self.key.public_key().as_ref();
        let x = public.get(1..33).ok_or(Unspecified)?;
        let y = public.get(33..65).ok_or(Unspecified)?;
        // The COSE key (RFC 9053): EC2, ES256, P-256, x and y.
        let mut key = vec![0xa5, 0x01, 0x02, 0x03, 0x26, 0x20, 0x01, 0x21, 0x58, 0x20];
        key.extend_from_slice(x);
        key.extend_from_slice(&[0x22, 0x58, 0x20]);
        key.extend_from_slice(y);
        let mut data = self.data(options, PRESENT | VERIFIED | ATTESTED);
        // The authenticator's model (none), and the credential: its id and key.
        data.extend_from_slice(&[0; 16]);
        data.extend_from_slice(&[0, 16]);
        data.extend_from_slice(&self.id);
        data.extend_from_slice(&key);
        let mut attestation = vec![0xa3];
        attestation.extend_from_slice(&text("fmt"));
        attestation.extend_from_slice(&text("none"));
        attestation.extend_from_slice(&text("attStmt"));
        attestation.push(0xa0);
        attestation.extend_from_slice(&text("authData"));
        attestation.extend_from_slice(&bytes(&data)?);
        Ok(json!({
            "id": URL_SAFE_NO_PAD.encode(self.id),
            "rawId": URL_SAFE_NO_PAD.encode(self.id),
            "type": "public-key",
            "response": {
                "attestationObject": URL_SAFE_NO_PAD.encode(attestation),
                "clientDataJSON": URL_SAFE_NO_PAD.encode(client),
                "transports": ["internal"],
            },
            "extensions": {},
        }))
    }

    /// The answer to `options`, the options to sign in (`{"publicKey": …}`): an assertion with
    /// the passkey, as `navigator.credentials.get` gives it.
    pub fn get(&mut self, options: &Value) -> Result<Value, Unspecified> {
        let options = options.get("publicKey").unwrap_or(&Value::Null);
        let client = self.client("webauthn.get", options);
        let data = self.data(options, PRESENT | VERIFIED);
        let mut signed = data.clone();
        signed.extend_from_slice(digest(&SHA256, &client).as_ref());
        let signature = self.key.sign(&self.random, &signed)?;
        Ok(json!({
            "id": URL_SAFE_NO_PAD.encode(self.id),
            "rawId": URL_SAFE_NO_PAD.encode(self.id),
            "type": "public-key",
            "response": {
                "authenticatorData": URL_SAFE_NO_PAD.encode(data),
                "clientDataJSON": URL_SAFE_NO_PAD.encode(client),
                "signature": URL_SAFE_NO_PAD.encode(signature.as_ref()),
            },
            "extensions": {},
        }))
    }

    /// The browser's client data for a request of `kind` with `options`.
    fn client(&self, kind: &str, options: &Value) -> Vec<u8> {
        json!({
            "type": kind,
            "challenge": options.get("challenge"),
            "origin": self.origin,
            "crossOrigin": false,
        })
        .to_string()
        .into_bytes()
    }

    /// The authenticator's data for `options`, with `flags`, counting this use.
    fn data(&mut self, options: &Value, flags: u8) -> Vec<u8> {
        let party = options
            .get("rp")
            .and_then(|party| party.get("id"))
            .or_else(|| options.get("rpId"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        self.counter = self.counter.saturating_add(1);
        let mut data = digest(&SHA256, party.as_bytes()).as_ref().to_vec();
        data.push(flags);
        data.extend_from_slice(&self.counter.to_be_bytes());
        data
    }
}

/// `text`, of fewer than 24 bytes, as a CBOR text string.
fn text(text: &str) -> Vec<u8> {
    let mut encoded = vec![0x60 | u8::try_from(text.len()).unwrap_or_default()];
    encoded.extend_from_slice(text.as_bytes());
    encoded
}

/// `data`, of fewer than 65,536 bytes, as a CBOR byte string.
fn bytes(data: &[u8]) -> Result<Vec<u8>, Unspecified> {
    let length = u16::try_from(data.len()).map_err(|_| Unspecified)?;
    let mut encoded = vec![0x59];
    encoded.extend_from_slice(&length.to_be_bytes());
    encoded.extend_from_slice(data);
    Ok(encoded)
}
