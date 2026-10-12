//! Erasing forgotten memories: each is sealed under a key of its own, destroyed when it is
//! forgotten, and the space key is rotated afterwards so backups lose it too (`specs/storage.md`,
//! "Erasure").

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
