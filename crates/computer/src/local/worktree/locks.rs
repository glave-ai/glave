//! A repository's worktrees are made one at a time. git does not make two at once safe:
//! `git worktree add` reads every worktree's folder under the repository's `.git/worktrees`,
//! and can find one another `add` has only half made. So each waits for the one before it,
//! keyed by the repository's shared git folder, which two paths to one repository share: in
//! this process first, then across processes (the person's server and a `glave` run in a
//! terminal) by a lock on a file in that folder. Each takes milliseconds.

use std::collections::HashMap;
use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use glave_ports::computer::{ComputerError, Refusal};
use tokio::sync::OwnedMutexGuard;

/// The file in a repository's shared git folder whose lock every glave takes to make a
/// worktree of it.
const LOCK_FILE: &str = "glave-worktree.lock";

/// The turn to make a worktree of one repository, until it is dropped.
pub(crate) struct Turn {
    _file: File,
    _here: OwnedMutexGuard<()>,
}

/// The most repositories a worktree is made in at once.
const MAX_REPOSITORIES: usize = 1_024;

/// A lock for each repository whose worktrees are being made. A lock lives only while someone
/// holds it or waits for it, so none is kept for a repository once its worktrees are made.
#[derive(Debug, Default)]
pub(crate) struct Locks {
    repositories: Mutex<HashMap<PathBuf, Weak<tokio::sync::Mutex<()>>>>,
}

impl Locks {
    /// Waits until no other worktree of the repository whose shared git folder is `common`
    /// (canonical) is being made, by this process or another; the next one waits until the
    /// answer is dropped.
    pub(crate) async fn lock(&self, common: &Path) -> Result<Turn, ComputerError> {
        let here = self.of(common)?.lock_owned().await;
        let path = common.join(LOCK_FILE);
        let file = match try_lock_file(&path)? {
            Some(file) => file,
            // Another process is making one: wait for it off the async threads.
            None => tokio::task::spawn_blocking(move || lock_file(&path))
                .await
                .map_err(|error| {
                    ComputerError::Failed(format!("waiting for the worktree lock: {error}"))
                })??,
        };
        Ok(Turn {
            _file: file,
            _here: here,
        })
    }

    /// The lock for `common`, made when no one holds or waits for one. Making one forgets
    /// every lock no one holds or waits for.
    fn of(&self, common: &Path) -> Result<Arc<tokio::sync::Mutex<()>>, ComputerError> {
        let mut repositories = self
            .repositories
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(lock) = repositories.get(common).and_then(Weak::upgrade) {
            return Ok(lock);
        }
        repositories.retain(|_, lock| lock.strong_count() > 0);
        if repositories.len() >= MAX_REPOSITORIES {
            return Err(ComputerError::Refused(
                Refusal::TooManyRepositories {
                    most: MAX_REPOSITORIES.to_string(),
                }
                .into(),
            ));
        }
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        repositories.insert(common.to_path_buf(), Arc::downgrade(&lock));
        Ok(lock)
    }
}

fn open_lock_file(path: &Path) -> Result<File, ComputerError> {
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .map_err(|error| {
            ComputerError::Failed(format!("could not open {}: {error}", path.display()))
        })
}

/// The lock on the file at `path`, unless another holds it.
fn try_lock_file(path: &Path) -> Result<Option<File>, ComputerError> {
    let file = open_lock_file(path)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(error)) => Err(locking(path, &error)),
    }
}

/// The lock on the file at `path`, once whoever holds it lets go. Blocks.
fn lock_file(path: &Path) -> Result<File, ComputerError> {
    let file = open_lock_file(path)?;
    file.lock().map_err(|error| locking(path, &error))?;
    Ok(file)
}

fn locking(path: &Path, error: &std::io::Error) -> ComputerError {
    ComputerError::Failed(format!("could not lock {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use glave_ports::block_on;
    use glave_ports::testing::TempDir;

    use super::*;

    /// A shared git folder of its own for `name`, empty.
    fn common(name: &str) -> TempDir {
        TempDir::new(&format!("worktree-locks-{name}")).unwrap()
    }

    /// Whether `lock` is taken when first asked, rather than waiting in this process.
    async fn taken_at_once(locks: &Locks, common: &Path) -> bool {
        tokio::select! {
            biased;
            _ = locks.of(common).unwrap().lock_owned() => true,
            () = std::future::ready(()) => false,
        }
    }

    #[test]
    fn a_change_waits_only_for_one_to_the_same_repository() {
        let locks = Locks::default();
        let (app, other) = (common("app"), common("other"));
        block_on!(async {
            let held = locks.lock(&app).await.unwrap();
            assert!(
                !taken_at_once(&locks, &app).await,
                "the same repository waits"
            );
            assert!(taken_at_once(&locks, &other).await, "another goes ahead");
            drop(held);
            assert!(
                taken_at_once(&locks, &app).await,
                "free once the first is done: one that stopped waiting holds nothing"
            );
        });
    }

    #[test]
    fn two_processes_take_turns_by_the_file_in_the_git_folder() {
        // Two sets of locks share nothing in memory, as two processes don't.
        let (first, second) = (Locks::default(), Locks::default());
        let app = common("processes");
        block_on!(async {
            let held = first.lock(&app).await.unwrap();
            assert!(
                try_lock_file(&app.join(LOCK_FILE)).unwrap().is_none(),
                "another process finds the file locked"
            );
            let waiting = tokio::spawn({
                let app = app.to_path_buf();
                async move { second.lock(&app).await.map(|_| ()) }
            });
            drop(held);
            waiting.await.unwrap().unwrap();
            assert!(
                try_lock_file(&app.join(LOCK_FILE)).unwrap().is_some(),
                "and free once both are done"
            );
        });
    }

    #[test]
    fn locks_last_only_while_held_and_are_capped() {
        let locks = Locks::default();
        let common = |n: usize| PathBuf::from(format!("/code/{n}/.git"));
        let mut held = Vec::new();
        for n in 0..MAX_REPOSITORIES {
            held.push(locks.of(&common(n)).unwrap());
        }
        let refused = locks.of(&common(MAX_REPOSITORIES));
        assert!(
            matches!(refused, Err(ComputerError::Refused(_))),
            "{refused:?}"
        );
        held.clear();
        let _one = locks.of(&common(0)).unwrap();
        assert_eq!(
            locks.repositories.lock().unwrap().len(),
            1,
            "the rest are gone"
        );
    }
}
