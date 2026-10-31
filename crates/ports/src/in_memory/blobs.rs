//! A blob store held in memory, for tests and for runs that keep nothing.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, MutexGuard};

use glave_event::{BlobRef, Sha256, ThreadId};

use crate::blob::{
    Blob, BlobError, BlobStore, Chunks, Pieces, Swept, UNREFERENCED_FOR_MS, digest, read_all,
};
use crate::env::{Clock, SystemClock};
use crate::scope::{OrgId, Scope};

/// A blob store held in memory, keyed by scope and hash, its uploads timed by its clock.
pub struct InMemoryBlobs {
    blobs: Mutex<Held>,
    clock: Arc<dyn Clock>,
}

/// Every blob, by scope and hash.
type Held = BTreeMap<(Scope, Sha256), Kept>;

/// One blob: what it is, its bytes, the threads that refer to it, whether one did since it was
/// last uploaded with none, and when that was.
#[derive(Debug)]
struct Kept {
    reference: BlobRef,
    bytes: Vec<u8>,
    threads: BTreeSet<ThreadId>,
    referred: bool,
    uploaded_ms: i64,
}

impl InMemoryBlobs {
    /// Blobs whose uploads are timed by the computer's clock.
    pub fn new() -> Self {
        Self::with_clock(Arc::new(SystemClock))
    }

    /// Blobs whose uploads are timed by `clock`.
    pub fn with_clock(clock: Arc<dyn Clock>) -> Self {
        Self {
            blobs: Mutex::default(),
            clock,
        }
    }

    fn lock(&self) -> Result<MutexGuard<'_, Held>, BlobError> {
        self.blobs
            .lock()
            .map_err(|_| BlobError::Storage("the in-memory blob store's lock is poisoned".into()))
    }
}

impl Default for InMemoryBlobs {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for InMemoryBlobs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InMemoryBlobs").finish_non_exhaustive()
    }
}

impl BlobStore for InMemoryBlobs {
    fn put(
        &self,
        scope: &Scope,
        thread: Option<ThreadId>,
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
            let now = self.clock.now().as_millisecond();
            let mut blobs = self.lock()?;
            let kept = blobs.entry((scope, sha256)).or_insert_with(|| Kept {
                reference: reference.clone(),
                bytes,
                threads: BTreeSet::new(),
                referred: false,
                uploaded_ms: now,
            });
            // An upload is kept a day from now, however a thread used it before.
            kept.referred = thread.is_some();
            if let Some(thread) = thread {
                kept.threads.insert(thread);
            } else {
                kept.uploaded_ms = now;
            }
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

    fn refer(
        &self,
        scope: &Scope,
        sha256: &Sha256,
        thread: ThreadId,
    ) -> impl Future<Output = Result<bool, BlobError>> + Send {
        let referred = self.lock().map(|mut blobs| {
            blobs
                .get_mut(&(*scope, sha256.clone()))
                .map(|kept| {
                    kept.threads.insert(thread);
                    kept.referred = true;
                })
                .is_some()
        });
        ready(referred)
    }

    fn release(
        &self,
        scope: &Scope,
        thread: ThreadId,
    ) -> impl Future<Output = Result<(), BlobError>> + Send {
        let released = self.lock().map(|mut blobs| {
            for ((kept_in, _), kept) in blobs.iter_mut() {
                if kept_in == scope {
                    kept.threads.remove(&thread);
                }
            }
        });
        ready(released)
    }

    fn sweep(&self, org: OrgId) -> impl Future<Output = Result<Swept, BlobError>> + Send {
        let before = self
            .clock
            .now()
            .as_millisecond()
            .saturating_sub(UNREFERENCED_FOR_MS);
        let swept = self.lock().map(|mut blobs| {
            let held = blobs.len();
            blobs.retain(|(scope, _), kept| {
                scope.org() != org
                    || !kept.threads.is_empty()
                    || (!kept.referred && kept.uploaded_ms >= before)
            });
            Swept {
                blobs: (held - blobs.len()) as u64,
                orphans: 0,
            }
        });
        ready(swept)
    }

    fn organizations(&self) -> impl Future<Output = Result<Vec<OrgId>, BlobError>> + Send {
        let organizations = self.lock().map(|blobs| {
            let held: BTreeSet<OrgId> = blobs.keys().map(|(scope, _)| scope.org()).collect();
            held.into_iter().collect()
        });
        ready(organizations)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::ManualClock;
    use crate::{block_on, conformance};

    #[test]
    fn in_memory_blobs_pass_the_blob_store_conformance_suite() {
        let clock = ManualClock::at_millis(0);
        let blobs = InMemoryBlobs::with_clock(Arc::new(clock.clone()));
        assert_eq!(
            block_on!(conformance::blob_store(&blobs, &clock, None)),
            Ok(())
        );
    }
}
