//! The worktrees kept on this machine: what each one is, whether removing it would lose work,
//! and removing, holding or trashing it. A worktree is safe to remove when it has no
//! uncommitted changes and every commit on it is on some remote (so a squash-merged branch
//! counts); removing one never deletes its branch. One whose repository is gone is moved to
//! the Trash, never deleted. Threads' scratch folders are kept and cleaned up the same way:
//! an empty one is removed, and one holding files goes to the Trash, so nothing is lost.

mod trash;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use glave_ports::computer::ComputerError;
use glave_process::Processes;

use crate::local::git::Git;
use crate::local::worktree::{Record, read_record, record_path, write_record};

pub use trash::Trash;

/// The most worktrees a listing reads.
const MAX_LISTED: usize = 1_000;

/// One kept worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kept {
    /// The worktree's folder.
    pub path: PathBuf,
    /// Its repository's shared git folder; empty for a scratch folder.
    pub repository: PathBuf,
    /// The branch it is on; `None` when it is detached or unknown.
    pub branch: Option<String>,
    /// Whose it is, as the label it was opened with: the thread it belongs to.
    pub owner: Option<String>,
    /// Why a cleanup wanted it gone but kept it, such as `idle`.
    pub held: Option<String>,
    pub condition: Condition,
}

/// Whether removing a worktree would lose anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Condition {
    /// Nothing uncommitted, and every commit on some remote.
    Safe,
    Unsafe {
        /// Changes or new files not committed.
        changed: bool,
        /// Commits on no remote.
        unpushed: u64,
    },
    /// Its repository's shared git folder no longer exists.
    RepositoryGone,
    /// A thread's scratch folder, not a worktree: `empty` when it holds nothing.
    Scratch { empty: bool },
}

impl Kept {
    /// Why removing it would lose work, as the person reads it; `None` when it would not.
    pub fn at_risk(&self) -> Option<String> {
        match self.condition {
            Condition::Safe | Condition::Scratch { empty: true } => None,
            Condition::RepositoryGone => Some("its repository is gone".into()),
            Condition::Scratch { empty: false } => {
                Some("files in a scratch folder, which go to the Trash".into())
            }
            Condition::Unsafe { changed, unpushed } => {
                let mut parts = Vec::new();
                if changed {
                    parts.push("uncommitted changes".to_owned());
                }
                match unpushed {
                    0 => {}
                    1 => parts.push("1 commit not pushed".to_owned()),
                    n => parts.push(format!("{n} commits not pushed")),
                }
                Some(parts.join(", "))
            }
        }
    }
}

/// What a cleanup did with one worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Released {
    /// Removed; its branch is kept.
    Removed,
    /// Kept and marked held, because removing it would lose this.
    Held(String),
    /// Its repository is gone, so it was moved to the Trash, here.
    Trashed(PathBuf),
    /// Its repository is gone and there is no Trash to move it to, so it was kept.
    NoTrash,
}

/// The worktrees under `base`, one folder per repository, sorted by path. Folders that are
/// neither a worktree nor one glave recorded are left out.
pub async fn list(
    base: &Path,
    environment: &[(OsString, OsString)],
    processes: &Processes,
) -> Result<Vec<Kept>, ComputerError> {
    let mut found = Vec::new();
    for repository in folders(base).await? {
        for path in folders(&repository).await? {
            if found.len() >= MAX_LISTED {
                break;
            }
            if let Some(kept) = describe(&path, environment, processes).await? {
                found.push(kept);
            }
        }
    }
    found.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(found)
}

/// The scratch folders under `base`, sorted by path. Folders glave did not record are left
/// out.
pub async fn list_scratch(base: &Path) -> Result<Vec<Kept>, ComputerError> {
    let mut found = Vec::new();
    for path in folders(base).await? {
        let Some(record) = read_record(&path).await? else {
            continue;
        };
        let Ok(path) = tokio::fs::canonicalize(&path).await else {
            continue;
        };
        let empty = match tokio::fs::read_dir(&path).await {
            Ok(mut entries) => matches!(entries.next_entry().await, Ok(None)),
            Err(_) => false,
        };
        found.push(Kept {
            path,
            repository: PathBuf::new(),
            branch: None,
            owner: record.owner,
            held: record.held,
            condition: Condition::Scratch { empty },
        });
    }
    Ok(found)
}

/// A cleanup wants `kept` gone for `why` (such as `idle`): removed when that is safe, moved
/// to `trash` when its repository is gone, and otherwise kept and marked held for `why`.
pub async fn release(
    kept: &Kept,
    environment: &[(OsString, OsString)],
    processes: &Processes,
    why: &str,
    trash: Option<&Trash>,
) -> Result<Released, ComputerError> {
    match kept.condition {
        Condition::Safe | Condition::Scratch { empty: true } => {
            remove(kept, environment, processes).await?;
            Ok(Released::Removed)
        }
        Condition::RepositoryGone | Condition::Scratch { empty: false } => match trash {
            Some(trash) => {
                let moved = trash.put(&kept.path).await?;
                let _ = tokio::fs::remove_file(record_path(&kept.path)).await;
                Ok(Released::Trashed(moved))
            }
            None => Ok(Released::NoTrash),
        },
        Condition::Unsafe { .. } => {
            hold(kept, why).await?;
            Ok(Released::Held(kept.at_risk().unwrap_or_default()))
        }
    }
}

