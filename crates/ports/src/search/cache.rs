//! The search index held in memory (`specs/storage.md`, "Search index"). A store keeps each
//! searchable event's words sealed, so it cannot match them itself: a search opens a project's
//! rows once, newest first up to [`MOST_LOADED`], into an inverted index kept in a
//! [`SearchCache`], and later searches read only the rows written since. A row gone (a deleted
//! chat) or one committed out of order changes the count or sum of the rows' positions, which
//! the index checks before each search, loading again when they differ.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::future::Future;
use std::sync::Mutex;

use super::Found;
use crate::log::LogError;
use crate::scope::Scope;

const MIB: u64 = 1024 * 1024;
/// Sealed bytes of a project's newest rows one load opens, at most: about 3 million words, or
/// 20,000 to 40,000 messages and outputs. Older ones are not found until newer ones go.
pub const MOST_LOADED: u64 = 24 * MIB;
/// Sealed bytes a project's index holds before it is loaded again, newest first.
const MOST_HELD: u64 = 32 * MIB;
/// Bytes of memory every project's index in one store's cache takes together, at most.
pub const MOST_CACHED: usize = 64 * 1024 * 1024;

/// How many rows a project's index has, and the sum of their positions: what tells a search
/// whether rows went, or came in out of order, since it last read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Fingerprint {
    pub rows: u64,
    pub sum: i64,
}

/// One row of a project's index, opened: the event it is for, its words' keyed hashes (none
/// once its key was destroyed), its position in the store's write order, and its sealed size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexRow {
    pub pos: i64,
    pub found: Found,
    pub terms: Vec<u64>,
    pub sealed: u64,
}

/// How a store reads its sealed index for a [`SearchCache`].
pub trait IndexRows: Sync {
    /// `scope`'s newest rows after position `after` whose sealed bytes come to at most `most`,
    /// opened, oldest first, and the fingerprint of all its rows, read in one snapshot.
    fn rows(
        &self,
        scope: &Scope,
        after: i64,
        most: u64,
    ) -> impl Future<Output = Result<(Fingerprint, Vec<IndexRow>), LogError>> + Send;
}

/// One project's index in memory: its events in write order, and each word's events.
#[derive(Debug, Default)]
struct Index {
    found: Vec<Found>,
    postings: HashMap<u64, Vec<u32>>,
    words: usize,
    held: u64,
    last: i64,
    seen: Fingerprint,
}

impl Index {
    fn loaded(seen: Fingerprint, rows: Vec<IndexRow>) -> Self {
        let mut index = Self {
            seen,
            ..Self::default()
        };
        index.add(rows);
        index
    }

    fn add(&mut self, rows: Vec<IndexRow>) {
        for row in rows {
            let Ok(at) = u32::try_from(self.found.len()) else {
                return;
            };
            self.found.push(row.found);
            self.last = self.last.max(row.pos);
            self.held = self.held.saturating_add(row.sealed);
            self.words += row.terms.len();
            for term in row.terms {
                self.postings.entry(term).or_default().push(at);
            }
        }
    }

    /// Adds `rows`, read with `now`, when they are all that changed since the last read and the
    /// index stays within [`MOST_HELD`]; `false` when it must be loaded again.
    fn caught_up(&mut self, now: Fingerprint, rows: Vec<IndexRow>) -> bool {
        let added = u64::try_from(rows.len()).unwrap_or(u64::MAX);
        let sum = rows
            .iter()
            .fold(self.seen.sum, |sum, row| sum.wrapping_add(row.pos));
        if self.seen.rows.saturating_add(added) != now.rows || sum != now.sum {
            return false;
        }
        self.add(rows);
        self.seen = now;
        self.held <= MOST_HELD
    }

    /// The events sharing the most of `terms`, then the newest, at most `limit`.
    fn find(&self, terms: &[u64], limit: usize) -> Vec<Found> {
        let mut shared: BTreeMap<u32, u32> = BTreeMap::new();
        for term in terms.iter().collect::<BTreeSet<_>>() {
            for at in self.postings.get(term).into_iter().flatten() {
                *shared.entry(*at).or_default() += 1;
            }
        }
        let mut ranked: Vec<(u32, u32)> = shared.into_iter().map(|(at, n)| (n, at)).collect();
        ranked.sort_unstable_by(|a, b| b.cmp(a));
        ranked
            .into_iter()
            .take(limit)
            .filter_map(|(_, at)| self.found.get(usize::try_from(at).ok()?).copied())
            .collect()
    }

    /// The memory it takes, near enough: each event, each word's place, each distinct word.
    fn bytes(&self) -> usize {
        let word = size_of::<u64>() + size_of::<Vec<u32>>() + size_of::<usize>();
        size_of::<Self>()
            + self.found.len() * size_of::<Found>()
            + self.words * size_of::<u32>()
            + self.postings.len() * word
    }
}

/// The projects' indexes one store keeps in memory, the least recently searched let go first
/// past its size, so its memory is bounded however many projects are searched.
#[derive(Debug)]
pub struct SearchCache {
    held: Mutex<Vec<(Scope, Index)>>,
    most: usize,
}

impl SearchCache {
    /// A cache of at most `most` bytes.
    pub fn new(most: usize) -> Self {
        Self {
            held: Mutex::new(Vec::new()),
            most,
        }
    }

    /// The memory its indexes take now.
    pub fn bytes(&self) -> usize {
        self.held
            .lock()
            .map_or(0, |held| held.iter().map(|(_, index)| index.bytes()).sum())
    }

    /// The events of `scope` sharing the most of `terms` (keyed hashes, as `rows` holds them),
    /// then the newest, at most `limit`, reading only what changed since the last search.
    pub async fn search(
        &self,
        rows: &impl IndexRows,
        scope: &Scope,
        terms: &[u64],
        limit: usize,
    ) -> Result<Vec<Found>, LogError> {
        let index = match self.take(scope)? {
            Some(mut index) => {
                let (now, added) = rows.rows(scope, index.last, u64::MAX).await?;
                if index.caught_up(now, added) {
                    index
                } else {
                    load(rows, scope).await?
                }
            }
            None => load(rows, scope).await?,
        };
        let found = index.find(terms, limit);
        self.keep(*scope, index)?;
        Ok(found)
    }

    /// `scope`'s index, out of the cache while it is brought up to date: a search beside it
    /// loads its own, and the last one kept stays.
    fn take(&self, scope: &Scope) -> Result<Option<Index>, LogError> {
        let mut held = self.held.lock().map_err(|_| poisoned())?;
        let at = held.iter().position(|(kept, _)| kept == scope);
        Ok(at.map(|at| held.remove(at).1))
    }

    fn keep(&self, scope: Scope, index: Index) -> Result<(), LogError> {
        let mut held = self.held.lock().map_err(|_| poisoned())?;
        held.retain(|(kept, _)| *kept != scope);
        held.push((scope, index));
        let mut total: usize = held.iter().map(|(_, index)| index.bytes()).sum();
        while total > self.most && !held.is_empty() {
            let (_, gone) = held.remove(0);
            total -= gone.bytes();
        }
        Ok(())
    }
}

async fn load(rows: &impl IndexRows, scope: &Scope) -> Result<Index, LogError> {
    let (now, newest) = rows.rows(scope, 0, MOST_LOADED).await?;
    Ok(Index::loaded(now, newest))
}

fn poisoned() -> LogError {
    LogError::Storage("the search cache's lock is poisoned".into())
}

#[cfg(test)]
mod tests;
