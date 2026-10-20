//! glave's own streams in the event log, beside the threads: its ledgers, each under a reserved
//! id in every project, and the gateway's stream of each person. A stream is read by folding
//! its events in order, and written by any number of glave processes at once: an append that
//! meets another writer's `(thread, seq)` reads what was written and tries again after it.

use std::future::Future;

use glave_event::{RawEvent, Seq, ThreadId};
use thiserror::Error;

use crate::log::{EventLog, LogError};
use crate::scope::Scope;

/// Events read per page.
pub const PAGE: usize = 500;

/// Appends tried before giving up on a stream other writers keep moving.
pub const ATTEMPTS: usize = 8;

/// glave's ledgers, each under its reserved id ([`ThreadId::reserved`]), numbered by the id's
/// last byte. None is ever a conversation: lists and reads of threads leave them out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reserved {
    /// `…0000`: a project's tasks (`task.started`, `task.ended`) and schedules
    /// (`schedule.created`, `schedule.deleted`).
    Tasks,
    /// `…0001`: a project's memories, and in a personal project, the person's
    /// (`specs/memory.md`).
    Memory,
    /// `…0002`: the model accounts (`provider.connected`, `provider.removed`) and git hosts
    /// (`credential.connected`, `credential.removed`) a person connected, in their personal
    /// project.
    Accounts,
    /// `…0003`: a project's own settings, today its folder (`project.folder.set`).
    Project,
    /// `…0004`: what a person decided about the projects that ask to be trusted
    /// (`trust.decided`), in their personal project.
    Trust,
    /// `…0005`: the browser sessions of the person on their own server, in their personal
    /// project.
    Browsers,
    /// `…0006`: the catalogs of the services whose model accounts a person connected
    /// (`catalog.refreshed`), in their personal project.
    Catalogs,
    /// `…0007`: what the person's own server serves HTTPS with (`acme.registered`,
    /// `certificate.issued`), in their personal project.
    Certificates,
}

impl Reserved {
    /// Every ledger, by number.
    pub const ALL: [Self; 8] = [
        Self::Tasks,
        Self::Memory,
        Self::Accounts,
        Self::Project,
        Self::Trust,
        Self::Browsers,
        Self::Catalogs,
        Self::Certificates,
    ];

    /// The ledger's id, the same in every project.
    pub fn id(self) -> ThreadId {
        let number = match self {
            Self::Tasks => 0,
            Self::Memory => 1,
            Self::Accounts => 2,
            Self::Project => 3,
            Self::Trust => 4,
            Self::Browsers => 5,
            Self::Catalogs => 6,
            Self::Certificates => 7,
        };
        ThreadId::reserved(number)
    }

    /// The ledger `thread` is, if it is one rather than a thread.
    pub fn of(thread: ThreadId) -> Option<Self> {
        Self::ALL.into_iter().find(|ledger| ledger.id() == thread)
    }
}

/// Why a stream could not be read or written.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StreamError {
    /// The log failed.
    #[error(transparent)]
    Log(#[from] LogError),
    /// Other writers appended first every time.
    #[error("glave's records kept changing while it wrote to them; try again")]
    Busy { thread: ThreadId },
}

/// Reads `thread`'s events after `after` (from its start when `None`), a page at a time, and
/// hands each page to `take` in order, until the stream's end.
pub async fn catch_up<L: EventLog>(
    log: &L,
    scope: &Scope,
    thread: ThreadId,
    mut after: Option<Seq>,
    mut take: impl FnMut(Vec<RawEvent>),
) -> Result<(), StreamError> {
    loop {
        let page = log.read(scope, thread, after, PAGE).await?;
        let full = page.len() == PAGE;
        after = page.last().map(|event| event.seq).or(after);
        take(page);
        if !full {
            return Ok(());
        }
    }
}

/// Appends to `thread` the events `attempt` makes, with what it made: `attempt` reads the
/// stream as it stands and numbers its events after it, or makes nothing, and nothing is
/// written. When another writer appended first, it is asked again, up to [`ATTEMPTS`] times.
/// The events appended are returned, for a view kept in memory to take them in.
pub async fn append_retrying<L, T, E, F>(
    log: &L,
    scope: &Scope,
    thread: ThreadId,
    mut attempt: impl FnMut() -> F,
) -> Result<Option<(Vec<RawEvent>, T)>, E>
where
    L: EventLog,
    E: From<StreamError>,
    F: Future<Output = Result<Option<(Vec<RawEvent>, T)>, E>>,
{
    for _ in 0..ATTEMPTS {
        let Some((events, made)) = attempt().await? else {
            return Ok(None);
        };
        match log.append(scope, &events).await {
            Ok(()) => return Ok(Some((events, made))),
            Err(LogError::Conflict { .. } | LogError::Gap { .. }) => {}
            Err(error) => return Err(StreamError::Log(error).into()),
        }
    }
    Err(StreamError::Busy { thread }.into())
}

#[cfg(test)]
mod tests;