/// Whether `kept` holds a file git ignores that a build wouldn't make again: the person's, such
/// as a `.env`, which a worktree freed for room would lose.
pub(crate) async fn holds_own_ignored(
    kept: &Kept,
    environment: &[(OsString, OsString)],
    processes: &Processes,
) -> Result<bool, ComputerError> {
    let git = Git {
        processes,
        environment,
        at: &kept.path,
        options: Vec::new(),
        credential: None,
    };
    let said = git.must(&["status", "--ignored", "--porcelain"]).await?;
    Ok(said
        .lines()
        .filter_map(|line| line.strip_prefix("!! "))
        .filter_map(|path| path.split('/').next())
        .any(|top| !crate::disk::BUILT.contains(&top)))
}

/// Removes the worktree, whatever it holds, and its record; its branch is kept. For a worktree
/// that is not safe, only when the person said so by name.
pub async fn remove(
    kept: &Kept,
    environment: &[(OsString, OsString)],
    processes: &Processes,
) -> Result<(), ComputerError> {
    if matches!(kept.condition, Condition::Scratch { .. }) {
        tokio::fs::remove_dir_all(&kept.path)
            .await
            .map_err(|error| {
                ComputerError::Failed(format!("could not remove {}: {error}", kept.path.display()))
            })?;
        let _ = tokio::fs::remove_file(record_path(&kept.path)).await;
        return Ok(());
    }
    let mut git_dir = OsString::from("--git-dir=");
    git_dir.push(&kept.repository);
    let git = Git {
        processes,
        environment,
        at: &kept.repository,
        options: vec![git_dir],
        credential: None,
    };
    let path = kept.path.to_string_lossy();
    git.work(&["worktree", "remove", "--force", &path]).await?;
    let _ = tokio::fs::remove_file(record_path(&kept.path)).await;
    Ok(())
}

/// Marks the worktree held for `why`, in its record.
async fn hold(kept: &Kept, why: &str) -> Result<(), ComputerError> {
    let mut record = read_record(&kept.path).await?.unwrap_or_else(|| Record {
        repository: kept.repository.clone(),
        branch: kept.branch.clone().unwrap_or_default(),
        ..Record::default()
    });
    record.held = Some(why.to_owned());
    write_record(&kept.path, &record).await
}

/// What the worktree at `path` is, or `None` when it is neither a worktree nor recorded.
async fn describe(
    path: &Path,
    environment: &[(OsString, OsString)],
    processes: &Processes,
) -> Result<Option<Kept>, ComputerError> {
    let record = read_record(path).await?;
    let repository = match &record {
        Some(record) => record.repository.clone(),
        None => match common_from_link(path).await {
            Some(common) => common,
            None => return Ok(None),
        },
    };
    let Ok(path) = tokio::fs::canonicalize(path).await else {
        return Ok(None);
    };
    let mut kept = Kept {
        repository,
        branch: record
            .as_ref()
            .map(|record| record.branch.clone())
            .filter(|branch| !branch.is_empty()),
        owner: record.as_ref().and_then(|record| record.owner.clone()),
        held: record.as_ref().and_then(|record| record.held.clone()),
        condition: Condition::RepositoryGone,
        path,
    };
    if !tokio::fs::try_exists(&kept.repository)
        .await
        .unwrap_or(false)
    {
        return Ok(Some(kept));
    }
    let git = Git::at(processes, environment, &kept.path);
    if let Ok(branch) = git.run(&["symbolic-ref", "--short", "-q", "HEAD"]).await {
        kept.branch = Some(branch);
    }
    // Anything git cannot answer counts against removing it.
    let changed = git
        .run(&["status", "--porcelain"])
        .await
        .map_or(true, |status| !status.is_empty());
    let unpushed = git
        .run(&["rev-list", "--count", "HEAD", "--not", "--remotes"])
        .await
        .ok()
        .and_then(|count| count.parse::<u64>().ok())
        .unwrap_or(u64::MAX);
    kept.condition = if !changed && unpushed == 0 {
        Condition::Safe
    } else {
        Condition::Unsafe { changed, unpushed }
    };
    Ok(Some(kept))
}

/// The shared git folder a worktree's `.git` file points into: `gitdir: <common>/worktrees/<n>`.
async fn common_from_link(path: &Path) -> Option<PathBuf> {
    let link = tokio::fs::read_to_string(path.join(".git")).await.ok()?;
    let gitdir = PathBuf::from(link.strip_prefix("gitdir:")?.trim());
    let worktrees = gitdir.parent()?;
    (worktrees.file_name()? == "worktrees").then(|| worktrees.parent().map(Path::to_path_buf))?
}

/// The folders in `parent`, sorted; none when it does not exist.
pub(crate) async fn folders(parent: &Path) -> Result<Vec<PathBuf>, ComputerError> {
    let mut entries = match tokio::fs::read_dir(parent).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(ComputerError::Failed(format!(
                "could not read {}: {error}",
                parent.display()
            )));
        }
    };
    let mut found = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(|error| {
        ComputerError::Failed(format!("could not read {}: {error}", parent.display()))
    })? {
        if found.len() >= MAX_LISTED {
            break;
        }
        if entry.file_type().await.is_ok_and(|kind| kind.is_dir()) {
            found.push(entry.path());
        }
    }
    found.sort();
    Ok(found)
}

#[cfg(test)]
mod tests;
