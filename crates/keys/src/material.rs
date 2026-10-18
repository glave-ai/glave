//! Key material: 32 bytes that are wiped from memory when dropped and never printed.

use std::fmt;

use zeroize::Zeroizing;

use crate::error::KeyError;

/// A 256-bit key.
#[derive(Clone)]
pub struct KeyMaterial(Zeroizing<[u8; 32]>);

impl KeyMaterial {
    /// A fresh key from the operating system's random source.
    pub fn random() -> Result<Self, KeyError> {
        let mut bytes = Zeroizing::new([0u8; 32]);
        random(bytes.as_mut_slice())?;
        Ok(Self(bytes))
    }

    /// A key from exactly 32 bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, KeyError> {
        let mut key = Zeroizing::new([0u8; 32]);
        if bytes.len() != key.len() {
            return Err(KeyError::Corrupt(format!(
                "a key is 32 bytes, not {}",
                bytes.len()
            )));
        }
        key.copy_from_slice(bytes);
        Ok(Self(key))
    }

    pub fn bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for KeyMaterial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("KeyMaterial(..)")
    }
}

/// Fills `bytes` from the operating system's random source.
pub(crate) fn random(bytes: &mut [u8]) -> Result<(), KeyError> {
    getrandom::fill(bytes).map_err(|e| KeyError::Random(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_never_printed() {
        let key = KeyMaterial::from_bytes(&[7; 32]).unwrap();
        assert_eq!(format!("{key:?}"), "KeyMaterial(..)");
    }

    #[test]
    fn a_key_must_be_32_bytes() {
        assert!(KeyMaterial::from_bytes(&[1; 31]).is_err());
        assert_ne!(
            KeyMaterial::random().unwrap().bytes(),
            KeyMaterial::random().unwrap().bytes()
        );
    }
}
