//! Finding, making and unwrapping keys: organization key → space key → data key.

use std::fmt::Write as _;

use glave_event::Sha256;
use glave_ports::{OrgId, Scope};
use hmac::{Hmac, KeyInit as _, Mac as _};
use zeroize::Zeroizing;

use crate::error::KeyError;
use crate::key_id::{KeyId, Month, Period};
use crate::record::{DataKey, SpaceRecord, SpaceVersion};
use crate::seal::{associated, open, seal};
use crate::secret::Secret;
use crate::storage::KeyStorage;

use super::cache::{Slot, Value};
use super::{Keys, check_space};

/// How many times a change to a space's keys is retried when another writer got there first.
pub(super) const ATTEMPTS: usize = 8;

/// One version of a space key: one half wraps data keys and is replaced on rotation; the
/// other names blobs and is carried forward, so blob names stay stable until the space is
/// erased.
#[derive(Debug, Clone)]
pub(crate) struct SpaceKey {
    pub(super) wrap: Secret,
    pub(super) name: Secret,
}

impl SpaceKey {
    pub(super) fn fresh() -> Result<Self, KeyError> {
        Ok(Self {
            wrap: Secret::random()?,
            name: Secret::random()?,
        })
    }

    /// HMAC-SHA-256 of the blob's hash under the naming half, in lowercase hex.
    pub(super) fn name_blob(&self, sha256: &Sha256) -> Result<String, KeyError> {
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(self.name.bytes())
            .map_err(|e| KeyError::Corrupt(e.to_string()))?;
        mac.update(sha256.as_str().as_bytes());
        let mut hex = String::with_capacity(64);
        for byte in mac.finalize().into_bytes() {
            let _ = write!(hex, "{byte:02x}");
        }
        Ok(hex)
    }

    /// One word of a full-text index: HMAC-SHA-256 of the word under the naming half, kept
    /// apart from blob names by its own label, its first 16 bytes in lowercase hex. The index
    /// holds no word in the clear, and loses every word when the space is erased.
    pub(super) fn name_term(&self, term: &str) -> Result<String, KeyError> {
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(self.name.bytes())
            .map_err(|e| KeyError::Corrupt(e.to_string()))?;
        mac.update(b"glave/search/v1\0");
        mac.update(term.as_bytes());
        let mut hex = String::with_capacity(32);
        for byte in mac.finalize().into_bytes().iter().take(16) {
            let _ = write!(hex, "{byte:02x}");
        }
        Ok(hex)
    }

    fn to_bytes(&self) -> Zeroizing<Vec<u8>> {
        let mut bytes = Zeroizing::new(Vec::with_capacity(64));
        bytes.extend_from_slice(self.wrap.bytes());
        bytes.extend_from_slice(self.name.bytes());
        bytes
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, KeyError> {
        let (wrap, name) = bytes
            .split_at_checked(32)
            .filter(|(_, name)| name.len() == 32)
            .ok_or_else(|| KeyError::Corrupt("a space key is 64 bytes".into()))?;
        Ok(Self {
            wrap: Secret::from_bytes(wrap)?,
            name: Secret::from_bytes(name)?,
        })
    }
}

fn space_aad(scope: &Scope, version: u32) -> Vec<u8> {
    associated(&[
        b"glave/space-key/v1",
        scope.org.to_string().as_bytes(),
        scope.space.to_string().as_bytes(),
        version.to_string().as_bytes(),
    ])
}

fn data_aad(scope: &Scope, id: &KeyId, space_version: u32) -> Vec<u8> {
    associated(&[
        b"glave/data-key/v1",
        scope.org.to_string().as_bytes(),
        id.to_string().as_bytes(),
        space_version.to_string().as_bytes(),
    ])
}

pub(super) fn wrap_space(
    org_key: &Secret,
    scope: &Scope,
    version: u32,
    key: &SpaceKey,
) -> Result<Vec<u8>, KeyError> {
    seal(org_key, &space_aad(scope, version), &key.to_bytes())
}

pub(super) fn wrap_data(
    space: &SpaceKey,
    scope: &Scope,
    id: &KeyId,
    space_version: u32,
    key: &Secret,
) -> Result<Vec<u8>, KeyError> {
    seal(
        &space.wrap,
        &data_aad(scope, id, space_version),
        key.bytes(),
    )
}

pub(super) fn unwrap_data(
    space: &SpaceKey,
    scope: &Scope,
    id: &KeyId,
    entry: &DataKey,
) -> Result<Secret, KeyError> {
    let wrapped = entry
        .wrapped
        .as_deref()
        .ok_or_else(|| KeyError::Destroyed(id.to_string()))?;
    let aad = data_aad(scope, id, entry.space_version);
    let bytes = Zeroizing::new(open(&space.wrap, &aad, wrapped, &id.to_string())?);
    Secret::from_bytes(&bytes)
}

pub(super) fn key_id(scope: &Scope, entry: &DataKey) -> KeyId {
    KeyId {
        space: scope.space,
        period: entry.period(),
        n: entry.n,
    }
}

impl<S: KeyStorage> Keys<S> {
    /// The organization's key, made and wrapped by the root the first time.
    pub(super) async fn org_key(&self, org: OrgId) -> Result<Secret, KeyError> {
        if let Some(Value::Key(key)) = self.cache.get(&Slot::Org(org)) {
            return Ok(key);
        }
        let wrapped = if let Some(wrapped) = self.storage.org_key(org).await? {
            wrapped
        } else {
            let fresh = Secret::random()?;
            let wrapped = self.root.wrap(org, &fresh).await?;
            self.storage.add_org_key(org, wrapped).await?
        };
        let key = self.root.unwrap(org, &wrapped).await?;
        self.cache.put(Slot::Org(org), Value::Key(key.clone()));
        Ok(key)
    }

