//! The sealed format: XChaCha20-Poly1305 with a random 24-byte nonce, stored as
//! nonce ‖ ciphertext ‖ tag, and associated data built from length-prefixed fields.

use chacha20poly1305::aead::{Aead as _, KeyInit as _, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

use crate::error::KeyError;
use crate::secret::{Secret, random};

const NONCE: usize = 24;

/// Associated data from `fields`, each preceded by its length as four big-endian bytes, so
/// no two different lists of fields encode alike.
pub(crate) fn associated(fields: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(fields.iter().map(|field| field.len() + 4).sum());
    for field in fields {
        let length = u32::try_from(field.len()).unwrap_or(u32::MAX);
        out.extend_from_slice(&length.to_be_bytes());
        out.extend_from_slice(field);
    }
    out
}

/// Seals `plaintext` under `key`, bound to `aad`.
pub(crate) fn seal(key: &Secret, aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, KeyError> {
    let cipher = XChaCha20Poly1305::new(&(*key.bytes()).into());
    let mut nonce = [0u8; NONCE];
    random(&mut nonce)?;
    let sealed = cipher
        .encrypt(
            &XNonce::from(nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| KeyError::Corrupt("the content is too large to seal".into()))?;
    let mut out = Vec::with_capacity(NONCE + sealed.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&sealed);
    Ok(out)
}

/// Opens what [`seal`] made under `key` and the same `aad`. `name` names the key in the
/// error when the bytes do not open.
pub(crate) fn open(
    key: &Secret,
    aad: &[u8],
    sealed: &[u8],
    name: &str,
) -> Result<Vec<u8>, KeyError> {
    let tampered = || KeyError::Tampered(name.to_owned());
    let (nonce, body) = sealed.split_at_checked(NONCE).ok_or_else(tampered)?;
    let nonce: [u8; NONCE] = nonce.try_into().map_err(|_| tampered())?;
    let cipher = XChaCha20Poly1305::new(&(*key.bytes()).into());
    cipher
        .decrypt(&XNonce::from(nonce), Payload { msg: body, aad })
        .map_err(|_| tampered())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealed_content_opens_only_with_its_key_and_associated_data() {
        let key = Secret::random().unwrap();
        let aad = associated(&[b"row", b"1"]);
        let sealed = seal(&key, &aad, b"hello").unwrap();
        assert_eq!(sealed.len(), 24 + 5 + 16, "nonce, ciphertext, tag");
        assert_eq!(open(&key, &aad, &sealed, "k").unwrap(), b"hello");
        let other_row = associated(&[b"row", b"2"]);
        assert_eq!(
            open(&key, &other_row, &sealed, "k"),
            Err(KeyError::Tampered("k".into()))
        );
        let other_key = Secret::random().unwrap();
        assert!(open(&other_key, &aad, &sealed, "k").is_err());
        let mut flipped = sealed.clone();
        if let Some(byte) = flipped.last_mut() {
            *byte ^= 1;
        }
        assert!(open(&key, &aad, &flipped, "k").is_err());
        assert!(open(&key, &aad, &sealed[..10], "k").is_err());
    }

    #[test]
    fn length_prefixes_keep_field_boundaries() {
        assert_ne!(associated(&[b"ab", b"c"]), associated(&[b"a", b"bc"]));
    }

    #[test]
    fn every_seal_uses_a_fresh_nonce() {
        let key = Secret::random().unwrap();
        assert_ne!(
            seal(&key, b"", b"same").unwrap(),
            seal(&key, b"", b"same").unwrap()
        );
    }
}
