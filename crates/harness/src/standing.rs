//! Whether a thread is shown: active, archived (hidden from lists, kept, reversible) or
//! deleted (hidden for good, marked for erasure). The latest `thread.archived`,
//! `thread.unarchived` or `thread.deleted` in its log decides.

use glave_event::{Body, Event, ThreadArchived, ThreadDeleted, ThreadId, ThreadUnarchived};
use glave_ports::{BlobStore, EventLog, ModelProvider, Scope};
use jiff::Timestamp;

use crate::approval::Approver;
use crate::caller::Caller;
use crate::harness::{Harness, HarnessError, Thread};
use crate::removed::append_next;

/// The event types that decide a thread's standing.
pub(crate) const KINDS: &[&str] = &["thread.archived", "thread.unarchived", "thread.deleted"];

/// Whether a thread is shown.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Standing {
    /// Shown in lists.
    #[default]
    Active,
    /// Hidden from lists and kept in the log; `unarchive` brings it back.
    Archived,
    /// Hidden for good, and waiting for its content to be erased.
    Deleted,
}

impl Standing {
    /// The standing the thread's `events` leave it in.
    pub(crate) fn of(events: &[Event]) -> Self {
        Self::Active.after(events)
    }

    /// The standing `events` leave a thread in that was in `self` before them.
    pub(crate) fn after(self, events: &[Event]) -> Self {
        events
            .iter()
            .fold(self, |standing, event| match event.body {
                Body::ThreadArchived(_) => Self::Archived,
                Body::ThreadUnarchived(_) => Self::Active,
                Body::ThreadDeleted(_) => Self::Deleted,
                _ => standing,
            })
    }
}

/// What to do to a thread's standing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Archive,
    Unarchive,
    Delete,
}

/// What a change does to a thread in a given standing.
enum Plan {
    /// It is already as asked.
    Nothing,
    Record,
    /// The thread is deleted, so it is no longer there to change.
    Missing,
}

impl Change {
    fn plan(self, standing: Standing) -> Plan {
        match (standing, self) {
            (Standing::Deleted, _) => Plan::Missing,
            (Standing::Archived, Self::Archive) | (Standing::Active, Self::Unarchive) => {
                Plan::Nothing
            }
            _ => Plan::Record,
        }
    }

    /// The standing after the change.
    fn result(self) -> Standing {
        match self {
            Self::Archive => Standing::Archived,
            Self::Unarchive => Standing::Active,
            Self::Delete => Standing::Deleted,
        }
    }
}

/// The standing of `thread`, from its latest deciding event.
pub async fn standing_of<L: EventLog>(
    log: &L,
    scope: &Scope,
    thread: ThreadId,
) -> Result<Standing, HarnessError> {
    let latest = log.latest_of(scope, thread, KINDS).await?;
    Ok(match latest.as_ref().map(|event| event.kind.as_str()) {
        Some("thread.archived") => Standing::Archived,
        Some("thread.deleted") => Standing::Deleted,
        _ => Standing::Active,
    })
}

/// Applies `change` to a thread straight in the log, made `at` with `random` for its id, as the
/// local person (so `by` is not recorded), from outside any harness: a harness holding the
/// thread takes the change in before its next event (`outside.rs`). A change that leaves
/// things as they are records nothing.
pub async fn change_standing<L: EventLog>(
    log: &L,
    scope: &Scope,
    thread: ThreadId,
    at: Timestamp,
    random: [u8; 10],
    change: Change,
) -> Result<(), HarnessError> {
    if log.read(scope, thread, None, 1).await?.is_empty() {
        return Err(HarnessError::NotFound(thread));
    }
    match change.plan(standing_of(log, scope, thread).await?) {
        Plan::Missing => return Err(HarnessError::NotFound(thread)),
        Plan::Nothing => return Ok(()),
        Plan::Record => {}
    }
    match change {
        Change::Archive => {
            append_next(log, scope, thread, at, random, &ThreadArchived { by: None }).await
        }
        Change::Unarchive => {
            append_next(
                log,
                scope,
                thread,
                at,
                random,
                &ThreadUnarchived { by: None },
            )
            .await
        }
        Change::Delete => {
            append_next(log, scope, thread, at, random, &ThreadDeleted { by: None }).await
        }
    }
}

impl Thread {
    /// Whether the thread is shown, archived or deleted.
    pub fn standing(&self) -> Standing {
        self.standing
    }
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Applies `change` for `caller` to a thread this harness holds, recording it as the
    /// thread's next event, after taking in any change made from outside meanwhile. A change
    /// that leaves things as they are records nothing.
    pub async fn change_standing(
        &self,
        thread: &mut Thread,
        caller: &Caller,
        change: Change,
    ) -> Result<(), HarnessError> {
        // Anything else written from outside makes the record below a conflict, as ever.
        self.take_in(thread).await?;
        match change.plan(thread.standing) {
            Plan::Missing => return Err(HarnessError::NotFound(thread.id)),
            Plan::Nothing => return Ok(()),
            Plan::Record => {}
        }
        let by = Self::other_than_for(thread, caller.person);
        match change {
            Change::Archive => self.record(thread, &ThreadArchived { by }).await,
            Change::Unarchive => self.record(thread, &ThreadUnarchived { by }).await,
            Change::Delete => self.record(thread, &ThreadDeleted { by }).await,
        }?;
        thread.standing = change.result();
        // Glave's own record: call ids do not count it, so the model is sent the same.
        thread.unnumbered = thread.unnumbered.saturating_add(1);
        Ok(())
    }
}
