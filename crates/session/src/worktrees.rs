//! `worktrees/*` and the worktree side of archiving: cleanup that runs in the server, so every
//! `workspace.removed` it records goes through the one writer of the threads the server holds.

use glave_computer::worktrees::Kept;
use glave_event::{ThreadId, WorkspaceRemoved};
use glave_harness::record_removed;
use glave_ports::{BlobStore, EventLog, ModelProvider, Scope};
use jiff::Timestamp;
use tokio::sync::watch;

use crate::cleanup::{Cleanup, Ledger, leaf};
use crate::held::{Slot, Use, finished};
use crate::protocol::{ApiError, Reply, WorktreeInfo, code};
use crate::server::Server;

/// A worktree as the API shows it.
pub fn info(kept: &Kept) -> WorktreeInfo {
    WorktreeInfo {
        path: kept.path.to_string_lossy().into_owned(),
        name: leaf(kept),
        branch: kept.branch.clone(),
        at_risk: kept.at_risk(),
        held: kept.held.clone(),
    }
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// `worktrees/list`.
    pub(crate) async fn worktrees_list(&self) -> Result<Reply, ApiError> {
        let Some(cleaning) = &self.inner.cleaning else {
            return Ok(Reply::Worktrees {
                worktrees: Vec::new(),
            });
        };
        let kept = cleaning.list().await.map_err(|error| failed(&error))?;
        Ok(Reply::Worktrees {
            worktrees: kept.iter().map(info).collect(),
        })
    }

    /// `worktrees/prune`.
    pub(crate) async fn worktrees_prune(&self, force: &[String]) -> Result<Reply, ApiError> {
        let notes = self.cleaned(Job::Prune(force)).await?;
        Ok(Reply::Cleaned { notes })
    }

    /// `worktrees/idle`.
    pub(crate) async fn worktrees_idle(&self) -> Result<Reply, ApiError> {
        let notes = self.cleaned(Job::Idle).await?;
        Ok(Reply::Cleaned { notes })
    }

    /// Releases the worktrees `thread` made, as it was archived or deleted. A failure to look
    /// is reported as a note, never an error: the thread is archived either way.
    pub(crate) async fn release_worktrees(&self, thread: ThreadId) -> Vec<String> {
        let job = Job::Archived(self.inner.scope, thread);
        self.cleaned(job).await.unwrap_or_else(|error| {
            vec![format!("could not clean up worktrees: {}", error.message)]
        })
    }

    /// Does `job` with a cleanup over this server's ledger and the worktrees kept here.
    async fn cleaned(&self, job: Job<'_>) -> Result<Vec<String>, ApiError> {
        let Some(cleaning) = &self.inner.cleaning else {
            return Ok(Vec::new());
        };
        let kept = cleaning.list().await.map_err(|error| failed(&error))?;
        let now = self.inner.harness.clock().now();
        let trash = cleaning.trash(now);
        let cleanup = Cleanup {
            ledger: self,
            environment: &cleaning.environment,
            trash: trash.as_ref(),
            now,
            entropy: self.inner.harness.entropy(),
        };
        Ok(match job {
            Job::Prune(force) => cleanup.prune(&kept, force).await,
            Job::Idle => cleanup.idle(&kept).await,
            Job::Archived(scope, thread) => cleanup.archived(&kept, scope, thread).await,
        })
    }
}

/// What a cleanup is asked to do.
enum Job<'a> {
    /// Remove what is safe, and the worktrees named (by folder) whatever they hold.
    Prune(&'a [String]),
    /// The daily check.
    Idle,
    /// Release one thread's worktrees.
    Archived(Scope, ThreadId),
}

fn failed(error: &glave_ports::ComputerError) -> ApiError {
    ApiError::new(code::INTERNAL, error.message())
}

impl<P, L, B> Ledger for Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    fn in_use(&self, scope: &Scope, thread: ThreadId) -> bool {
        *scope == self.inner.scope && matches!(self.lock().get(&thread), Some(Slot::Busy { .. }))
    }

    async fn last_active(
        &self,
        scope: &Scope,
        thread: ThreadId,
    ) -> Result<Option<Timestamp>, String> {
        glave_harness::last_active(self.inner.harness.log(), scope, thread)
            .await
            .map_err(|error| error.to_string())
    }

    async fn record(
        &self,
        scope: &Scope,
        thread: ThreadId,
        at: Timestamp,
        random: [u8; 10],
        removed: &WorkspaceRemoved,
    ) -> Result<(), String> {
        let held = *scope == self.inner.scope && self.lock().contains_key(&thread);
        if !held {
            // Not a thread this server holds: nothing here can fall out of step with the log.
            return record_removed(self.inner.harness.log(), scope, thread, at, random, removed)
                .await
                .map_err(|error| error.to_string());
        }
        let (finish, mut done) = watch::channel(None);
        let removed = removed.clone();
        self.occupy_for(
            thread,
            finish,
            Use::Record,
            move |inner, mut held, _| async move {
                let recorded = inner.harness.record_removal(&mut held, &removed).await;
                (held, recorded)
            },
        )
        .await
        .map_err(|error| error.message)?;
        let ended = finished(&mut done)
            .await
            .ok_or_else(|| "the recording stopped".to_owned())?;
        ended
            .as_ref()
            .as_ref()
            .map_err(ToString::to_string)
            .copied()
    }
}
