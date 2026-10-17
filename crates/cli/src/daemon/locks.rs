//! How the task hosts of one glave folder, one per folder, share the work every one of them
//! could do, now that each runs every project's tasks and schedules, until one server serves
//! the person (`specs/one-server.md`). Two locks in GLAVE_HOME/run, which the system lets go of
//! however a host ends: each host holds `hosts.lock` shared while it runs, so a host that can
//! hold it alone knows no other host runs tasks, and only then records the tasks left open as
//! interrupted; and one host at a time holds `schedules.lock` and fires the schedules.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

use anyhow::Context as _;

use crate::home;

/// A lock file in `folder`, made private to the person when it is new.
fn open(folder: &Path, name: &str) -> anyhow::Result<File> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let path = folder.join(format!("{name}.lock"));
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(&path)
        .with_context(|| format!("could not open {}", path.display()))
}

/// Takes `file`'s lock alone if no one holds it: whether it did.
fn take(file: &File) -> anyhow::Result<bool> {
    match file.try_lock() {
        Ok(()) => Ok(true),
        Err(TryLockError::WouldBlock) => Ok(false),
        Err(TryLockError::Error(error)) => Err(error).context("could not take a task host's lock"),
    }
}

/// Where the locks are: GLAVE_HOME/run.
pub fn folder() -> anyhow::Result<PathBuf> {
    Ok(home::folder()?.join("run"))
}

/// This host's share among the running hosts: the held lock, and whether this host is the
/// only one, in which case `alone` runs first, while no other host can start.
pub async fn join<F: Future<Output = anyhow::Result<()>>>(
    folder: &Path,
    alone: impl FnOnce() -> F,
) -> anyhow::Result<File> {
    let hosts = open(folder, "hosts")?;
    if take(&hosts)? {
        alone().await?;
        hosts
            .unlock()
            .context("could not share the task hosts' lock")?;
    }
    // Shared, it waits only for a host that is recovering tasks alone.
    tokio::task::spawn_blocking(move || hosts.lock_shared().map(|()| hosts))
        .await?
        .context("could not share the task hosts' lock")
}

/// The schedules' lock, when no other host holds it: whoever has it fires the schedules.
pub fn schedules(folder: &Path) -> anyhow::Result<Option<File>> {
    let file = open(folder, "schedules")?;
    Ok(take(&file)?.then_some(file))
}

/// Whether a task host fires the schedules now.
pub fn schedules_fired(folder: &Path) -> bool {
    // Taken here only to look: closing the file lets go of it.
    matches!(schedules(folder), Ok(None))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn only_a_lone_host_recovers_and_one_host_fires_the_schedules() {
        let dir = std::env::temp_dir().join(format!("glave-locks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let runtime = runtime();
        let recovered = std::cell::Cell::new(0);
        let recover = || async {
            recovered.set(recovered.get() + 1);
            Ok(())
        };
        let first = runtime.block_on(join(&dir, recover)).unwrap();
        assert_eq!(recovered.get(), 1, "the first host is alone");
        let second = runtime.block_on(join(&dir, recover)).unwrap();
        assert_eq!(recovered.get(), 1, "another host is running");
        drop((first, second));
        let _third = runtime.block_on(join(&dir, recover)).unwrap();
        assert_eq!(recovered.get(), 2, "alone again once the others ended");

        assert!(!schedules_fired(&dir));
        let firing = schedules(&dir).unwrap();
        assert!(firing.is_some());
        assert!(schedules_fired(&dir));
        assert!(schedules(&dir).unwrap().is_none(), "one host at a time");
        drop(firing);
        assert!(!schedules_fired(&dir), "free once its host ends");
    }
}
