//! A person's computer in the cloud kept from filling, for what only the server can say
//! (`specs/computers.md`, decision 7): as a turn that used the computer ends, with more than 80%
//! of its disk used, the worktrees of archived threads, then of threads idle for 7 days, oldest
//! first, are freed until it is below, each recorded on its thread so it opens its worktree
//! again when it comes back. The computer looks again at each just before removing it, and keeps
//! what it can't rebuild. Build outputs, caches and the Trash it frees itself (`Keeper` in
//! `glave-computer`). A look that freed nothing waits an hour for the next.

use std::sync::{Arc, PoisonError};

use glave_event::{ComputerKind, RemovedReason, ThreadId, WorkspaceRemoved};
use glave_ports::computer::{ComputerId, DiskSpace, Freed, KeptWorktree};
use glave_ports::{BlobStore, EventLog, ModelProvider, Scope, Standing};
use jiff::{SignedDuration, Timestamp};

use crate::approval::Approver;
use crate::harness::{Harness, HarnessError, Thread};
use crate::removed::{owner_of, record_removed};
use crate::tools::workspace::Workspace;

/// How long a thread must have done nothing for its worktree to be freed for room.
const IDLE: SignedDuration = SignedDuration::from_hours(7 * 24);

/// How long after a look that freed nothing the next one waits: what fills the disk is the
/// person's, and asking about each worktree at every turn's end would only slow them.
const AFTER_NOTHING: SignedDuration = SignedDuration::from_hours(1);

/// A worktree that may go for room: archived before idle, then the oldest.
struct Candidate {
    not_archived: bool,
    active: Timestamp,
    scope: Scope,
    thread: ThreadId,
    kept: KeptWorktree,
    reason: RemovedReason,
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Frees room on `thread`'s person's own computer, just used by its turn, when it is full.
    pub(crate) async fn free_room_at_home(&self, thread: &Thread) {
        let Ok((home, ComputerKind::Person)) = self.home_of(thread.scope, thread.for_) else {
            return;
        };
        let (computer, now) = (home.info().computer, self.clock.now());
        if self.freed_nothing_lately(computer, now) {
            return;
        }
        if let Err(error) = self.free_room(thread, &home).await {
            tracing::warn!(%error, "could not free room on a person's computer");
        }
    }

    async fn free_room(&self, thread: &Thread, home: &Arc<Workspace>) -> Result<(), HarnessError> {
        let (computers, computer) = (home.computers(), home.info().computer);
        if !self.is_full(home, computer).await? {
            return Ok(());
        }
        let kept = computers.worktrees(computer).await?;
        let mut freed = false;
        for candidate in self.candidates(thread, kept).await {
            let path = candidate.kept.path.clone();
            // One that can't be freed now is no reason to leave the rest.
            match computers.free_worktree(computer, &path).await {
                Ok(Freed::Removed) => {
                    self.record_freed(&candidate).await;
                    freed = true;
                }
                Ok(Freed::Kept { why }) => tracing::info!(%why, "a worktree stays, for now"),
                Err(error) => tracing::warn!(%error, "a worktree could not be freed"),
            }
            if !self.is_full(home, computer).await? {
                return Ok(());
            }
        }
        if !freed {
            self.freed_nothing(computer, self.clock.now());
        }
        Ok(())
    }

    /// Whether the last look at `computer`'s disk, within [`AFTER_NOTHING`], freed nothing.
    fn freed_nothing_lately(&self, computer: ComputerId, now: Timestamp) -> bool {
        self.lacking_room
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&computer)
            .is_some_and(|at| now.duration_since(*at) < AFTER_NOTHING)
    }

    /// Notes that a look at `computer`'s disk freed nothing `now`, forgetting those long past.
    fn freed_nothing(&self, computer: ComputerId, now: Timestamp) {
        let mut lacking = self
            .lacking_room
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        lacking.retain(|_, at| now.duration_since(*at) < AFTER_NOTHING);
        lacking.insert(computer, now);
    }

    async fn is_full(&self, home: &Workspace, computer: ComputerId) -> Result<bool, HarnessError> {
        let info = home.computers().status(computer).await?;
        Ok(info.disk.is_some_and(DiskSpace::full))
    }

    /// Of `kept`, what may go for room, in the order it goes: never `thread`'s own, one of
    /// another organization's or with a turn open.
    async fn candidates(&self, thread: &Thread, kept: Vec<KeptWorktree>) -> Vec<Candidate> {
        let now = self.clock.now();
        let mut candidates = Vec::new();
        for kept in kept {
            let Some((scope, id)) = kept.owner.as_deref().and_then(owner_of) else {
                continue;
            };
            if id == thread.id || scope.org() != thread.scope.org() {
                continue;
            }
            let Ok(Some(view)) = self.log.thread_view(&scope, id).await else {
                continue;
            };
            // A label is a file on the computer, which any command could write: never another
            // person's thread, whatever it says.
            if view.turn.is_some() || view.whose() != thread.for_ {
                continue;
            }
            let active = view.updated_at.timestamp();
            let archived = view.standing == Standing::Archived;
            let idle = now.duration_since(active) >= IDLE;
            if !archived && !idle {
                continue;
            }
            candidates.push(Candidate {
                not_archived: !archived,
                active,
                scope,
                thread: id,
                kept,
                reason: if archived {
                    RemovedReason::Archived
                } else {
                    RemovedReason::Idle
                },
            });
        }
        candidates.sort_by_key(|candidate| (candidate.not_archived, candidate.active));
        candidates
    }

    /// Records on its thread that `candidate` was freed, so it opens it again when it comes back.
    async fn record_freed(&self, candidate: &Candidate) {
        let removed = WorkspaceRemoved {
            root: candidate.kept.path.clone(),
            branch: candidate.kept.branch.clone(),
            reason: candidate.reason,
            trash: None,
            lost: None,
        };
        let recorded = match self.entropy.bytes() {
            Ok(random) => {
                let at = self.clock.now();
                let (scope, thread) = (candidate.scope, candidate.thread);
                record_removed(&self.log, &scope, thread, at, random, &removed)
                    .await
                    .map_err(|error| error.to_string())
            }
            Err(error) => Err(error.to_string()),
        };
        if let Err(error) = recorded {
            tracing::warn!(%error, "a worktree freed for room could not be recorded");
        }
    }
}
