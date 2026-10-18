//! Whether a thread is shown: active, archived (hidden from lists, kept, reversible) or
//! deleted (hidden for good, marked for erasure). The latest `thread.archived`,
//! `thread.unarchived` or `thread.deleted` in its log decides. And renaming: a title of the
//! person's own, recorded as `thread.renamed`.

use glave_event::{
    Body, Event, ThreadArchived, ThreadDeleted, ThreadId, ThreadRenamed, ThreadUnarchived,
};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider, Scope, Standing};
use jiff::Timestamp;

use crate::approval::Approver;
use crate::harness::{Harness, HarnessError, Thread};
use crate::removed::append_next;

/// The event types that decide a thread's standing.
pub(crate) const KINDS: &[&str] = &["thread.archived", "thread.unarchived", "thread.deleted"];

/// The standing `events` leave a thread in that was in `before` them.
pub(crate) fn standing_after(before: Standing, events: &[Event]) -> Standing {
    events
        .iter()
        .fold(before, |standing, event| match event.body {
            Body::ThreadArchived(_) => Standing::Archived,
            Body::ThreadUnarchived(_) => Standing::Active,
            Body::ThreadDeleted(_) => Standing::Deleted,
            _ => standing,
        })
}

/// The longest title a person may give a thread, in characters.
pub const MAX_TITLE: usize = 200;

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
        // A thread put away keeps nothing running.
        if change != Change::Unarchive {
            self.release_commands(thread).await?;
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

    /// Gives a thread this harness holds the title `title` for `caller`, recorded as
    /// `thread.renamed`. The title is trimmed; one that is empty or longer than [`MAX_TITLE`]
    /// characters is refused. A deleted thread is no longer there to rename.
    pub async fn rename(
        &self,
        thread: &mut Thread,
        caller: &Caller,
        title: &str,
    ) -> Result<(), HarnessError> {
        let title = title.trim();
        if title.is_empty() || title.chars().count() > MAX_TITLE {
            return Err(HarnessError::Invalid(format!(
                "a title is 1 to {MAX_TITLE} characters, not only spaces"
            )));
        }
        self.take_in(thread).await?;
        if thread.standing == Standing::Deleted {
            return Err(HarnessError::NotFound(thread.id));
        }
        let by = Self::other_than_for(thread, caller.person);
        let renamed = ThreadRenamed {
            title: title.to_owned(),
            by,
        };
        self.record(thread, &renamed).await?;
        // Glave's own record: call ids do not count it, so the model is sent the same.
        thread.unnumbered = thread.unnumbered.saturating_add(1);
        Ok(())
    }
}
