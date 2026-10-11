//! The event log port: append-only, gap-free per thread, read in pages.

use std::future::Future;

use glave_event::{RawEvent, Seq, ThreadId};
use thiserror::Error;

use crate::scope::Scope;

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

    /// The threads in `scope`, most recently written first, at most `limit` of them.
    fn threads(
        &self,
        scope: &Scope,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<ThreadId>, LogError>> + Send;

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
