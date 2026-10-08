//! In-memory implementations of the storage ports, for tests and for runs that keep nothing.

use std::collections::{BTreeMap, HashMap};
use std::future::{Future, ready};
use std::sync::Mutex;

use glave_event::{BlobRef, RawEvent, Seq, Sha256, ThreadId};

use crate::blob::{BlobError, BlobStore, digest};
use crate::log::{EventLog, LogError, check_append};
use crate::scope::Scope;

/// An event log held in memory. Nothing survives the process.
#[derive(Debug, Default)]
pub struct MemoryLog {
    threads: Mutex<HashMap<(Scope, ThreadId), Vec<RawEvent>>>,
}

impl MemoryLog {
    pub fn new() -> Self {
        Self::default()
    }

    fn append_now(&self, scope: &Scope, events: &[RawEvent]) -> Result<(), LogError> {
        let Some(first) = events.first() else {
            return Ok(());
        };
        let mut threads = self
            .threads
            .lock()
            .map_err(|_| LogError::Storage("the in-memory log's lock is poisoned".into()))?;
        let thread = threads.entry((*scope, first.thread)).or_default();
        check_append(thread.last().map(|event| event.seq), events)?;
        thread.extend(events.iter().cloned());
        Ok(())
    }

    fn read_now(
        &self,
        scope: &Scope,
        thread: ThreadId,
        after: Option<Seq>,
        limit: usize,
    ) -> Result<Vec<RawEvent>, LogError> {
        let threads = self
            .threads
            .lock()
            .map_err(|_| LogError::Storage("the in-memory log's lock is poisoned".into()))?;
        let Some(events) = threads.get(&(*scope, thread)) else {
            return Ok(Vec::new());
        };
        Ok(events
            .iter()
            .filter(|event| after.is_none_or(|after| event.seq > after))
            .take(limit)
            .cloned()
            .collect())
    }
}

impl EventLog for MemoryLog {
    fn append(
        &self,
        scope: &Scope,
        events: &[RawEvent],
    ) -> impl Future<Output = Result<(), LogError>> + Send {
        ready(self.append_now(scope, events))
    }

    fn read(
        &self,
        scope: &Scope,
        thread: ThreadId,
        after: Option<Seq>,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<RawEvent>, LogError>> + Send {
        ready(self.read_now(scope, thread, after, limit))
    }
}

/// A blob store held in memory, keyed by scope and hash.
#[derive(Debug, Default)]
pub struct MemoryBlobs {
    blobs: Mutex<BTreeMap<(Scope, Sha256), Vec<u8>>>,
}

impl MemoryBlobs {
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

impl BlobStore for MemoryBlobs {
    fn put(
        &self,
        scope: &Scope,
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
    use crate::conformance;

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a current-thread runtime always builds")
            .block_on(future)
    }

    #[test]
    fn memory_log_passes_the_event_log_conformance_suite() {
        assert_eq!(block_on(conformance::event_log(&MemoryLog::new())), Ok(()));
    }

    #[test]
    fn memory_blobs_pass_the_blob_store_conformance_suite() {
        assert_eq!(
            block_on(conformance::blob_store(&MemoryBlobs::new())),
            Ok(())
        );
    }
}
