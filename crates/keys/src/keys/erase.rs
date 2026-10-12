//! Erasure: destroying a space key, or a data key followed by rotating the space key.

use glave_ports::Scope;
use uuid::Uuid;

use crate::error::KeyError;
use crate::key_id::{KeyId, Period};
use crate::record::{SpaceRecord, SpaceVersion};
use crate::secret::Secret;
use crate::storage::KeyStorage;

use super::hierarchy::{ATTEMPTS, SpaceKey, key_id, unwrap_data, wrap_data, wrap_space};
use super::{Keys, check_space};

impl<S: KeyStorage> Keys<S> {
    /// Erases a space: destroys every version of its space key, and with them every data key
    /// they wrapped. Everything sealed in the space becomes unreadable, in the store and in any
    /// backup. New content in the space gets new keys.
    pub async fn erase_space(&self, scope: &Scope) -> Result<(), KeyError> {
        for _ in 0..ATTEMPTS {
            let (revision, mut record) = self.load(scope).await?;
            if revision.is_none() {
                return Ok(());
            }
            for version in &mut record.versions {
                version.wrapped = None;
            }
            for key in &mut record.data {
                key.wrapped = None;
            }
            let saved = self
                .storage
                .save_space(scope, revision, record.encode()?)
                .await?;
            self.cache.forget(*scope);
            if saved {
                return Ok(());
            }
        }
        Err(KeyError::Contended(scope.to_string()))
    }

    /// Erases what one data key sealed: destroys it, then rotates the space key so backups
    /// holding the old wrapped data key can no longer unwrap it. Every surviving data key is
    /// re-wrapped under a new space-key version and the old versions are destroyed, all in
    /// one save.
    pub async fn destroy_data_key(&self, scope: &Scope, id: KeyId) -> Result<(), KeyError> {
        check_space(scope, &id)?;
        for _ in 0..ATTEMPTS {
            let (revision, mut record) = self.load(scope).await?;
            if record.data_key(id.period, id.n).is_none() {
                return Err(KeyError::Unknown(id.to_string()));
            }
            for entry in &mut record.data {
                if key_id(scope, entry) == id {
                    entry.wrapped = None;
                }
            }
            // With no space key left, everything is already unreadable; there is nothing to
            // rotate.
            if record.live().is_none() {
                return Ok(());
            }
            self.rotate(scope, &mut record).await?;
            let saved = self
                .storage
                .save_space(scope, revision, record.encode()?)
                .await?;
            self.cache.forget(*scope);
            if saved {
                return Ok(());
            }
        }
        Err(KeyError::Contended(scope.to_string()))
    }

    /// Destroys every key of one memory at once, so every version of it is unreadable in the
    /// store from now on. Backups still hold it wrapped under the space key in use until
    /// [`Keys::rotate_space_key`] runs (`specs/memory.md`: within a day, batching the day's
    /// forgets). A memory that never had a key has nothing to destroy.
    pub async fn destroy_memory_key(&self, scope: &Scope, memory: Uuid) -> Result<(), KeyError> {
        for _ in 0..ATTEMPTS {
            let (revision, mut record) = self.load(scope).await?;
            let mut changed = false;
            for entry in &mut record.data {
                if entry.period() == Period::Memory(memory) && entry.wrapped.is_some() {
                    entry.wrapped = None;
                    changed = true;
                }
            }
            if !changed {
                return Ok(());
            }
            let saved = self
                .storage
                .save_space(scope, revision, record.encode()?)
                .await?;
            self.cache.forget(*scope);
            if saved {
                return Ok(());
            }
        }
        Err(KeyError::Contended(scope.to_string()))
    }

    /// Whether a data key was destroyed since the space key was last rotated, so backups can
    /// still unwrap it.
    pub async fn rotation_due(&self, scope: &Scope) -> Result<bool, KeyError> {
        let (_, record) = self.load(scope).await?;
        Ok(record.rotation_due())
    }

    /// Rotates the space key when a data key was destroyed under the one in use: the surviving
    /// data keys are re-wrapped under a new version and the old versions destroyed, so
    /// backups lose every destroyed key. Returns whether it rotated.
    pub async fn rotate_space_key(&self, scope: &Scope) -> Result<bool, KeyError> {
        for _ in 0..ATTEMPTS {
            let (revision, mut record) = self.load(scope).await?;
            if !record.rotation_due() {
                return Ok(false);
            }
            self.rotate(scope, &mut record).await?;
            let saved = self
                .storage
                .save_space(scope, revision, record.encode()?)
                .await?;
            self.cache.forget(*scope);
            if saved {
                return Ok(true);
            }
        }
        Err(KeyError::Contended(scope.to_string()))
    }

    /// Re-wraps every surviving data key in `record` under a fresh space-key version, keeping
    /// the half that names blobs, and destroys the old versions.
    async fn rotate(&self, scope: &Scope, record: &mut SpaceRecord) -> Result<(), KeyError> {
        let Some(live) = record.live() else {
            return Ok(());
        };
        let fresh = SpaceKey {
            wrap: Secret::random()?,
            name: self.space_key_at(scope, record, live.version).await?.name,
        };
        let version = record.next_version();
        let org_key = self.org_key(scope.org).await?;
        let mut rewrapped = Vec::with_capacity(record.data.len());
        for entry in &record.data {
            if entry.wrapped.is_none() {
                rewrapped.push(None);
                continue;
            }
            let entry_id = key_id(scope, entry);
            let space = self
                .space_key_at(scope, record, entry.space_version)
                .await?;
            let key = unwrap_data(&space, scope, &entry_id, entry)?;
            rewrapped.push(Some(wrap_data(&fresh, scope, &entry_id, version, &key)?));
        }
        for (entry, wrapped) in record.data.iter_mut().zip(rewrapped) {
            if let Some(wrapped) = wrapped {
                entry.wrapped = Some(wrapped);
                entry.space_version = version;
            }
        }
        for old in &mut record.versions {
            old.wrapped = None;
        }
        record.versions.push(SpaceVersion {
            version,
            wrapped: Some(wrap_space(&org_key, scope, version, &fresh)?),
        });
        Ok(())
    }
}
