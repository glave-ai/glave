//! Erasure: destroying a space key, or a data key followed by rotating the space key.

use glave_ports::Scope;

use crate::error::KeyError;
use crate::key_id::KeyId;
use crate::record::SpaceVersion;
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
            if record.data_key(id.month, id.n).is_none() {
                return Err(KeyError::Unknown(id.to_string()));
            }
            // With no space key left, everything is already unreadable; there is nothing to
            // rotate.
            let fresh = match record.live() {
                Some(live) => SpaceKey {
                    wrap: Secret::random()?,
                    name: self.space_key_at(scope, &record, live.version).await?.name,
                },
                None => return Ok(()),
            };
            let version = record.next_version();
            let org_key = self.org_key(scope.org).await?;
            let mut survivors = Vec::new();
            for entry in &record.data {
                let entry_id = key_id(scope, entry);
                if entry.wrapped.is_none() || entry_id == id {
                    continue;
                }
                let space = self
                    .space_key_at(scope, &record, entry.space_version)
                    .await?;
                let key = unwrap_data(&space, scope, &entry_id, entry)?;
                survivors.push((
                    entry_id,
                    wrap_data(&fresh, scope, &entry_id, version, &key)?,
                ));
            }
            for entry in &mut record.data {
                let entry_id = key_id(scope, entry);
                if entry_id == id {
                    entry.wrapped = None;
                } else if let Some((_, wrapped)) = survivors.iter().find(|(s, _)| *s == entry_id) {
                    entry.wrapped = Some(wrapped.clone());
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
}
