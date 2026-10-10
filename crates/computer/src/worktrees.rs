//! The worktrees kept on this machine: what each one is, and removing them. Threads' worktrees
//! are never removed on their own; the person lists them and removes the ones they are done
//! with, and one with changes not yet committed only when they say so.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use glave_ports::ComputerError;

use crate::local::git::{Git, not_an_option};

/// The most worktrees a listing reads.
const MAX_LISTED: usize = 1_000;

/// One kept worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kept {
    /// The worktree's folder.
    pub path: PathBuf,
    /// The repository it is a worktree of: its main checkout.
    pub repository: PathBuf,
    /// The branch it is on; `None` when it is detached.
    pub branch: Option<String>,
    /// Whether it holds changes or new files not committed.
    pub changed: bool,
    /// Whether everything committed on it is already in the repository's default branch (or
    /// its main checkout's commit, without a remote): removing it loses no commit.
    pub merged: bool,
}

/// What pruning did with one worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pruned {
    Removed(Kept),
    /// Left in place: it holds uncommitted changes and the person did not confirm.
    KeptChanged(Kept),
}

/// The worktrees under `base`, one folder per repository, sorted by path. Folders that are not
/// worktrees of a repository git can still read are left out.
pub async fn list(
    base: &Path,
    environment: &[(OsString, OsString)],
) -> Result<Vec<Kept>, ComputerError> {
    let mut found = Vec::new();
    for repository in folders(base).await? {
        for path in folders(&repository).await? {
            if found.len() >= MAX_LISTED {
                break;
            }
            if let Some(kept) = describe(&path, environment).await {
                found.push(kept);
            }
        }
    }
    found.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(found)
}

/// Removes each of `chosen`. One with uncommitted changes is removed only if `confirm` says
/// yes for it; its branch is kept unless it was merged, so no commit is lost.
pub async fn prune(
    chosen: Vec<Kept>,
    environment: &[(OsString, OsString)],
    mut confirm: impl FnMut(&Kept) -> bool,
) -> Result<Vec<Pruned>, ComputerError> {
    let mut done = Vec::with_capacity(chosen.len());
    for kept in chosen {
        if kept.changed && !confirm(&kept) {
            done.push(Pruned::KeptChanged(kept));
            continue;
        }
        remove(&kept, environment).await?;
        done.push(Pruned::Removed(kept));
    }
    Ok(done)
}

async fn remove(kept: &Kept, environment: &[(OsString, OsString)]) -> Result<(), ComputerError> {
    let git = Git::at(environment, &kept.repository);
    let path = kept.path.to_string_lossy();
    let mut args = vec!["worktree", "remove"];
    if kept.changed {
        args.push("--force");
    }
    args.push(&path);
    git.must(&args).await?;
    if let Some(branch) = kept.branch.as_deref().filter(|_| kept.merged) {
        not_an_option("the branch", branch)?;
        // A branch git does not see as merged is kept: the worktree is gone, its commits not.
        let _ = git.run(&["branch", "-d", branch]).await;
    }
    Ok(())
}

/// What the worktree at `path` is, or `None` when it is not one git can read.
async fn describe(path: &Path, environment: &[(OsString, OsString)]) -> Option<Kept> {
    let git = Git::at(environment, path);
    let common = crate::local::worktree::common_dir(&git).await.ok()?;
    let repository = common.parent()?.to_path_buf();
    let top = git.run(&["rev-parse", "--show-toplevel"]).await.ok()?;
    let canonical = tokio::fs::canonicalize(path).await.ok()?;
    if Path::new(&top) != canonical || repository == canonical {
        return None;
    }
    let branch = git
        .run(&["symbolic-ref", "--short", "-q", "HEAD"])
        .await
        .ok();
    let changed = git
        .run(&["status", "--porcelain"])
        .await
        .map_or(true, |status| !status.is_empty());
    let main = Git::at(environment, &repository);
    let base = match main
        .run(&[
            "rev-parse",
            "--verify",
            "--quiet",
            "refs/remotes/origin/HEAD",
        ])
        .await
    {
        Ok(base) => Some(base),
        Err(_) => main
            .run(&["rev-parse", "--verify", "--quiet", "HEAD"])
            .await
            .ok(),
    };
    let tip = git.run(&["rev-parse", "HEAD"]).await.ok()?;
    let merged = match base {
        Some(base) => git
            .run(&["merge-base", "--is-ancestor", &tip, &base])
            .await
            .is_ok(),
        None => false,
    };
    Some(Kept {
        path: canonical,
        repository,
        branch,
        changed,
        merged,
    })
}

/// The folders in `parent`, sorted; none when it does not exist.
async fn folders(parent: &Path) -> Result<Vec<PathBuf>, ComputerError> {
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
