//! A repository's worktrees are made one at a time. git does not make two at once safe:
//! `git worktree add` reads every worktree's folder under the repository's `.git/worktrees`,
//! and can find one another `add` has only half made. So each waits for the one before it,
//! keyed by the repository's shared git folder, which two paths to one repository share.
//! Each takes milliseconds.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use glave_ports::computer::ComputerError;
use tokio::sync::OwnedMutexGuard;

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
    /// (canonical) is being made; the next one waits until the answer is dropped.
    pub(crate) async fn lock(&self, common: &Path) -> Result<OwnedMutexGuard<()>, ComputerError> {
        let lock = self.of(common)?;
        Ok(lock.lock_owned().await)
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
            return Err(ComputerError::Refused(format!(
                "worktrees are being made in {MAX_REPOSITORIES} repositories at once on this \
                 computer; try again when some are done"
            )));
        }
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        repositories.insert(common.to_path_buf(), Arc::downgrade(&lock));
        Ok(lock)
    }
}

#[cfg(test)]
mod tests {
    use glave_ports::block_on;

    use super::*;

    /// Whether `lock` is taken when first asked, rather than waiting.
    async fn taken_at_once(locks: &Locks, common: &Path) -> bool {
        tokio::select! {
            biased;
            _ = locks.lock(common) => true,
            () = std::future::ready(()) => false,
        }
    }

    #[test]
    fn a_change_waits_only_for_one_to_the_same_repository() {
        let locks = Locks::default();
        let (app, other) = (Path::new("/code/app/.git"), Path::new("/code/other/.git"));
        block_on!(async {
            let held = locks.lock(app).await.unwrap();
            assert!(
                !taken_at_once(&locks, app).await,
                "the same repository waits"
            );
            assert!(taken_at_once(&locks, other).await, "another goes ahead");
            drop(held);
            assert!(
                taken_at_once(&locks, app).await,
                "free once the first is done: one that stopped waiting holds nothing"
            );
        });
    }

    #[test]
    fn locks_last_only_while_held_and_are_capped() {
        let locks = Locks::default();
        let common = |n: usize| PathBuf::from(format!("/code/{n}/.git"));
        block_on!(async {
            let mut held = Vec::new();
            for n in 0..MAX_REPOSITORIES {
                held.push(locks.lock(&common(n)).await.unwrap());
            }
            let refused = locks.lock(&common(MAX_REPOSITORIES)).await;
            assert!(
                matches!(refused, Err(ComputerError::Refused(_))),
                "{refused:?}"
            );
            held.clear();
            let _one = locks.lock(&common(0)).await.unwrap();
            assert_eq!(
                locks.repositories.lock().unwrap().len(),
                1,
                "the rest are gone"
            );
        });
    }
}
