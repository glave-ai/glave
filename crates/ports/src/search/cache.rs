//! The search index held in memory (`specs/storage.md`, "Search index"). A store keeps each
//! searchable event's words sealed, so it cannot match them itself: a search opens a project's
//! rows once, newest first up to [`MOST_LOADED`], into an inverted index kept in a
//! [`SearchCache`], and later searches read only the rows written since, as much again at most.
//! A row gone (a deleted chat), one committed out of order, or rows past what one read takes
//! change the count or the sums of the rows' positions, which the index checks before each
//! search, loading again when they differ.

use glave_event::ThreadId;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::future::Future;
use std::sync::Mutex;

use super::{Found, weight};
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

/// The modulus of [`Fingerprint::squares`], small enough that a square of a position below it
/// fits in 63 bits, as SQL computes it.
pub const SQUARES_MODULUS: i64 = 1_000_000_007;

/// How many rows a project's index has, the sum of their positions, and the sum of their
/// squares modulo [`SQUARES_MODULUS`]: what tells a search whether rows went, or came in out of
/// order, since it last read. The second sum is not linear, so rows gone and rows come late that
/// keep the count and the first sum equal still change it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Fingerprint {
    pub rows: u64,
    pub sum: i64,
    pub squares: i64,
}

impl Fingerprint {
    /// This fingerprint with a row at `pos` added.
    fn with(self, pos: i64) -> Self {
        let root = pos.rem_euclid(SQUARES_MODULUS);
        Self {
            rows: self.rows.saturating_add(1),
            sum: self.sum.wrapping_add(pos),
            squares: self
                .squares
                .wrapping_add(root.wrapping_mul(root).rem_euclid(SQUARES_MODULUS)),
        }
    }
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
    bytes: usize,
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
        index.found.shrink_to_fit();
        index.postings.shrink_to_fit();
        index.postings.values_mut().for_each(Vec::shrink_to_fit);
        index.measure();
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
            for term in row.terms {
                self.postings.entry(term).or_default().push(at);
            }
        }
    }

    /// Adds `rows`, read with `now`, when they are all that changed since the last read and the
    /// index stays within [`MOST_HELD`]; `false` when it must be loaded again.
    fn caught_up(&mut self, now: Fingerprint, rows: Vec<IndexRow>) -> bool {
        let read = rows.iter().fold(self.seen, |seen, row| seen.with(row.pos));
        if read != now {
            return false;
        }
        if !rows.is_empty() {
            self.add(rows);
            self.measure();
        }
        self.seen = now;
        self.held <= MOST_HELD
    }

    /// The events best matching `terms`, none of the thread `except`, then the newest, at most
    /// `limit`: each word weighs as rare as it is among the index's events ([`weight`]).
    fn find(&self, terms: &[u64], except: Option<ThreadId>, limit: usize) -> Vec<Found> {
        let mut scores: BTreeMap<u32, f64> = BTreeMap::new();
        for term in terms.iter().collect::<BTreeSet<_>>() {
            let events = self.postings.get(term).map_or(&[][..], Vec::as_slice);
            let weight = weight(self.found.len(), events.len());
            for at in events {
                *scores.entry(*at).or_default() += weight;
            }
        }
        let mut ranked: Vec<(f64, u32, Found)> = scores
            .into_iter()
            .filter_map(|(at, score)| {
                let found = *self.found.get(usize::try_from(at).ok()?)?;
                (Some(found.thread) != except).then_some((score, at, found))
            })
            .collect();
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.cmp(&a.1)));
        ranked
            .into_iter()
            .take(limit)
            .map(|(_, _, found)| found)
            .collect()
    }

    /// Takes the memory it holds now: what each list has room for, and the map's slots.
    fn measure(&mut self) {
        let slot = size_of::<u64>() + size_of::<Vec<u32>>() + 1;
        let lists: usize = self.postings.values().map(Vec::capacity).sum();
        self.bytes = size_of::<Self>()
            + self.found.capacity() * size_of::<Found>()
            + lists * size_of::<u32>()
            + self.postings.capacity() * slot;
    }

    fn bytes(&self) -> usize {
        self.bytes
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

    /// The events of `scope` best matching `terms` (keyed hashes, as `rows` holds them), none of
    /// the thread `except`, then the newest, at most `limit`, reading only what changed since
    /// the last search: at most [`MOST_LOADED`], past which the index is loaded again.
    pub async fn search(
        &self,
        rows: &impl IndexRows,
        scope: &Scope,
        (terms, except): (&[u64], Option<ThreadId>),
        limit: usize,
    ) -> Result<Vec<Found>, LogError> {
        let index = match self.take(scope)? {
            Some(mut index) => {
                let (now, added) = rows.rows(scope, index.last, MOST_LOADED).await?;
                if index.caught_up(now, added) {
                    index
                } else {
                    load(rows, scope).await?
                }
            }
            None => load(rows, scope).await?,
        };
        let found = index.find(terms, except, limit);
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
