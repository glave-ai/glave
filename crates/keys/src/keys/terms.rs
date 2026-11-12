//! The search index's rows sealed (`specs/storage.md`, "Search index"): each searchable event's
//! words, as keyed hashes, sealed under the data key of its project and month and bound to its
//! event's place, so whoever holds the store sees neither words nor which events share them;
//! and a search of them, through the cache that opens them into memory.

use glave_event::{RawEvent, Seq, ThreadId};
use glave_ports::search::{Found, IndexRow, IndexRows, SearchCache, query_terms};
use glave_ports::{LogError, Scope};

use crate::error::KeyError;
use crate::key_id::{Month, Period};
use crate::material::KeyMaterial;
use crate::seal::{associated, open, seal};
use crate::storage::KeyStorage;

use super::{Keys, Sealed};

/// Bytes of one word's keyed hash in a sealed row.
const TERM: usize = 8;

impl<S: KeyStorage> Keys<S> {
    /// The words `terms` as `scope`'s search index matches them (`glave_ports::search`): each a
    /// keyed hash under the project key's naming half, so the same query finds the same words
    /// until the project is erased, and no other project's words match. `None` when the project
    /// has no key yet (or it was erased): nothing can be found.
    pub async fn search_terms(
        &self,
        scope: &Scope,
        terms: &[String],
    ) -> Result<Option<Vec<u64>>, KeyError> {
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

    /// `event`'s words `terms`, hashed ([`Keys::search_terms`]) and sealed as one row of the
    /// search index under the data key of its project and month, bound to its thread and `seq`.
    /// `None` when there are no words, or the project has no key (it was erased).
    pub async fn seal_terms(
        &self,
        scope: &Scope,
        event: &RawEvent,
        terms: &[String],
    ) -> Result<Option<Sealed>, KeyError> {
        let Some(hashed) = self.search_terms(scope, terms).await? else {
            return Ok(None);
        };
        if hashed.is_empty() {
            return Ok(None);
        }
        let period = Period::Month(Month::of(event.at.timestamp()));
        let (id, key) = self.sealing_key(scope, period).await?;
        let key_id = id.to_string();
        let mut plain = Vec::with_capacity(hashed.len() * TERM);
        for term in &hashed {
            plain.extend_from_slice(&term.to_be_bytes());
        }
        let aad = terms_aad(scope, event.thread, event.seq, &key_id);
        let bytes = seal(&key, &aad, &plain)?;
        Ok(Some(Sealed { key_id, bytes }))
    }

    /// The events of `scope` sharing the most of `query`'s words, then the newest, at most
    /// `limit`, from the index `rows` keeps, through `cache` (`glave_ports::search`).
    pub async fn search(
        &self,
        cache: &SearchCache,
        rows: &impl IndexRows,
        (scope, query): (&Scope, &str),
        limit: usize,
    ) -> Result<Vec<Found>, LogError> {
        let words = query_terms(query);
        if words.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let hashed = self
            .search_terms(scope, &words)
            .await
            .map_err(LogError::sealing)?;
        match hashed {
            Some(hashed) => cache.search(rows, scope, &hashed, limit).await,
            None => Ok(Vec::new()),
        }
    }

    /// Opens the row [`Keys::seal_terms`] sealed for the event `found`, at `pos` in the store's
    /// write order: its words' keyed hashes, or none once the key that sealed it was destroyed,
    /// since that month or project is erased, and its words with it.
    pub async fn open_row(
        &self,
        scope: &Scope,
        (pos, found): (i64, Found),
        key_id: &str,
        sealed: &[u8],
    ) -> Result<IndexRow, LogError> {
        let opening = |error| LogError::opening(found.seq, error);
        let terms = match self.data_key(scope, key_id.parse().map_err(opening)?).await {
            Ok(key) => open_terms(scope, found, (key_id, &key), sealed).map_err(opening)?,
            Err(KeyError::Destroyed(_)) => Vec::new(),
            Err(error) => return Err(opening(error)),
        };
        Ok(IndexRow {
            pos,
            found,
            terms,
            sealed: u64::try_from(sealed.len()).unwrap_or(u64::MAX),
        })
    }
}

/// The keyed hashes in a row sealed for `found` under `key`.
fn open_terms(
    scope: &Scope,
    Found { thread, seq }: Found,
    (key_id, key): (&str, &KeyMaterial),
    sealed: &[u8],
) -> Result<Vec<u64>, KeyError> {
    let plain = open(key, &terms_aad(scope, thread, seq, key_id), sealed, key_id)?;
    let (terms, rest) = plain.as_chunks::<TERM>();
    if !rest.is_empty() {
        return Err(KeyError::Corrupt(format!(
            "a search index row under {key_id} is not whole words"
        )));
    }
    Ok(terms.iter().map(|term| u64::from_be_bytes(*term)).collect())
}

/// What binds a sealed index row to its event: a label of its own, so no event payload's
/// associated data reads alike, then the organization, project, thread, `seq` and `key_id`.
fn terms_aad(scope: &Scope, thread: ThreadId, seq: Seq, key_id: &str) -> Vec<u8> {
    associated(&[
        b"glave/search/v1",
        scope.org().to_string().as_bytes(),
        scope.project().to_string().as_bytes(),
        thread.to_string().as_bytes(),
        seq.get().to_string().as_bytes(),
        key_id.as_bytes(),
    ])
}
