//! A thread's own worktree: a checkout of the repository around a folder, on a branch of its
//! own, in a folder outside the repository, so threads working at once never touch each
//! other's files. Kept until the person removes it; opening the same branch again reopens it.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use glave_ports::{ComputerError, digest};

use super::git::{Git, not_an_option};
use super::paths;

/// Where the worktree for `branch` of the repository around `path` lives under `base`,
/// created if it is not there yet; answers the folder matching `path` inside it.
pub(crate) async fn open(
    environment: &[(OsString, OsString)],
    base: &Path,
    path: &str,
    branch: &str,
) -> Result<PathBuf, ComputerError> {
    let folder = paths::folder(path).await?;
    not_an_option("the branch", branch)?;
    let git = Git::at(environment, &folder);
    let Ok(top) = git.run(&["rev-parse", "--show-toplevel"]).await else {
        return Err(ComputerError::Refused(format!(
            "{path} is not in a git repository, so it cannot have a worktree. Turn the worktree \
             option off to work in the folder itself, or make it a repository with a commit."
        )));
    };
    if git
        .run(&["rev-parse", "--verify", "--quiet", "HEAD"])
        .await
        .is_err()
    {
        return Err(ComputerError::Refused(format!(
            "the repository at {top} has no commits yet, and a worktree starts from one. \
             Commit first, or turn the worktree option off."
        )));
    }
    if git
        .run(&["check-ref-format", "--branch", branch])
        .await
        .is_err()
    {
        return Err(ComputerError::Refused(format!(
            "{branch:?} is not a valid branch name"
        )));
    }
    let prefix = git.must(&["rev-parse", "--show-prefix"]).await?;
    let common = common_dir(&git).await?;
    let dest = base.join(repository_id(&top, &common)).join(leaf(branch));
    if tokio::fs::try_exists(&dest).await.unwrap_or(false) {
        check_reusable(environment, &dest, &common, branch).await?;
    } else {
        add(&git, &dest, branch).await?;
    }
    let inside = dest.join(&prefix);
    let root = if prefix.is_empty() || !tokio::fs::try_exists(&inside).await.unwrap_or(false) {
        dest
    } else {
        inside
    };
    paths::folder(&root.to_string_lossy()).await
}

/// The repository's shared git folder, made canonical: the same for every worktree of it.
pub(crate) async fn common_dir(git: &Git<'_>) -> Result<PathBuf, ComputerError> {
    let common = git
        .must(&["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .await?;
    tokio::fs::canonicalize(&common)
        .await
        .map_err(|error| ComputerError::Failed(format!("{common}: {error}")))
}

/// A folder name for a repository: its name, and a short digest of where its history is, so
/// two repositories with one name never share a folder.
fn repository_id(top: &str, common: &Path) -> String {
    let name = Path::new(top)
        .file_name()
        .map_or_else(|| "repository".into(), |name| name.to_string_lossy());
    let hash = digest(common.to_string_lossy().as_bytes());
    let short = hash.as_str().get(..8).unwrap_or_default();
    format!("{name}-{short}")
}

/// A folder name for a branch: `glave/0a1b2c3d4e5f` is `glave-0a1b2c3d4e5f`.
fn leaf(branch: &str) -> String {
    branch.replace('/', "-")
}

/// Refuses a folder already at the worktree's place unless it is this repository's worktree on
/// `branch`.
async fn check_reusable(
    environment: &[(OsString, OsString)],
    dest: &Path,
    common: &Path,
    branch: &str,
) -> Result<(), ComputerError> {
    let there = Git::at(environment, dest);
    let same_repository = common_dir(&there).await.is_ok_and(|found| found == common);
    let on = there.run(&["symbolic-ref", "--short", "-q", "HEAD"]).await;
    if same_repository && on.as_deref() == Ok(branch) {
        Ok(())
    } else {
        Err(ComputerError::Refused(format!(
            "{} already exists and is not this repository's worktree on {branch}; move it \
             away, or remove it with `glave worktrees prune`",
            dest.display()
        )))
    }
}

/// Adds the worktree: on `branch` if it exists, or on a new branch from the current commit.
async fn add(git: &Git<'_>, dest: &Path, branch: &str) -> Result<(), ComputerError> {
    let parent = dest
        .parent()
        .ok_or_else(|| ComputerError::Failed(format!("{} has no parent", dest.display())))?;
    tokio::fs::create_dir_all(parent).await.map_err(|error| {
        ComputerError::Failed(format!("could not create {}: {error}", parent.display()))
    })?;
    let dest = dest.to_string_lossy();
    let reference = format!("refs/heads/{branch}");
    let exists = git
        .run(&["rev-parse", "--verify", "--quiet", &reference])
        .await
        .is_ok();
    if exists {
        git.must(&["worktree", "add", "--quiet", &dest, branch])
            .await?;
    } else {
        git.must(&["worktree", "add", "--quiet", "-b", branch, &dest, "HEAD"])
            .await?;
    }
    Ok(())
}
