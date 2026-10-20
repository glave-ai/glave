//! The blob store port: content kept once by its SHA-256 hash and referenced from events,
//! written and read as a stream of chunks, kept while a thread refers to it.

use std::collections::VecDeque;
use std::fmt;
use std::future::{Future, ready};

use glave_event::{BlobRef, Sha256, ThreadId};
use sha2::Digest as _;
use thiserror::Error;

use crate::scope::Scope;
use crate::tools::BoxFuture;

/// Holds content by hash: turn configurations, large tool outputs, files. A blob's record
/// (its name, size, key and the threads that refer to it) is kept apart from its bytes.
pub trait BlobStore: Send + Sync {
    /// Stores what `bytes` yields, at most `limit` bytes, for `thread`, which refers to it, and
    /// returns a reference to it. Storing the same content twice keeps one copy; each thread
    /// that stores it is recorded, so erasing a thread can find the blobs only it used. More
    /// than `limit` bytes is refused ([`BlobError::TooLarge`]) and nothing is kept.
    fn put(
        &self,
        scope: &Scope,
        thread: ThreadId,
        media_type: &str,
        limit: u64,
        bytes: Box<dyn Chunks>,
    ) -> impl Future<Output = Result<BlobRef, BlobError>> + Send;

    /// The blob stored under `sha256` in `scope`, if any, its bytes read as they are taken.
    fn get(
        &self,
        scope: &Scope,
        sha256: &Sha256,
    ) -> impl Future<Output = Result<Option<Blob>, BlobError>> + Send;

    /// Drops every reference `thread` holds, as deleting or erasing it does, and deletes the
    /// blobs it referred to that no thread refers to any more. A blob another thread refers to
    /// stays.
    fn release(
        &self,
        scope: &Scope,
        thread: ThreadId,
    ) -> impl Future<Output = Result<(), BlobError>> + Send;
}

/// A blob found: what it is, and its bytes.
pub struct Blob {
    pub reference: BlobRef,
    pub bytes: Box<dyn Chunks>,
}

impl fmt::Debug for Blob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Blob")
            .field("reference", &self.reference)
            .finish_non_exhaustive()
    }
}

/// Bytes that arrive a piece at a time: a blob being stored, or one being read.
pub trait Chunks: Send {
    /// The next piece, or `None` once every byte has come.
    fn next_chunk(&mut self) -> BoxFuture<'_, Result<Option<Vec<u8>>, BlobError>>;
}

/// Bytes already in memory, given out in the pieces they were made of.
#[derive(Debug, Default)]
pub struct Pieces(VecDeque<Vec<u8>>);

impl From<Vec<u8>> for Pieces {
    fn from(bytes: Vec<u8>) -> Self {
        Self(VecDeque::from([bytes]))
    }
}

impl FromIterator<Vec<u8>> for Pieces {
    fn from_iter<I: IntoIterator<Item = Vec<u8>>>(pieces: I) -> Self {
        Self(pieces.into_iter().collect())
    }
}

impl Chunks for Pieces {
    fn next_chunk(&mut self) -> BoxFuture<'_, Result<Option<Vec<u8>>, BlobError>> {
        Box::pin(ready(Ok(self.0.pop_front())))
    }
}

/// Every byte `chunks` yields, refused once there are more than `limit`.
pub async fn read_all(chunks: &mut dyn Chunks, limit: u64) -> Result<Vec<u8>, BlobError> {
    let mut out = Vec::new();
    while let Some(chunk) = chunks.next_chunk().await? {
        out.extend_from_slice(&chunk);
        if out.len() as u64 > limit {
            return Err(BlobError::TooLarge(limit));
        }
    }
    Ok(out)
}

/// Why a blob could not be stored or read.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BlobError {
    #[error("blob storage failed: {0}")]
    Storage(String),
    /// More bytes than the limit given.
    #[error("the content is larger than {0} bytes")]
    TooLarge(u64),
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

    #[test]
    fn pieces_are_read_whole_up_to_a_limit() {
        let mut pieces: Pieces = [b"ab".to_vec(), Vec::new(), b"c".to_vec()]
            .into_iter()
            .collect();
        assert_eq!(
            crate::block_on!(read_all(&mut pieces, 3)),
            Ok(b"abc".to_vec())
        );
        let mut more = Pieces::from(b"abcd".to_vec());
        assert_eq!(
            crate::block_on!(read_all(&mut more, 3)),
            Err(BlobError::TooLarge(3))
        );
    }
}
