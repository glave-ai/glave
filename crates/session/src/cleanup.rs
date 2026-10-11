//! Cleaning up threads' worktrees, deterministically: a worktree goes when its thread is
//! archived, idle for 30 days or pruned, but only when that loses nothing (no uncommitted
//! changes, every commit on a remote); otherwise it is held, with why. One whose repository
//! is gone goes to the Trash. Each removal is recorded on its thread as `workspace.removed`,
//! and no branch is ever deleted.
//!
//! Recording goes through a [`Ledger`], so that when a session server holds the thread the
//! event is appended by that server, the thread's one writer, and when no server runs it is
//! appended to the log directly.

use std::ffi::OsString;
use std::future::Future;
use std::path::{Path, PathBuf};

use glave_computer::worktrees::{self, Condition, Kept, Released, Trash};
use glave_event::{RemovedReason, ThreadId, WorkspaceRemoved};
use glave_harness::{Entropy, last_active, owner_label, owner_of, record_removed};
use glave_ports::ComputerError;
use glave_ports::{EventLog, Scope};
use jiff::{SignedDuration, Timestamp};

/// How long a thread may do nothing before its worktree is cleaned up.
pub const IDLE: SignedDuration = SignedDuration::from_hours(30 * 24);

/// The folder in glave's folder where threads' worktrees are kept, one folder per repository.
pub const FOLDER: &str = "worktrees";

/// The file in glave's folder holding when worktrees were last checked.
pub const SWEPT: &str = "worktrees-checked";

/// How often glave checks worktrees as it starts.
const EVERY: SignedDuration = SignedDuration::from_hours(24);

/// Where a cleanup learns about threads and records on them.
pub trait Ledger: Send + Sync {
    /// Whether a turn is running in the thread, so its worktree must stay where it is.
    fn in_use(&self, scope: &Scope, thread: ThreadId) -> bool;

    /// When the thread last did anything, if it exists.
    fn last_active(
        &self,
        scope: &Scope,
        thread: ThreadId,
    ) -> impl Future<Output = Result<Option<Timestamp>, String>> + Send;

    /// Records, as the thread's next event, that its worktree was removed.
    fn record(
        &self,
        scope: &Scope,
        thread: ThreadId,
        at: Timestamp,
        random: [u8; 10],
        removed: &WorkspaceRemoved,
    ) -> impl Future<Output = Result<(), String>> + Send;
}

/// A ledger that reads and appends straight to the log, for when no server holds the threads.
pub struct InLog<'a, L>(pub &'a L);

impl<L: EventLog> Ledger for InLog<'_, L> {
    fn in_use(&self, _scope: &Scope, _thread: ThreadId) -> bool {
        false
    }

    async fn last_active(
        &self,
        scope: &Scope,
        thread: ThreadId,
    ) -> Result<Option<Timestamp>, String> {
        last_active(self.0, scope, thread)
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
        record_removed(self.0, scope, thread, at, random, removed)
            .await
            .map_err(|error| error.to_string())
    }
}

/// Where this machine keeps threads' worktrees, and what cleaning them up needs.
#[derive(Debug, Clone)]
pub struct Cleaning {
    /// The folder the worktrees are kept in, one folder per repository.
    pub base: PathBuf,
    /// The environment git runs with.
    pub environment: Vec<(OsString, OsString)>,
    /// The person's home folder, to find their Trash; without one nothing is trashed.
    pub home: Option<PathBuf>,
    /// `XDG_DATA_HOME`, where the Trash lives on Linux when it is set.
    pub data_home: Option<PathBuf>,
}

impl Cleaning {
    /// The worktrees kept here.
    pub async fn list(&self) -> Result<Vec<Kept>, ComputerError> {
        worktrees::list(&self.base, &self.environment).await
    }

    /// The person's Trash, for something moved to it at `now`, if this machine has one glave
    /// knows.
    pub fn trash(&self, now: Timestamp) -> Option<Trash> {
        let deleted_at = now.strftime("%Y-%m-%dT%H:%M:%S").to_string();
        Trash::for_home(
            self.home.as_deref()?,
            self.data_home.as_deref(),
            &deleted_at,
        )
    }
}

/// What a cleanup works with: the ledger of threads, the environment git runs with, the
/// Trash, and the moment it runs.
pub struct Cleanup<'a, T> {
    pub ledger: &'a T,
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
    }
}

/// The name a person gives a worktree on the command line: its folder's name.
pub fn leaf(kept: &Kept) -> String {
    kept.path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Whether the daily check of worktrees is due at `now`: false when the last one was less
/// than a day before, or there are no worktrees to check. When it is due, the time is
/// recorded first, so a check that fails is not retried until tomorrow.
pub async fn is_due(home: &Path, now: Timestamp) -> Result<bool, String> {
    let stamp = home.join(SWEPT);
    let last = tokio::fs::read_to_string(&stamp)
        .await
        .ok()
        .and_then(|text| text.trim().parse::<Timestamp>().ok());
    if last.is_some_and(|last| now.duration_since(last) < EVERY) {
        return Ok(false);
    }
    if !tokio::fs::try_exists(home.join(FOLDER))
        .await
        .unwrap_or(false)
    {
        return Ok(false);
    }
    tokio::fs::write(&stamp, now.to_string())
        .await
        .map_err(|error| format!("could not write {}: {error}", stamp.display()))?;
    Ok(true)
}

impl<T: Ledger> Cleanup<'_, T> {
    /// Releases `kept` for `reason`, recording a removal on its thread; one line saying what
    /// happened, for the person.
    pub async fn release(&self, kept: &Kept, reason: RemovedReason) -> String {
        let why = named(reason);
        let path = kept.path.display();
        if self.in_use(kept) {
            return format!("kept {path} ({why}): its thread is running a turn");
        }
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
        if self.in_use(kept) {
            return format!("kept {path}: its thread is running a turn");
        }
        match worktrees::remove(kept, self.environment).await {
            Ok(()) => {
                let line = format!("removed {path} (forced); its branch stays");
                self.recorded(kept, RemovedReason::Pruned, None, line).await
            }
            Err(error) => format!("could not remove {path}: {error}"),
        }
    }

    /// Whether the worktree's thread has a turn running.
    fn in_use(&self, kept: &Kept) -> bool {
        kept.owner
            .as_deref()
            .and_then(owner_of)
            .is_some_and(|(scope, thread)| self.ledger.in_use(&scope, thread))
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
        };
        let random = self.entropy.bytes();
        match self
            .ledger
            .record(&scope, thread, self.now, random, &removed)
            .await
        {
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
                Condition::Safe | Condition::RepositoryGone => {
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
            let Ok(Some(last)) = self.ledger.last_active(&scope, thread).await else {
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
