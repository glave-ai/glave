//! The key hierarchy at work: sealing and opening event payloads and blobs, and erasing by
//! destroying keys.

mod blob;
mod cache;
mod erase;
mod hierarchy;

use std::fmt;
use std::sync::Arc;

use glave_event::{RawEvent, Seq, Sha256, ThreadId, sealed_under_account, sealed_under_memory};
use glave_ports::{Clock, Scope, SystemClock};

use crate::error::KeyError;
use crate::key_id::{KeyId, Month, Period};
use crate::root::Root;
use crate::seal::{associated, open, seal};
use crate::storage::KeyStorage;

pub use self::blob::{BlobOpener, BlobSealer, Finished, Opened, seal_into};
use self::cache::Cache;

/// How a store's keys are set up: the root that wraps organization keys, the clock that
/// picks the month a blob is sealed under, and how many unwrapped keys to keep in memory.
#[derive(Clone)]
pub struct KeySettings {
    pub root: Arc<dyn Root>,
    pub clock: Arc<dyn Clock>,
    pub cache_entries: usize,
}

impl KeySettings {
    /// `root`, the system clock, and a cache of 4096 keys.
    pub fn new(root: Arc<dyn Root>) -> Self {
        Self {
            root,
            clock: Arc::new(SystemClock),
            cache_entries: 4096,
        }
    }
}

impl fmt::Debug for KeySettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeySettings")
            .field("cache_entries", &self.cache_entries)
            .finish_non_exhaustive()
    }
}

/// Sealed bytes and the data key that sealed them, as the store keeps them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sealed {
    pub key_id: String,
    pub bytes: Vec<u8>,
}

/// The envelope members a sealed payload is bound to, besides its scope and `key_id`.
#[derive(Debug, Clone, Copy)]
pub struct EventFields<'a> {
    pub glave: u32,
    pub thread: ThreadId,
    pub seq: Seq,
    pub kind: &'a str,
    pub v: u32,
}

impl<'a> From<&'a RawEvent> for EventFields<'a> {
    fn from(event: &'a RawEvent) -> Self {
        Self {
            glave: event.glave,
            thread: event.thread,
            seq: event.seq,
            kind: event.kind.as_str(),
            v: event.v.get(),
        }
    }
}

/// The key hierarchy over one store's wrapped keys.
pub struct Keys<S> {
    root: Arc<dyn Root>,
    clock: Arc<dyn Clock>,
    storage: S,
    cache: Cache,
}

impl<S: KeyStorage> Keys<S> {
    pub fn new(settings: KeySettings, storage: S) -> Self {
        Self {
            root: settings.root,
            clock: settings.clock,
            storage,
            cache: Cache::new(settings.cache_entries),
        }
    }

    pub fn storage(&self) -> &S {
        &self.storage
    }

    /// Seals `event`'s payload under its project's data key for the month it happened in, or,
    /// for an event holding a memory's text or an account (a model account or a git host),
    /// under that item's own key.
    pub async fn seal_event(&self, scope: &Scope, event: &RawEvent) -> Result<Sealed, KeyError> {
        let period = match (sealed_under_memory(event), sealed_under_account(event)) {
            (Some(memory), _) => Period::Memory(memory.as_uuid()),
            (None, Some(account)) => Period::Account(account),
            (None, None) => Period::Month(Month::of(event.at.timestamp())),
        };
        let (id, key) = self.sealing_key(scope, period).await?;
        let key_id = id.to_string();
        let aad = event_aad(scope, &EventFields::from(event), &key_id);
        let bytes = seal(&key, &aad, event.data.get().as_bytes())?;
        Ok(Sealed { key_id, bytes })
    }

    /// Opens a payload [`Keys::seal_event`] sealed, checking it belongs to this row.
    pub async fn open_event(
        &self,
        scope: &Scope,
        fields: &EventFields<'_>,
        key_id: &str,
        sealed: &[u8],
    ) -> Result<Vec<u8>, KeyError> {
        let key = self.data_key(scope, key_id.parse()?).await?;
        open(&key, &event_aad(scope, fields, key_id), sealed, key_id)
    }

    /// The name a blob is stored under in `scope`: HMAC-SHA-256 of its hash under the project
    /// key, so the store does not reveal which known files it holds. `None` when the project
    /// has no key yet (or it was erased), so nothing sealed can be found.
    pub async fn blob_name(
        &self,
        scope: &Scope,
        sha256: &Sha256,
    ) -> Result<Option<String>, KeyError> {
        let Some(project) = self.live_project_key(scope).await? else {
            return Ok(None);
        };
        Ok(Some(project.name_blob(sha256)?))
    }

    /// The words `terms` as `scope`'s full-text index holds them (`glave_ports::search`):
    /// each a keyed hash under the project key's naming half, so the index reveals no word, the
    /// same query finds the same words until the project is erased, and no other project's words
    /// match. `None` when the project has no key yet (or it was erased): nothing can be found.
    pub async fn search_terms(
        &self,
        scope: &Scope,
        terms: &[String],
    ) -> Result<Option<Vec<String>>, KeyError> {
        if terms.is_empty() {
            return Ok(Some(Vec::new()));
        }
        let Some(project) = self.live_project_key(scope).await? else {
            return Ok(None);
        };
        terms
            .iter()
            .map(|term| project.name_term(term))
            .collect::<Result<Vec<_>, _>>()
            .map(Some)
    }
}

/// What a payload reads as once the item key that sealed it, a memory's or a model account's,
/// was destroyed: an empty object. Its envelope outlives it, so a ledger still reads in full
/// after a forget or a removal, and its content is gone (`specs/memory.md`, "Forgetting really
/// forgets"). `None` for any other failure, and for a destroyed month's key, which fails the
/// read as before.
pub fn erased_payload(key_id: &str, error: &KeyError) -> Option<Vec<u8>> {
    let destroyed = matches!(error, KeyError::Destroyed(_));
    let item = key_id
        .parse::<KeyId>()
        .is_ok_and(|id| matches!(id.period, Period::Memory(_) | Period::Account(_)));
    (destroyed && item).then(|| b"{}".to_vec())
}

/// What binds a sealed payload to its row: the envelope version, organization, project,
/// thread, `seq`, `type`, `v` and `key_id`, in that order (`specs/storage.md`).
fn event_aad(scope: &Scope, fields: &EventFields<'_>, key_id: &str) -> Vec<u8> {
    associated(&[
        fields.glave.to_string().as_bytes(),
        scope.org().to_string().as_bytes(),
        scope.project().to_string().as_bytes(),
        fields.thread.to_string().as_bytes(),
        fields.seq.get().to_string().as_bytes(),
        fields.kind.as_bytes(),
        fields.v.to_string().as_bytes(),
        key_id.as_bytes(),
    ])
}

/// A data key named for another project never opens content in this one.
pub(crate) fn check_project(scope: &Scope, id: &KeyId) -> Result<(), KeyError> {
    if id.project == scope.project() {
        Ok(())
    } else {
        Err(KeyError::Tampered(id.to_string()))
    }
}

#[cfg(test)]
mod tests;
