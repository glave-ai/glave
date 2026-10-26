//! What was written to a thread from outside the harness holding it. Glave keeps a few records
//! of its own about a thread that do not change the conversation: a cleanup removing its
//! worktree, archiving, unarchiving, deleting or renaming it, and a title's call (`title.rs`).
//! They may be appended by another glave process, such as `glave threads archive` or the daily
//! worktree check, or by the server's own title task, while this one holds the thread. When an append then meets the log's `(thread, seq)` conflict, the harness reads
//! what was written after its last event; if all of it is such a record, it takes it in and
//! writes its own events after it. Anything else, such as a message, means another harness is
//! running the thread, and stays a conflict.

use std::sync::Arc;

use glave_event::{Body, Event, RawEvent, Seq, decode};
use glave_ports::computer::Sources;
use glave_ports::{BlobStore, EventLog, LogError, ModelProvider, Standing};

use crate::approval::Approver;
use crate::harness::{Harness, HarnessError, Thread};
use crate::removed::owner_label;
use crate::standing::{KINDS, standing_after};

/// The most records taken in at once; more than this from outside is not a cleanup.
const MOST: usize = 100;

/// Whether `event` is one glave writes about a thread from outside, and can be taken in. These
/// are the records call ids do not count (`bind::own_records`), less `workspace.bound`, which
/// only the harness starting a thread writes, and which would move where its tools act; and a
/// `model.responded` on its own, a title's call, which only adds to the thread's usage.
/// `hook.ran` is not one: only a harness running a turn in the thread writes it.
fn takes_in(event: &Event) -> bool {
    matches!(
        event.body,
        Body::WorkspaceRemoved(_)
            | Body::ThreadArchived(_)
            | Body::ThreadUnarchived(_)
            | Body::ThreadDeleted(_)
            | Body::ThreadRenamed(_)
            | Body::ModelResponded(_)
    )
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Takes in the records written to `thread` from outside since its last event, and says
    /// whether it could: `false` when something else was written, which `thread` cannot follow.
    /// Call ids count what it takes in as resuming the thread would, so the model is sent the
    /// same.
    pub(crate) async fn take_in(&self, thread: &mut Thread) -> Result<bool, HarnessError> {
        let last = Seq::new(thread.next_seq.get().saturating_sub(1)).ok();
        let page = self.log.read(&thread.scope, thread.id, last, MOST).await?;
        if page.len() >= MOST {
            return Ok(false);
        }
        let mut events = Vec::with_capacity(page.len());
        for raw in page {
            events.push(decode(raw).map_err(|e| HarnessError::Event(e.to_string()))?);
        }
        if !events.iter().all(takes_in) {
            return Ok(false);
        }
        let count = u64::try_from(events.len()).map_err(|e| HarnessError::Event(e.to_string()))?;
        thread.next_seq = Seq::new(thread.next_seq.get().saturating_add(count))?;
        thread.unnumbered = thread
            .unnumbered
            .saturating_add(crate::bind::own_records(&events));
        thread.standing = standing_after(thread.standing, &events);
        if thread.workspace.is_some()
            && events
                .iter()
                .any(|event| matches!(event.body, Body::WorkspaceRemoved(_)))
        {
            thread.removed = true;
            self.reopen_removed(thread).await?;
        }
        Ok(true)
    }

    /// After `append` failed with `error`, takes in what was written from outside, if it can,
    /// and numbers `events` after it. A thread that is now deleted, or archived when `events`
    /// do not change that, stops with an error instead.
    pub(crate) async fn follow(
        &self,
        thread: &mut Thread,
        error: LogError,
        events: &mut [RawEvent],
    ) -> Result<(), HarnessError> {
        if !matches!(error, LogError::Conflict { .. }) || !self.take_in(thread).await? {
            return Err(error.into());
        }
        let changes_standing = events
            .iter()
            .any(|event| KINDS.contains(&event.kind.as_str()));
        match thread.standing {
            Standing::Deleted => return Err(HarnessError::NotFound(thread.id)),
            Standing::Archived if !changes_standing => {
                return Err(HarnessError::Archived(thread.id));
            }
            Standing::Active | Standing::Archived => {}
        }
        for (event, offset) in events.iter_mut().zip(0_u64..) {
            event.seq = Seq::new(thread.next_seq.get().saturating_add(offset))?;
        }
        Ok(())
    }

    /// Opens the thread's worktree again from its branch, when a cleanup removed it while this
    /// harness held the thread, as resuming the thread would, and closes the one it replaces.
    /// An archived or deleted thread's stays removed until it is active again.
    pub(crate) async fn reopen_removed(&self, thread: &mut Thread) -> Result<(), HarnessError> {
        if !thread.removed || thread.standing != Standing::Active {
            return Ok(());
        }
        let again = match thread
            .workspace
            .as_ref()
            .map(|workspace| workspace.info().sources.clone())
        {
            Some(Sources::Worktree { path, branch, .. }) => {
                let owner = owner_label(thread.scope, thread.id);
                Some(
                    self.workspace
                        .worktree(&path, &branch, Some(&owner))
                        .await?,
                )
            }
            Some(Sources::Scratch { name, owner }) => {
                Some(self.workspace.scratch(&name, owner.as_deref()).await?)
            }
            _ => None,
        };
        if let Some(again) = again {
            let replaced = thread.workspace.replace(Arc::new(again));
            self.close_workspace(replaced).await;
        }
        thread.removed = false;
        Ok(())
    }
}
