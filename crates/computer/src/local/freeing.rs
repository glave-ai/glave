//! What this machine keeps for threads that a server may free for room (`specs/computers.md`,
//! decision 7): worktrees, removed only when what they hold can be rebuilt, and scratch folders,
//! removed when empty and otherwise moved to the Trash, so nothing is lost until the Trash is
//! emptied. Each is looked at again just before it goes, and none goes with a workspace open on
//! it.

use std::path::Path;

use glave_ports::computer::{ComputerError, ComputerId, Freed, KeptWorktree};
use glave_ports::{Clock as _, SystemClock};

use super::LocalComputers;
use crate::worktrees::{self, Trash};

/// Why something stays, for the record.
fn kept_because(why: &str) -> Freed {
    Freed::Kept {
        why: why.to_owned(),
    }
}

impl LocalComputers {
    /// The worktrees and scratch folders kept here for threads, with no git.
    pub(super) async fn kept_for_threads(
        &self,
        computer: ComputerId,
    ) -> Result<Vec<KeptWorktree>, ComputerError> {
        self.registry.environment(computer)?;
        let mut kept = Vec::new();
        if let Some(base) = &self.worktrees {
            let recorded = worktrees::recorded(base).await?;
            kept.extend(recorded.into_iter().map(|(path, record)| KeptWorktree {
                path: path.to_string_lossy().into_owned(),
                branch: Some(record.branch).filter(|branch| !branch.is_empty()),
                owner: record.owner,
                scratch: false,
            }));
        }
        if let Some(base) = &self.scratch {
            let scratch = worktrees::list_scratch(base).await?;
            kept.extend(scratch.into_iter().map(|kept| KeptWorktree {
                path: kept.path.to_string_lossy().into_owned(),
                branch: None,
                owner: kept.owner,
                scratch: true,
            }));
        }
        Ok(kept)
    }

    /// Frees the worktree or scratch folder at `path`, looked at again now.
    pub(super) async fn free_kept(
        &self,
        computer: ComputerId,
        path: &str,
    ) -> Result<Freed, ComputerError> {
        self.registry.environment(computer)?;
        if let Some(base) = &self.scratch
            && let Some(kept) = worktrees::scratch_at(base, Path::new(path)).await?
        {
            return self.free_scratch(&kept).await;
        }
        let Some(base) = &self.worktrees else {
            return Ok(kept_because("this computer keeps no worktrees"));
        };
        let (environment, processes) = (&self.environment, &self.processes);
        let found = worktrees::at(base, Path::new(path), environment, processes);
        let Some(kept) = found.await? else {
            return Ok(kept_because("it is not here any more"));
        };
        // Nothing opens a worktree of this repository meanwhile, and a thread that had this
        // one open is seen last, with the removal to follow at once.
        let _turn = self.worktree_locks.lock(&kept.repository).await?;
        if let Some(risk) = kept.at_risk() {
            return Ok(kept_because(&risk));
        }
        if !worktrees::only_rebuildable(&kept, environment, processes).await? {
            return Ok(kept_because("it holds more than a build would make again"));
        }
        if self.registry.open_on(&kept.path)? {
            return Ok(kept_because("a thread has it open"));
        }
        worktrees::remove(&kept, environment, processes).await?;
        Ok(Freed::Removed)
    }

    /// Frees a scratch folder no workspace has open: removed when empty, else moved to the
    /// person's Trash, which keeps it until room is needed. With no Trash, it stays.
    async fn free_scratch(&self, kept: &worktrees::Kept) -> Result<Freed, ComputerError> {
        if self.registry.open_on(&kept.path)? {
            return Ok(kept_because("a thread has it open"));
        }
        let deleted_at = SystemClock.now().strftime("%Y-%m-%dT%H:%M:%S").to_string();
        let trash = Trash::for_environment(&self.environment, &deleted_at);
        let released = worktrees::release(
            kept,
            &self.environment,
            &self.processes,
            "room",
            trash.as_ref(),
        );
        Ok(match released.await? {
            worktrees::Released::Removed => Freed::Removed,
            worktrees::Released::Trashed(to) => Freed::Trashed {
                to: to.to_string_lossy().into_owned(),
            },
            worktrees::Released::NoTrash => {
                kept_because("it holds files, and there is no Trash to move them to")
            }
            worktrees::Released::Held(why) => Freed::Kept { why },
        })
    }
}