    /// The space's record of keys and its revision; an empty record if it has none.
    pub(super) async fn load(&self, scope: &Scope) -> Result<(Option<u64>, SpaceRecord), KeyError> {
        match self.storage.space(scope).await? {
            Some(stored) => Ok((Some(stored.revision), SpaceRecord::decode(&stored.record)?)),
            None => Ok((None, SpaceRecord::default())),
        }
    }

    /// Version `version` of the space key, unwrapped.
    pub(super) async fn space_key_at(
        &self,
        scope: &Scope,
        record: &SpaceRecord,
        version: u32,
    ) -> Result<SpaceKey, KeyError> {
        if let Some(Value::Space(key)) = self.cache.get(&Slot::Space(*scope, version)) {
            return Ok(key);
        }
        let name = format!("v1/{}/space/{version}", scope.space);
        let wrapped = record
            .version(version)
            .ok_or_else(|| KeyError::Unknown(name.clone()))?
            .wrapped
            .as_deref()
            .ok_or_else(|| KeyError::Destroyed(name.clone()))?;
        let org_key = self.org_key(scope.org).await?;
        let bytes = Zeroizing::new(open(&org_key, &space_aad(scope, version), wrapped, &name)?);
        let key = SpaceKey::from_bytes(&bytes)?;
        self.cache
            .put(Slot::Space(*scope, version), Value::Space(key.clone()));
        Ok(key)
    }

    /// The space key in use, or `None` if the space has none.
    pub(super) async fn live_space_key(&self, scope: &Scope) -> Result<Option<SpaceKey>, KeyError> {
        if let Some(Value::Version(version)) = self.cache.get(&Slot::Live(*scope))
            && let Some(Value::Space(key)) = self.cache.get(&Slot::Space(*scope, version))
        {
            return Ok(Some(key));
        }
        let (_, record) = self.load(scope).await?;
        let Some(live) = record.live() else {
            return Ok(None);
        };
        let key = self.space_key_at(scope, &record, live.version).await?;
        self.cache
            .put(Slot::Live(*scope), Value::Version(live.version));
        Ok(Some(key))
    }

    /// The data key that opens content sealed under `id`.
    pub(super) async fn data_key(&self, scope: &Scope, id: KeyId) -> Result<Secret, KeyError> {
        check_space(scope, &id)?;
        if let Some(Value::Key(key)) = self.cache.get(&Slot::Data(*scope, id)) {
            return Ok(key);
        }
        let (_, record) = self.load(scope).await?;
        let entry = record
            .data_key(id.period, id.n)
            .ok_or_else(|| KeyError::Unknown(id.to_string()))?;
        if entry.wrapped.is_none() {
            return Err(KeyError::Destroyed(id.to_string()));
        }
        let space = self
            .space_key_at(scope, &record, entry.space_version)
            .await?;
        let key = unwrap_data(&space, scope, &id, entry)?;
        self.cache
            .put(Slot::Data(*scope, id), Value::Key(key.clone()));
        Ok(key)
    }

    /// The data key that seals `period`'s new content in `scope`, made (with the space key, if
    /// the space has none) the first time.
    pub(super) async fn sealing_key(
        &self,
        scope: &Scope,
        period: Period,
    ) -> Result<(KeyId, Secret), KeyError> {
        if let Some(Value::Id(id)) = self.cache.get(&Slot::Sealing(*scope, period))
            && let Some(Value::Key(key)) = self.cache.get(&Slot::Data(*scope, id))
        {
            return Ok((id, key));
        }
        for _ in 0..ATTEMPTS {
            let (revision, mut record) = self.load(scope).await?;
            let mut changed = false;
            let (version, space) = if let Some(live) = record.live() {
                (
                    live.version,
                    self.space_key_at(scope, &record, live.version).await?,
                )
            } else {
                let version = record.next_version();
                let space = SpaceKey::fresh()?;
                let org_key = self.org_key(scope.org).await?;
                record.versions.push(SpaceVersion {
                    version,
                    wrapped: Some(wrap_space(&org_key, scope, version, &space)?),
                });
                changed = true;
                (version, space)
            };
            let (id, key) = if let Some(entry) = record.sealing(period) {
                let id = key_id(scope, entry);
                (id, unwrap_data(&space, scope, &id, entry)?)
            } else {
                let id = KeyId {
                    space: scope.space,
                    period,
                    n: record.next_n(period),
                };
                let key = Secret::random()?;
                let made = Month::of((self.clock)());
                let (month, memory, provider) = match period {
                    Period::Month(month) => (month, None, None),
                    Period::Memory(memory) => (made, Some(memory), None),
                    Period::Account(account) => (made, None, Some(account)),
                };
                record.data.push(DataKey {
                    month,
                    memory,
                    provider,
                    n: id.n,
                    space_version: version,
                    wrapped: Some(wrap_data(&space, scope, &id, version, &key)?),
                });
                changed = true;
                (id, key)
            };
            if changed
                && !self
                    .storage
                    .save_space(scope, revision, record.encode()?)
                    .await?
            {
                continue;
            }
            self.cache
                .put(Slot::Space(*scope, version), Value::Space(space));
            self.cache.put(Slot::Live(*scope), Value::Version(version));
            self.cache
                .put(Slot::Data(*scope, id), Value::Key(key.clone()));
            self.cache.put(Slot::Sealing(*scope, period), Value::Id(id));
            return Ok((id, key));
        }
        Err(KeyError::Contended(scope.to_string()))
    }
}
