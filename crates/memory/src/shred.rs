//! Erasing forgotten memories: each is sealed under a key of its own, destroyed when it is
//! forgotten, and the space key is rotated afterwards so backups lose it too (`specs/storage.md`,
//! "Erasure").

use std::sync::Arc;

use glave_event::MemoryId;
use glave_keys::{KeyStorage, Keys};
use glave_ports::{BoxFuture, Scope};

/// What destroys a memory's key and rotates a space's.
pub trait Shredder: Send + Sync {
    /// Destroys every key of `memory` in `scope`: every version becomes unreadable at once.
    fn destroy(&self, scope: Scope, memory: MemoryId) -> BoxFuture<'_, Result<(), String>>;

    /// Rotates `scope`'s space key when a key was destroyed under it. Whether it rotated.
    fn rotate(&self, scope: Scope) -> BoxFuture<'_, Result<bool, String>>;
}

impl<S: KeyStorage + 'static> Shredder for Keys<S> {
    fn destroy(&self, scope: Scope, memory: MemoryId) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            self.destroy_memory_key(&scope, memory.as_uuid())
                .await
                .map_err(|error| error.to_string())
        })
    }

    fn rotate(&self, scope: Scope) -> BoxFuture<'_, Result<bool, String>> {
        Box::pin(async move {
            self.rotate_space_key(&scope)
                .await
                .map_err(|error| error.to_string())
        })
    }
}

impl<T: Shredder + ?Sized> Shredder for Arc<T> {
    fn destroy(&self, scope: Scope, memory: MemoryId) -> BoxFuture<'_, Result<(), String>> {
        (**self).destroy(scope, memory)
    }

    fn rotate(&self, scope: Scope) -> BoxFuture<'_, Result<bool, String>> {
        (**self).rotate(scope)
    }
}

/// For a log that seals nothing, such as one held in memory for tests: there is no key to
/// destroy, and forgetting still removes the memory from every view.
pub struct NoKeys;

impl Shredder for NoKeys {
    fn destroy(&self, _: Scope, _: MemoryId) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(std::future::ready(Ok(())))
    }

    fn rotate(&self, _: Scope) -> BoxFuture<'_, Result<bool, String>> {
        Box::pin(std::future::ready(Ok(false)))
    }
}
