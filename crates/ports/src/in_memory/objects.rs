//! Objects held in memory, for tests and for runs that keep nothing.

use std::collections::BTreeMap;
use std::future::ready;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::blob::{BlobError, Chunks, Pieces};
use crate::objects::{ObjectWriter, Objects, object_name_ok};
use crate::scope::Scope;
use crate::tools::BoxFuture;

/// The bytes a read gives at a time: uneven, as a network gives them, so readers never rely on
/// where pieces end.
const PIECE: usize = 10_007;

/// Objects held in memory, by scope and name.
#[derive(Debug, Default, Clone)]
pub struct InMemoryObjects {
    held: Arc<Mutex<Held>>,
}

/// Every object, by scope and name.
type Held = BTreeMap<(Scope, String), Vec<u8>>;

impl InMemoryObjects {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(held: &Mutex<Held>) -> Result<MutexGuard<'_, Held>, BlobError> {
        held.lock()
            .map_err(|_| BlobError::Storage("the in-memory objects' lock is poisoned".into()))
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
            Ok(held.get(&(scope, object.to_owned())).map(|bytes| {
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
}

/// An object being written, kept only once it finishes.
struct Writer {
    held: Arc<Mutex<Held>>,
    key: (Scope, String),
    bytes: Vec<u8>,
}

impl ObjectWriter for Writer {
    fn write(&mut self, bytes: Vec<u8>) -> BoxFuture<'_, Result<(), BlobError>> {
        self.bytes.extend_from_slice(&bytes);
        Box::pin(ready(Ok(())))
    }

    fn finish(self: Box<Self>) -> BoxFuture<'static, Result<(), BlobError>> {
        let Writer { held, key, bytes } = *self;
        let finished = InMemoryObjects::lock(&held).map(|mut held| {
            held.insert(key, bytes);
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
