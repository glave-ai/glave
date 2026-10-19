//! Erasing what is sealed under a key of its own, a memory or an account, by destroying that
//! key, then rotating the project key so backups lose it too (`specs/storage.md`, "Erasure").

use glave_event::MemoryId;
use thiserror::Error;
use uuid::Uuid;

use crate::scope::Scope;
use crate::tools::BoxFuture;

/// What is sealed under a key of its own, so destroying that key erases it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Erasable {
    /// A memory: every version of it.
    Memory(MemoryId),
    /// An account a person connected: a model account or a git host.
    Account(Uuid),
}

/// The keys could not be destroyed or rotated.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct EraseError(pub String);

/// What erases by destroying keys: the data store's keys.
pub trait Eraser: Send + Sync {
    /// Destroys every key of `item` in `scope`: it is unreadable in the store at once. Backups
    /// still hold it until [`Eraser::rotate`].
    fn destroy(&self, scope: Scope, item: Erasable) -> BoxFuture<'_, Result<(), EraseError>>;

    /// Rotates `scope`'s project key when a key was destroyed under it, so backups lose it
    /// too. Whether it rotated.
    fn rotate(&self, scope: Scope) -> BoxFuture<'_, Result<bool, EraseError>>;

    /// Erases `item` from the store and from backups at once: destroys its keys, then rotates.
    fn erase(&self, scope: Scope, item: Erasable) -> BoxFuture<'_, Result<(), EraseError>> {
        Box::pin(async move {
            self.destroy(scope, item).await?;
            self.rotate(scope).await?;
            Ok(())
        })
    }
}
