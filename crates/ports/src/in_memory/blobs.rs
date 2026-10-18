//! A blob store held in memory, for tests and for runs that keep nothing.

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Mutex;

use glave_event::{BlobRef, Sha256, ThreadId};

use crate::blob::{BlobError, BlobStore, digest};
use crate::scope::Scope;

/// A blob store held in memory, keyed by scope and hash.
#[derive(Debug, Default)]
pub struct InMemoryBlobs {
    blobs: Mutex<BTreeMap<(Scope, Sha256), Vec<u8>>>,
}

impl InMemoryBlobs {
    pub fn new() -> Self {
        Self::default()
    }

    fn put_now(
        &self,
        scope: &Scope,
        bytes: Vec<u8>,
        media_type: &str,
    ) -> Result<BlobRef, BlobError> {
        let sha256 = digest(&bytes);
        let reference = BlobRef {
            sha256: sha256.clone(),
            bytes: u64::try_from(bytes.len())
                .map_err(|_| BlobError::Storage("blob is larger than 2^64 bytes".into()))?,
            media_type: media_type.to_owned(),
        };
        let mut blobs = self.blobs.lock().map_err(|_| {
            BlobError::Storage("the in-memory blob store's lock is poisoned".into())
        })?;
        blobs.entry((*scope, sha256)).or_insert(bytes);
        Ok(reference)
    }

    fn get_now(&self, scope: &Scope, sha256: &Sha256) -> Result<Option<Vec<u8>>, BlobError> {
        let blobs = self.blobs.lock().map_err(|_| {
            BlobError::Storage("the in-memory blob store's lock is poisoned".into())
        })?;
        Ok(blobs.get(&(*scope, sha256.clone())).cloned())
    }
}

impl BlobStore for InMemoryBlobs {
    fn put(
        &self,
        scope: &Scope,
        _thread: ThreadId,
        bytes: Vec<u8>,
        media_type: &str,
    ) -> impl Future<Output = Result<BlobRef, BlobError>> + Send {
        ready(self.put_now(scope, bytes, media_type))
    }

    fn get(
        &self,
        scope: &Scope,
        sha256: &Sha256,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, BlobError>> + Send {
        ready(self.get_now(scope, sha256))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{block_on, conformance};

    #[test]
    fn in_memory_blobs_pass_the_blob_store_conformance_suite() {
        assert_eq!(
            block_on!(conformance::blob_store(&InMemoryBlobs::new())),
            Ok(())
        );
    }
}
