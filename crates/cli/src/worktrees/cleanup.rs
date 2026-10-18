//! Cleaning up threads' worktrees, deterministically: a worktree goes when its thread is
//! archived, idle for 30 days or pruned, but only when that loses nothing (no uncommitted
//! changes, every commit on a remote); otherwise it is held, with why. One whose repository
//! is gone goes to the Trash. Each removal is recorded on its thread as `workspace.removed`,
//! and no branch is ever deleted.

use std::ffi::OsString;
use std::path::Path;

use glave_computer::worktrees::{self, Condition, Kept, Released, Trash};
use glave_event::{RemovedReason, ThreadId, WorkspaceRemoved};
use glave_harness::{last_active, owner_label, owner_of, record_removed};
use glave_ports::{Entropy, EventLog, Scope};
use jiff::{SignedDuration, Timestamp};

/// How long a thread may do nothing before its worktree is cleaned up.
pub const IDLE: SignedDuration = SignedDuration::from_hours(30 * 24);

/// What a cleanup works with: the log the threads are in, the environment git runs with, the
/// Trash, and the moment it runs.
pub struct Cleanup<'a, L> {
    pub log: &'a L,
    pub environment: &'a [(OsString, OsString)],
    pub trash: Option<&'a Trash>,
    pub now: Timestamp,
    pub entropy: &'a dyn Entropy,
}

/// The reason as `workspace.removed` and a hold name it.
fn named(reason: RemovedReason) -> &'static str {
    match reason {
        RemovedReason::Archived => "archived",
        RemovedReason::Idle => "idle",
        RemovedReason::RepositoryGone => "repository_gone",
        RemovedReason::Pruned => "pruned",
        RemovedReason::Released => "released",
    }
}

/// The name a person gives a worktree on the command line: its folder's name.
pub fn leaf(kept: &Kept) -> String {
    kept.path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

impl<L: EventLog> Cleanup<'_, L> {
    /// Releases `kept` for `reason`, recording a removal on its thread; one line saying what
    /// happened, for the person.
    pub async fn release(&self, kept: &Kept, reason: RemovedReason) -> String {
        let why = named(reason);
        let path = kept.path.display();
        match worktrees::release(kept, self.environment, why, self.trash).await {
            Ok(Released::Removed) => {
                let line = format!("removed {path} ({why}); its branch stays");
                self.recorded(kept, reason, None, line).await
            }
            Ok(Released::Trashed(moved)) => {
                let line = format!(
                    "moved {path} to the Trash ({}): its repository is gone",
                    moved.display()
                );
                let reason = RemovedReason::RepositoryGone;
                self.recorded(kept, reason, Some(&moved), line).await
            }
            Ok(Released::Held(risk)) => format!("kept {path} ({why}): {risk}"),
            Ok(Released::NoTrash) => {
                format!("kept {path}: its repository is gone, and there is no Trash to move it to")
            }
            Err(error) => format!("could not clean up {path}: {error}"),
        }
    }

    /// Removes `kept` whatever it holds, as the person asked by name; its branch stays.
    pub async fn force(&self, kept: &Kept) -> String {
        let path = kept.path.display();
        match worktrees::remove(kept, self.environment).await {
            Ok(()) => {
                let line = format!("removed {path} (forced); its branch stays");
                self.recorded(kept, RemovedReason::Pruned, None, line).await
            }
            Err(error) => format!("could not remove {path}: {error}"),
        }
    }

    /// Records the removal on the worktree's thread, when it names one; `line` otherwise.
    async fn recorded(
        &self,
        kept: &Kept,
        reason: RemovedReason,
        trash: Option<&Path>,
        line: String,
    ) -> String {
        let Some((scope, thread)) = kept.owner.as_deref().and_then(owner_of) else {
            return line;
        };
        let removed = WorkspaceRemoved {
            root: kept.path.to_string_lossy().into_owned(),
            branch: kept.branch.clone(),
            reason,
            trash: trash.map(|path| path.to_string_lossy().into_owned()),
            lost: None,
        };
        let random = self.entropy.bytes();
        match record_removed(self.log, &scope, thread, self.now, random, &removed).await {
            Ok(()) => line,
            Err(error) => format!("{line}, but it could not be recorded on its thread: {error}"),
        }
    }

    /// `glave worktrees prune`: removes every safe worktree, trashes those whose repository
    /// is gone, removes the ones named in `forced` whatever they hold, and says why each other
    /// one stays.
    pub async fn prune(&self, kept: &[Kept], forced: &[String]) -> Vec<String> {
        let mut lines = Vec::new();
        for kept in kept {
            let named = forced.iter().any(|name| *name == leaf(kept));
            let line = match kept.condition {
                Condition::Unsafe { .. } if named => self.force(kept).await,
                Condition::Unsafe { .. } => format!(
                    "kept {}: {}. To remove it anyway: glave worktrees prune --force {} (its \
                     branch stays)",
                    kept.path.display(),
                    kept.at_risk().unwrap_or_default(),
                    leaf(kept)
                ),
                // A scratch folder with files goes to the Trash, so nothing is lost.
                Condition::Safe | Condition::RepositoryGone | Condition::Scratch { .. } => {
                    self.release(kept, RemovedReason::Pruned).await
                }
            };
            lines.push(line);
        }
        lines
    }

    /// The daily check: worktrees whose repository is gone go to the Trash, and those whose
    /// thread has done nothing for [`IDLE`] are released as idle.
    pub async fn idle(&self, kept: &[Kept]) -> Vec<String> {
        let mut lines = Vec::new();
        for kept in kept {
            if kept.condition == Condition::RepositoryGone {
                lines.push(self.release(kept, RemovedReason::RepositoryGone).await);
                continue;
            }
            // Already held as idle: said once; it goes when it becomes safe.
            if kept.held.as_deref() == Some("idle") && kept.condition != Condition::Safe {
                continue;
            }
            let Some((scope, thread)) = kept.owner.as_deref().and_then(owner_of) else {
                continue;
            };
            let Ok(Some(last)) = last_active(self.log, &scope, thread).await else {
                continue;
            };
            if self.now.duration_since(last) >= IDLE {
                lines.push(self.release(kept, RemovedReason::Idle).await);
            }
        }
        lines
    }

    /// The thread in `scope` was archived or deleted: its worktrees are released.
    pub async fn archived(&self, kept: &[Kept], scope: Scope, thread: ThreadId) -> Vec<String> {
        let owner = owner_label(scope, thread);
        let mut lines = Vec::new();
        for kept in kept
            .iter()
            .filter(|kept| kept.owner.as_deref() == Some(&owner))
        {
            lines.push(self.release(kept, RemovedReason::Archived).await);
        }
        lines
    }
}

#[cfg(test)]
mod tests;
