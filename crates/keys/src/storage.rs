//! Where wrapped keys are kept: the data store, through this interface. It holds only wrapped
//! keys, never a key that could open content on its own.

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Mutex;

use glave_ports::{OrgId, Scope};

use crate::error::KeyError;

/// A space's wrapped keys as stored: one record, replaced whole, and its revision, which
/// increases by one on every save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSpace {
    pub revision: u64,
    pub record: Vec<u8>,
}

/// Keeps wrapped keys for [`crate::Keys`]. Shaped as if across a network: a space's keys are
/// read and written as one record, and every write is a compare-and-swap on its revision, so
/// rotations are atomic and concurrent writers cannot lose each other's keys.
pub trait KeyStorage: Send + Sync {
    /// The wrapped key of `org`, if it has one.
    fn org_key(&self, org: OrgId)
    -> impl Future<Output = Result<Option<Vec<u8>>, KeyError>> + Send;

    /// Keeps `wrapped` as the key of `org` unless it already has one, and returns the key
    /// that is kept, so two first writers agree on one key.
    fn add_org_key(
        &self,
        org: OrgId,
        wrapped: Vec<u8>,
    ) -> impl Future<Output = Result<Vec<u8>, KeyError>> + Send;

    /// The record of `scope`'s keys, if it has any.
    fn space(
        &self,
        scope: &Scope,
    ) -> impl Future<Output = Result<Option<StoredSpace>, KeyError>> + Send;

    /// Saves `record` for `scope` if its revision is still `expected` (`None`: the space has
    /// no record yet). Returns whether it was saved; a saved record's revision is one more
    /// than `expected` (or 1).
    fn save_space(
        &self,
        scope: &Scope,
        expected: Option<u64>,
        record: Vec<u8>,
    ) -> impl Future<Output = Result<bool, KeyError>> + Send;
}

/// Wrapped keys held in memory, for tests and in-memory stores.
#[derive(Debug, Default)]
pub struct MemoryKeyStorage {
    orgs: Mutex<BTreeMap<OrgId, Vec<u8>>>,
    spaces: Mutex<BTreeMap<Scope, StoredSpace>>,
}

impl MemoryKeyStorage {
    pub fn new() -> Self {
        Self::default()
    }

    fn poisoned<T>(_: T) -> KeyError {
        KeyError::Storage("the in-memory key storage's lock is poisoned".into())
    }

    fn add_org_now(&self, org: OrgId, wrapped: Vec<u8>) -> Result<Vec<u8>, KeyError> {
        let mut orgs = self.orgs.lock().map_err(Self::poisoned)?;
        Ok(orgs.entry(org).or_insert(wrapped).clone())
    }

    fn save_now(
        &self,
        scope: &Scope,
        expected: Option<u64>,
        record: Vec<u8>,
    ) -> Result<bool, KeyError> {
        let mut spaces = self.spaces.lock().map_err(Self::poisoned)?;
        let found = spaces.get(scope).map(|stored| stored.revision);
        if found != expected {
            return Ok(false);
        }
        let revision = expected.map_or(Some(1), |r| r.checked_add(1));
        let revision = revision.ok_or_else(|| KeyError::Storage("revision overflow".into()))?;
        spaces.insert(*scope, StoredSpace { revision, record });
        Ok(true)
    }
}

impl KeyStorage for MemoryKeyStorage {
    fn org_key(
        &self,
        org: OrgId,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, KeyError>> + Send {
        ready(
            self.orgs
                .lock()
                .map_err(Self::poisoned)
                .map(|orgs| orgs.get(&org).cloned()),
        )
    }

    fn add_org_key(
        &self,
        org: OrgId,
        wrapped: Vec<u8>,
    ) -> impl Future<Output = Result<Vec<u8>, KeyError>> + Send {
        ready(self.add_org_now(org, wrapped))
    }

    fn space(
        &self,
        scope: &Scope,
    ) -> impl Future<Output = Result<Option<StoredSpace>, KeyError>> + Send {
        ready(
            self.spaces
                .lock()
                .map_err(Self::poisoned)
                .map(|spaces| spaces.get(scope).cloned()),
        )
    }

    fn save_space(
        &self,
        scope: &Scope,
        expected: Option<u64>,
        record: Vec<u8>,
    ) -> impl Future<Output = Result<bool, KeyError>> + Send {
        ready(self.save_now(scope, expected, record))
    }
}
