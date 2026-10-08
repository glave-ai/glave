//! The blob store port: content kept once by its SHA-256 hash and referenced from events.

use std::future::Future;

use glave_event::{BlobRef, Sha256};
use sha2::Digest as _;
use thiserror::Error;

use crate::scope::Scope;

/// Holds content by hash: turn configurations, large tool outputs, files.
pub trait BlobStore: Send + Sync {
    /// Stores `bytes` and returns a reference to them. Storing the same bytes twice returns
    /// the same reference and keeps one copy.
    fn put(
        &self,
        scope: &Scope,
        bytes: Vec<u8>,
        media_type: &str,
    ) -> impl Future<Output = Result<BlobRef, BlobError>> + Send;

    /// Returns the bytes stored under `sha256` in `scope`, if any.
    fn get(
        &self,
        scope: &Scope,
        sha256: &Sha256,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, BlobError>> + Send;
}

/// Why a blob could not be stored or read.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BlobError {
    #[error("blob storage failed: {0}")]
    Storage(String),
}

/// The SHA-256 digest of `bytes`, as every blob store names content.
pub fn digest(bytes: &[u8]) -> Sha256 {
    Sha256::from_digest(sha2::Sha256::digest(bytes).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_matches_the_known_sha256_of_test() {
        assert_eq!(
            digest(b"test").as_str(),
            "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
        );
    }
}
