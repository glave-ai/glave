//! Erasure: destroying a project key, or a data key followed by rotating the project key.

use glave_ports::{BoxFuture, Erasable, EraseError, Eraser, Scope};

use crate::error::KeyError;
use crate::key_id::{KeyId, Period};
use crate::material::KeyMaterial;
use crate::record::{ProjectRecord, ProjectVersion};
use crate::storage::KeyStorage;

use super::hierarchy::{ATTEMPTS, ProjectKey, key_id, unwrap_data, wrap_data, wrap_project};
use super::{Keys, check_project};

/// A memory's keys, or an account's, destroyed at once: every version of it is unreadable in the
/// store from now on, and backups lose it when the project key rotates (`specs/memory.md`:
/// within a day, batching the day's forgets).
impl<S: KeyStorage> Eraser for Keys<S> {
    fn destroy(&self, scope: Scope, item: Erasable) -> BoxFuture<'_, Result<(), EraseError>> {
        let period = match item {
            Erasable::Memory(memory) => Period::Memory(memory.as_uuid()),
            Erasable::Account(account) => Period::Account(account),
        };
        Box::pin(async move {
            self.destroy_own_key(&scope, period)
                .await
                .map_err(|error| EraseError(error.to_string()))
        })
    }

    fn rotate(&self, scope: Scope) -> BoxFuture<'_, Result<bool, EraseError>> {
        Box::pin(async move {
            self.rotate_project_key(&scope)
                .await
                .map_err(|error| EraseError(error.to_string()))
        })
    }
}

impl<S: KeyStorage> Keys<S> {
    /// Erases a project: destroys every version of its project key, and with them every data key
    /// they wrapped. Everything sealed in the project becomes unreadable, in the store and in any
    /// backup. New content in the project gets new keys.
    pub async fn erase_project(&self, scope: &Scope) -> Result<(), KeyError> {
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
                .save_project(scope, revision, record.encode()?)
                .await?;
            self.cache.forget(*scope);
            if saved {
                return Ok(());
            }
        }
        Err(KeyError::Contended(scope.to_string()))
    }

    /// Erases what one data key sealed: destroys it, then rotates the project key so backups
    /// holding the old wrapped data key can no longer unwrap it. Every surviving data key is
    /// re-wrapped under a new project-key version and the old versions are destroyed, all in
    /// one save.
    pub async fn destroy_data_key(&self, scope: &Scope, id: KeyId) -> Result<(), KeyError> {
        check_project(scope, &id)?;
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
            // With no project key left, everything is already unreadable; there is nothing to
            // rotate.
            if record.live().is_none() {
                return Ok(());
            }
            self.rotate(scope, &mut record).await?;
            let saved = self
                .storage
                .save_project(scope, revision, record.encode()?)
                .await?;
            self.cache.forget(*scope);
            if saved {
                return Ok(());
            }
        }
        Err(KeyError::Contended(scope.to_string()))
    }

    /// Destroys every key of the item `period` names.
    async fn destroy_own_key(&self, scope: &Scope, period: Period) -> Result<(), KeyError> {
        for _ in 0..ATTEMPTS {
            let (revision, mut record) = self.load(scope).await?;
            let mut changed = false;
            for entry in &mut record.data {
                if entry.period() == period && entry.wrapped.is_some() {
                    entry.wrapped = None;
                    changed = true;
                }
            }
            if !changed {
                // Destroyed already, perhaps by another process: forget any copy held here.
                self.cache.forget(*scope);
                return Ok(());
            }
            let saved = self
                .storage
                .save_project(scope, revision, record.encode()?)
                .await?;
            self.cache.forget(*scope);
            if saved {
                return Ok(());
            }
        }
        Err(KeyError::Contended(scope.to_string()))
    }

    /// Whether a data key was destroyed since the project key was last rotated, so backups can
    /// still unwrap it.
    pub async fn rotation_due(&self, scope: &Scope) -> Result<bool, KeyError> {
        let (_, record) = self.load(scope).await?;
        Ok(record.rotation_due())
    }

    /// Rotates the project key when a data key was destroyed under the one in use: the surviving
    /// data keys are re-wrapped under a new version and the old versions destroyed, so
    /// backups lose every destroyed key. Returns whether it rotated.
    pub async fn rotate_project_key(&self, scope: &Scope) -> Result<bool, KeyError> {
        for _ in 0..ATTEMPTS {
            let (revision, mut record) = self.load(scope).await?;
            if !record.rotation_due() {
                return Ok(false);
            }
            self.rotate(scope, &mut record).await?;
            let saved = self
                .storage
                .save_project(scope, revision, record.encode()?)
                .await?;
            self.cache.forget(*scope);
            if saved {
                return Ok(true);
            }
        }
        Err(KeyError::Contended(scope.to_string()))
    }

    /// Re-wraps every surviving data key in `record` under a fresh project-key version, keeping
    /// the half that names blobs, and destroys the old versions.
    async fn rotate(&self, scope: &Scope, record: &mut ProjectRecord) -> Result<(), KeyError> {
        let Some(live) = record.live() else {
            return Ok(());
        };
        let fresh = ProjectKey {
            wrap: KeyMaterial::random()?,
            name: self.project_key_at(scope, record, live.version).await?.name,
        };
        let version = record.next_version();
        let org_key = self.org_key(scope.org()).await?;
        let mut rewrapped = Vec::with_capacity(record.data.len());
        for entry in &record.data {
            if entry.wrapped.is_none() {
                rewrapped.push(None);
                continue;
            }
            let entry_id = key_id(scope, entry);
            let project = self
                .project_key_at(scope, record, entry.project_version)
                .await?;
            let key = unwrap_data(&project, scope, &entry_id, entry)?;
            rewrapped.push(Some(wrap_data(&fresh, scope, &entry_id, version, &key)?));
        }
        for (entry, wrapped) in record.data.iter_mut().zip(rewrapped) {
            if let Some(wrapped) = wrapped {
                entry.wrapped = Some(wrapped);
                entry.project_version = version;
            }
        }
        for old in &mut record.versions {
            old.wrapped = None;
        }
        record.versions.push(ProjectVersion {
            version,
            wrapped: Some(wrap_project(&org_key, scope, version, &fresh)?),
        });
        Ok(())
    }
}
