//! The event log port: append-only, gap-free per thread, read in pages.

use std::future::Future;

use glave_event::{RawEvent, Seq, ThreadId};
use thiserror::Error;

use crate::scope::{Located, Scope};
use crate::search::Found;
use crate::view::{ThreadQuery, ThreadView};

/// The append-only log every event is written to.
///
/// Implementations must store each append all-or-nothing and enforce, in the same
/// transaction as the write, that a thread's events are numbered 1, 2, 3 with no gaps and no
/// duplicates. That check is what makes a second writer fail instead of scrambling a thread.
pub trait EventLog: Send + Sync {
    /// Appends events to one thread. All of `events` must belong to the same thread and
    /// continue its numbering directly after its current last event (or start at 1).
    fn append(
        &self,
        scope: &Scope,
        events: &[RawEvent],
    ) -> impl Future<Output = Result<(), LogError>> + Send;

    /// Reads up to `limit` events of a thread in order, starting after `after` (or from the
    /// beginning). A thread that does not exist in `scope` reads as empty.
    fn read(
        &self,
        scope: &Scope,
        thread: ThreadId,
        after: Option<Seq>,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<RawEvent>, LogError>> + Send;

    /// Searches `scope`'s conversations by keyword, with the store's own full-text index: the
    /// events whose words (`search::indexed`, maintained as events are appended) match any of
    /// `query`'s (`search::query_terms`), best match first, then newest first, at most `limit`.
    /// Never another scope's.
    fn search(
        &self,
        scope: &Scope,
        query: &str,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<Found>, LogError>> + Send;

    /// The scope `thread` is stored in, when any event of it is: the one lookup that takes a
    /// bare thread id (`specs/storage.md`, "Scope"). It answers only "which scope"; every read
    /// that follows is scoped as usual. The ledgers' reserved ids are in every project, so it is
    /// asked only of threads.
    fn locate(
        &self,
        thread: ThreadId,
    ) -> impl Future<Output = Result<Option<Located>, LogError>> + Send;

    /// A page of the threads view in `scope` (`specs/events.md`, "Storage"): the views of one
    /// standing, most recently updated first, after `query.after`, at most `query.limit`. Its
    /// cost is the page's, not the log's.
    fn thread_views(
        &self,
        scope: &Scope,
        query: &ThreadQuery,
    ) -> impl Future<Output = Result<Vec<ThreadView>, LogError>> + Send;

    /// The view of one thread, if it has one: every thread does from its first append.
    fn thread_view(
        &self,
        scope: &Scope,
        thread: ThreadId,
    ) -> impl Future<Output = Result<Option<ThreadView>, LogError>> + Send;

    /// Folds the view of `thread` again from every event of it, replaces the stored one, and
    /// returns it. The views are disposable: a rebuilt view equals the one kept by appends.
    fn rebuild_thread_view(
        &self,
        scope: &Scope,
        thread: ThreadId,
    ) -> impl Future<Output = Result<Option<ThreadView>, LogError>> + Send;

    /// The thread's latest event whose type is one of `kinds`, if it has one. The default reads
    /// the thread from the start; an adapter that can ask its storage answers faster.
    fn latest_of(
        &self,
        scope: &Scope,
        thread: ThreadId,
        kinds: &[&str],
    ) -> impl Future<Output = Result<Option<RawEvent>, LogError>> + Send {
        async move {
            let mut latest = None;
            let mut after: Option<Seq> = None;
            loop {
                let page = self.read(scope, thread, after, LATEST_PAGE).await?;
                let full = page.len() == LATEST_PAGE;
                after = page.last().map(|event| event.seq).or(after);
                for event in page {
                    if kinds.contains(&event.kind.as_str()) {
                        latest = Some(event);
                    }
                }
                if !full {
                    return Ok(latest);
                }
            }
        }
    }
}

/// Events the default `latest_of` reads per page.
const LATEST_PAGE: usize = 1_000;

/// Why an append or a read failed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LogError {
    /// Another writer appended first: the thread already has an event at this position.
    #[error("thread {thread} already has event {found}; expected to write {expected}")]
    Conflict {
        thread: ThreadId,
        expected: u64,
        found: u64,
    },
    /// The events would leave a hole in the thread's numbering.
    #[error("thread {thread} expects event {expected} next, got {found}")]
    Gap {
        thread: ThreadId,
        expected: u64,
        found: u64,
    },
    /// One append mixed events from different threads.
    #[error("an append must target one thread; got {first} and {other}")]
    MixedThreads { first: ThreadId, other: ThreadId },
    /// The storage behind the log failed.
    #[error("event storage failed: {0}")]
    Storage(String),
}

/// The storage failures every store reports alike.
impl LogError {
    /// A stored event that does not read back as one.
    pub fn corrupt(error: impl std::fmt::Display) -> Self {
        Self::Storage(format!("a stored event is corrupt: {error}"))
    }

    /// An event that could not be sealed before it was stored.
    pub fn sealing(error: impl std::fmt::Display) -> Self {
        Self::Storage(format!("could not seal an event: {error}"))
    }

    /// The stored event `seq` that could not be opened.
    pub fn opening(seq: Seq, error: impl std::fmt::Display) -> Self {
        Self::Storage(format!("event {} could not be opened: {error}", seq.get()))
    }
}

/// Checks that `events` all belong to one thread and continue its numbering after `last`.
/// Every implementation applies this rule; it lives here so they apply it identically.
pub fn check_append(last: Option<Seq>, events: &[RawEvent]) -> Result<(), LogError> {
    let Some(first) = events.first() else {
        return Ok(());
    };
    let mut expected = last.map_or(Some(1), |seq| seq.get().checked_add(1));
    for event in events {
        if event.thread != first.thread {
            return Err(LogError::MixedThreads {
                first: first.thread,
                other: event.thread,
            });
        }
        let found = event.seq.get();
        let Some(want) = expected else {
            return Err(LogError::Storage(format!(
                "thread {} is full",
                first.thread
            )));
        };
        if found < want {
            return Err(LogError::Conflict {
                thread: first.thread,
                expected: want,
                found,
            });
        }
        if found > want {
            return Err(LogError::Gap {
                thread: first.thread,
                expected: want,
                found,
            });
        }
        expected = want.checked_add(1);
    }
    Ok(())
}
