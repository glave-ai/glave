//! A thread's own worktree: a checkout of the repository around a folder, on a branch of its
//! own, in a folder outside the repository, so threads working at once never touch each
//! other's files. Kept until the person removes it; opening the same branch again reopens it.

mod locks;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use glave_ports::computer::ComputerError;
use glave_ports::digest;
use glave_process::Processes;
use serde::{Deserialize, Serialize};

use super::git::{Git, not_an_option};
use super::paths;

pub(crate) use locks::Locks;

/// A worktree opened: the folder its workspace is rooted at, and its repository's shared git
/// folder.
pub(crate) struct Made {
    pub(crate) root: PathBuf,
    pub(crate) common: PathBuf,
}

/// The version of a record's format.
const VERSION: u32 = 1;

/// What glave keeps beside each worktree, as `<folder>.json`: whose it is, and whether a
/// cleanup kept it because removing it would lose work. It is this computer's own state, about
/// its own disk, so it stays beside the worktree rather than in the store (`specs/invariants.md`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Record {
    /// The repository's shared git folder.
    pub(crate) repository: PathBuf,
    pub(crate) branch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) owner: Option<String>,
    /// Why a cleanup wanted it gone but kept it, such as `idle`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) held: Option<String>,
}

/// A record as written: its version beside it.
#[derive(Serialize)]
struct Written<'a> {
    version: u32,
    #[serde(flatten)]
    record: &'a Record,
}

/// A record as read.
#[derive(Deserialize)]
struct Stored {
    version: u32,
    #[serde(flatten)]
    record: Record,
}

/// Where the record for the worktree at `dest` is kept: beside it, never inside it.
pub(crate) fn record_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".json");
    dest.with_file_name(name)
}

/// The record beside the worktree at `dest`; `None` when there is none, or only one glave
/// cannot read, such as one torn by a crash (it is written whole the next time). Refuses one a
/// newer glave wrote, rather than misreading it.
pub(crate) async fn read_record(dest: &Path) -> Result<Option<Record>, ComputerError> {
    let path = record_path(dest);
    let Ok(text) = tokio::fs::read(&path).await else {
        return Ok(None);
    };
    let Ok(stored) = serde_json::from_slice::<Stored>(&text) else {
        return Ok(None);
    };
    if stored.version != VERSION {
        return Err(ComputerError::Refused(format!(
            "{} was written by a newer version of glave (record version {}); update glave to \
             use it",
            path.display(),
            stored.version
        )));
    }
    Ok(Some(stored.record))
}

/// Writes the record beside the worktree at `dest`, whole or not at all; refuses to replace
/// one a newer glave wrote.
pub(crate) async fn write_record(dest: &Path, record: &Record) -> Result<(), ComputerError> {
    read_record(dest).await?;
    let path = record_path(dest);
    let written = Written {
        version: VERSION,
        record,
    };
    let text = serde_json::to_vec_pretty(&written)
        .map_err(|error| ComputerError::Failed(format!("{}: {error}", path.display())))?;
    let at = path.clone();
    tokio::task::spawn_blocking(move || glave_ports::file::replace(&at, &text))
        .await
        .map_err(|error| ComputerError::Failed(format!("{}: {error}", path.display())))?
        .map_err(|error| {
            ComputerError::Failed(format!("could not write {}: {error}", path.display()))
        })
}

/// Where the worktree for `branch` of the repository around `path` lives under `base`,
/// created (in its turn among `locks`) if it is not there yet, with `owner` kept beside it and
/// any hold cleared; answers the folder matching `path` inside it. git runs as one of
/// `processes`, with `environment`.
pub(crate) async fn open(
    (processes, environment): (&Processes, &[(OsString, OsString)]),
    locks: &Locks,
    base: &Path,
    path: &str,
    branch: &str,
    owner: Option<&str>,
) -> Result<Made, ComputerError> {
    let folder = paths::folder(path).await?;
    not_an_option("the branch", branch)?;
    let git = Git::at(processes, environment, &folder);
    let Ok(top) = git.ask(&["rev-parse", "--show-toplevel"]).await? else {
        return Err(ComputerError::Refused(format!(
            "{path} is not in a git repository, so it cannot have a worktree. Turn the worktree \
             option off to work in the folder itself, or make it a repository with a commit."
        )));
    };
    if git
        .ask(&["rev-parse", "--verify", "--quiet", "HEAD"])
        .await?
        .is_err()
    {
        return Err(ComputerError::Refused(format!(
            "the repository at {top} has no commits yet, and a worktree starts from one. \
             Commit first, or turn the worktree option off."
        )));
    }
    if git
        .ask(&["check-ref-format", "--branch", branch])
        .await?
        .is_err()
    {
        return Err(ComputerError::Refused(format!(
            "{branch:?} is not a valid branch name"
        )));
    }
    let prefix = git.must(&["rev-parse", "--show-prefix"]).await?;
    let common = common_dir(&git).await?;
    let dest = base.join(repository_id(&top, &common)).join(leaf(branch));
    let turn = locks.lock(&common).await?;
    if tokio::fs::try_exists(&dest).await.unwrap_or(false) {
        check_reusable(processes, environment, &dest, &common, branch).await?;
    } else {
        add(&git, &dest, branch).await?;
    }
    drop(turn);
    let record = Record {
        repository: common.clone(),
        branch: branch.to_owned(),
        owner: owner.map(str::to_owned),
        held: None,
    };
    write_record(&dest, &record).await?;
    let inside = dest.join(&prefix);
    let root = if prefix.is_empty() || !tokio::fs::try_exists(&inside).await.unwrap_or(false) {
        dest
    } else {
        inside
    };
    let root = paths::folder(&root.to_string_lossy()).await?;
    Ok(Made { root, common })
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
    processes: &Processes,
    environment: &[(OsString, OsString)],
    dest: &Path,
    common: &Path,
    branch: &str,
) -> Result<(), ComputerError> {
    let there = Git::at(processes, environment, dest);
    let same_repository = common_dir(&there).await.is_ok_and(|found| found == common);
    // git that gives no answer says so, rather than have a good worktree taken for another.
    let on = there
        .ask(&["symbolic-ref", "--short", "-q", "HEAD"])
        .await?;
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
        .ask(&["rev-parse", "--verify", "--quiet", &reference])
        .await?
        .is_ok();
    if exists {
        git.work(&["worktree", "add", "--quiet", &dest, branch])
            .await?;
    } else {
        git.work(&["worktree", "add", "--quiet", "-b", branch, &dest, "HEAD"])
            .await?;
    }
    Ok(())
}
