//! References to content stored once by hash, such as large tool outputs and turn configs.

use serde::{Deserialize, Serialize};

use crate::error::InvalidValue;

/// A SHA-256 digest as 64 lowercase hexadecimal digits.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Sha256(String);

impl Sha256 {
    pub fn new(hex: impl Into<String>) -> Result<Self, InvalidValue> {
        let hex = hex.into();
        let valid = hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if valid {
            Ok(Self(hex))
        } else {
            Err(InvalidValue::new(
                "sha256",
                "must be 64 lowercase hexadecimal digits",
            ))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Sha256 {
    type Error = InvalidValue;

    fn try_from(hex: String) -> Result<Self, Self::Error> {
        Self::new(hex)
    }
}

impl From<Sha256> for String {
    fn from(digest: Sha256) -> Self {
        digest.0
    }
}

/// Points to content held in the blob store instead of inside an event.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlobRef {
    pub sha256: Sha256,
    pub bytes: u64,
    pub media_type: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digests_are_validated() {
        let good = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
        assert!(Sha256::new(good).is_ok());
        assert!(Sha256::new(good.to_uppercase()).is_err());
        assert!(Sha256::new("abc").is_err());
        assert!(Sha256::new(good.replace('9', "g")).is_err());
    }
}
