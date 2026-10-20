//! A blob store held in memory, for tests and for runs that keep nothing.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::{Mutex, MutexGuard};

use glave_event::{BlobRef, Sha256, ThreadId};

use crate::blob::{Blob, BlobError, BlobStore, Chunks, Pieces, digest, read_all};
use crate::scope::Scope;

/// A blob store held in memory, keyed by scope and hash.
#[derive(Debug, Default)]
pub struct InMemoryBlobs {
    blobs: Mutex<Held>,
}

/// Every blob, by scope and hash.
type Held = BTreeMap<(Scope, Sha256), Kept>;

/// One blob: what it is, its bytes, and the threads that refer to it.
#[derive(Debug)]
struct Kept {
    reference: BlobRef,
    bytes: Vec<u8>,
    threads: BTreeSet<ThreadId>,
}

impl InMemoryBlobs {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> Result<MutexGuard<'_, Held>, BlobError> {
        self.blobs
            .lock()
            .map_err(|_| BlobError::Storage("the in-memory blob store's lock is poisoned".into()))
    }
}

impl BlobStore for InMemoryBlobs {
    fn put(
        &self,
        scope: &Scope,
        thread: ThreadId,
        media_type: &str,
        limit: u64,
        mut bytes: Box<dyn Chunks>,
    ) -> impl Future<Output = Result<BlobRef, BlobError>> + Send {
        let scope = *scope;
        let media_type = media_type.to_owned();
        async move {
            let bytes = read_all(bytes.as_mut(), limit).await?;
            let sha256 = digest(&bytes);
            let reference = BlobRef {
                sha256: sha256.clone(),
                bytes: bytes.len() as u64,
                media_type,
            };
            let mut blobs = self.lock()?;
            let kept = blobs.entry((scope, sha256)).or_insert_with(|| Kept {
                reference: reference.clone(),
                bytes,
                threads: BTreeSet::new(),
            });
            kept.threads.insert(thread);
            Ok(kept.reference.clone())
        }
    }

    fn get(
        &self,
        scope: &Scope,
        sha256: &Sha256,
    ) -> impl Future<Output = Result<Option<Blob>, BlobError>> + Send {
        let found = self.lock().map(|blobs| {
            blobs.get(&(*scope, sha256.clone())).map(|kept| Blob {
                reference: kept.reference.clone(),
                bytes: Box::new(Pieces::from(kept.bytes.clone())),
            })
        });
        async move { found }
    }

    fn release(
        &self,
        scope: &Scope,
        thread: ThreadId,
    ) -> impl Future<Output = Result<(), BlobError>> + Send {
        let released = self.lock().map(|mut blobs| {
            blobs.retain(|(kept_in, _), kept| {
                if kept_in != scope || !kept.threads.remove(&thread) {
                    return true;
                }
                !kept.threads.is_empty()
            });
        });
        async move { released }
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
