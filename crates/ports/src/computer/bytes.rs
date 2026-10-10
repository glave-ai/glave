//! File contents that cross the wire intact: any bytes, carried in JSON as standard base64.

use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// A file's bytes. Serialized as base64, so binary files survive JSON.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Bytes(Vec<u8>);

impl Bytes {
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self(bytes.into())
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }

    pub fn into_vec(self) -> Vec<u8> {
        self.0
    }
}

impl fmt::Debug for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Bytes({} bytes)", self.0.len())
    }
}

impl Serialize for Bytes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&encode(&self.0))
    }
}

impl<'de> Deserialize<'de> for Bytes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        decode(&text).map(Self).map_err(D::Error::custom)
    }
}

fn symbol(index: u32) -> char {
    ALPHABET
        .get(usize::try_from(index & 63).unwrap_or(0))
        .map_or('A', |byte| char::from(*byte))
}

/// Standard base64, padded.
pub(crate) fn encode(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len().div_ceil(3).saturating_mul(4));
    for chunk in bytes.chunks(3) {
        let mut group = [0u8; 3];
        for (slot, byte) in group.iter_mut().zip(chunk) {
            *slot = *byte;
        }
        let [a, b, c] = group;
        let n = (u32::from(a) << 16) | (u32::from(b) << 8) | u32::from(c);
        text.push(symbol(n >> 18));
        text.push(symbol(n >> 12));
        text.push(if chunk.len() > 1 { symbol(n >> 6) } else { '=' });
        text.push(if chunk.len() > 2 { symbol(n) } else { '=' });
    }
    text
}

fn value(symbol: u8) -> Option<u32> {
    ALPHABET
        .iter()
        .position(|candidate| *candidate == symbol)
        .and_then(|index| u32::try_from(index).ok())
}

/// Reads standard base64, padded; anything else is refused.
pub(crate) fn decode(text: &str) -> Result<Vec<u8>, String> {
    let input = text.as_bytes();
    if !input.len().is_multiple_of(4) {
        return Err("base64 length is not a multiple of 4".into());
    }
    let mut bytes = Vec::with_capacity(input.len() / 4 * 3);
    let groups = input.len() / 4;
    for (index, group) in input.chunks(4).enumerate() {
        let padding = group.iter().rev().take_while(|byte| **byte == b'=').count();
        if padding > 2 || (padding > 0 && index + 1 != groups) {
            return Err("base64 padding in the wrong place".into());
        }
        let mut n = 0u32;
        for (position, symbol) in group.iter().enumerate() {
            let digit = if position >= 4 - padding {
                0
            } else {
                value(*symbol).ok_or_else(|| format!("{:?} is not base64", char::from(*symbol)))?
            };
            n = (n << 6) | digit;
        }
        let [_, a, b, c] = n.to_be_bytes();
        bytes.push(a);
        if padding < 2 {
            bytes.push(b);
        }
        if padding < 1 {
            bytes.push(c);
        }
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn known_values() {
        for (plain, coded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(encode(plain.as_bytes()), coded);
            assert_eq!(decode(coded).unwrap(), plain.as_bytes());
        }
        assert!(decode("Zg=").is_err());
        assert!(decode("Zg==Zm9v").is_err());
        assert!(decode("Z!==").is_err());
    }

    proptest! {
        #[test]
        fn any_bytes_round_trip(bytes in proptest::collection::vec(any::<u8>(), 0..300)) {
            let json = serde_json::to_string(&Bytes::new(bytes.clone())).unwrap();
            let back: Bytes = serde_json::from_str(&json).unwrap();
            prop_assert_eq!(back.into_vec(), bytes);
        }

        #[test]
        fn any_text_decodes_or_is_refused(text in ".{0,40}") {
            let _ = decode(&text);
        }
    }
}
