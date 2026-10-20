//! Objects held in memory, for tests and for runs that keep nothing.

use std::collections::BTreeMap;
use std::fmt;
use std::future::ready;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::blob::{BlobError, Chunks, Pieces};
use crate::env::{Clock, SystemClock};
use crate::objects::{Listed, ObjectWriter, Objects, object_name_ok};
use crate::scope::{OrgId, Scope};
use crate::tools::BoxFuture;

/// The bytes a read gives at a time: uneven, as a network gives them, so readers never rely on
/// where pieces end.
const PIECE: usize = 10_007;

/// Objects held in memory, by scope and name, each with when it was written by its clock.
/// Clones share them.
#[derive(Clone)]
pub struct InMemoryObjects {
    held: Arc<Mutex<Held>>,
    clock: Arc<dyn Clock>,
}

/// Every object, by scope and name: its bytes, and when it was written in milliseconds.
type Held = BTreeMap<(Scope, String), (Vec<u8>, i64)>;

impl InMemoryObjects {
    /// Objects written at the computer's time.
    pub fn new() -> Self {
        Self::with_clock(Arc::new(SystemClock))
    }

    /// Objects written at `clock`'s time.
    pub fn with_clock(clock: Arc<dyn Clock>) -> Self {
        Self {
            held: Arc::default(),
            clock,
        }
    }

    fn lock(held: &Mutex<Held>) -> Result<MutexGuard<'_, Held>, BlobError> {
        held.lock()
            .map_err(|_| BlobError::Storage("the in-memory objects' lock is poisoned".into()))
    }
}

impl Default for InMemoryObjects {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for InMemoryObjects {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InMemoryObjects").finish_non_exhaustive()
    }
}

fn named(object: &str) -> Result<(), BlobError> {
    if object_name_ok(object) {
        Ok(())
    } else {
        Err(BlobError::Storage(format!(
            "{object:?} is not an object name"
        )))
    }
}

impl Objects for InMemoryObjects {
    fn create(
        &self,
        scope: Scope,
        object: &str,
    ) -> BoxFuture<'_, Result<Box<dyn ObjectWriter>, BlobError>> {
        let writer = named(object).map(|()| {
            Box::new(Writer {
                held: Arc::clone(&self.held),
                clock: Arc::clone(&self.clock),
                key: (scope, object.to_owned()),
                bytes: Vec::new(),
            }) as Box<dyn ObjectWriter>
        });
        Box::pin(ready(writer))
    }

    fn read(
        &self,
        scope: Scope,
        object: &str,
    ) -> BoxFuture<'_, Result<Option<Box<dyn Chunks>>, BlobError>> {
        let found = named(object).and_then(|()| {
            let held = Self::lock(&self.held)?;
            Ok(held.get(&(scope, object.to_owned())).map(|(bytes, _)| {
                Box::new(bytes.chunks(PIECE).map(<[u8]>::to_vec).collect::<Pieces>())
                    as Box<dyn Chunks>
            }))
        });
        Box::pin(ready(found))
    }

    fn delete(&self, scope: Scope, object: &str) -> BoxFuture<'_, Result<(), BlobError>> {
        let deleted = named(object).and_then(|()| {
            Self::lock(&self.held)?.remove(&(scope, object.to_owned()));
            Ok(())
        });
        Box::pin(ready(deleted))
    }

    fn list(&self, org: OrgId) -> BoxFuture<'_, Result<Vec<Listed>, BlobError>> {
        let listed = Self::lock(&self.held).map(|held| {
            held.iter()
                .filter(|((scope, _), _)| scope.org() == org)
                .map(|((scope, object), (_, written_ms))| Listed {
                    scope: *scope,
                    object: object.clone(),
                    written_ms: *written_ms,
                })
                .collect()
        });
        Box::pin(ready(listed))
    }
}

/// An object being written, kept only once it finishes.
struct Writer {
    held: Arc<Mutex<Held>>,
    clock: Arc<dyn Clock>,
    key: (Scope, String),
    bytes: Vec<u8>,
}

impl ObjectWriter for Writer {
    fn write(&mut self, bytes: Vec<u8>) -> BoxFuture<'_, Result<(), BlobError>> {
        self.bytes.extend_from_slice(&bytes);
        Box::pin(ready(Ok(())))
    }

    fn finish(self: Box<Self>) -> BoxFuture<'static, Result<(), BlobError>> {
        let Writer {
            held,
            clock,
            key,
            bytes,
        } = *self;
        let written_ms = clock.now().as_millisecond();
        let finished = InMemoryObjects::lock(&held).map(|mut held| {
            held.insert(key, (bytes, written_ms));
        });
        Box::pin(ready(finished))
    }

    fn abort(self: Box<Self>) -> BoxFuture<'static, Result<(), BlobError>> {
        Box::pin(ready(Ok(())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{block_on, conformance};

    #[test]
    fn in_memory_objects_pass_the_objects_conformance_suite() {
        assert_eq!(
            block_on!(conformance::objects(&InMemoryObjects::new())),
            Ok(())
        );
    }
}
