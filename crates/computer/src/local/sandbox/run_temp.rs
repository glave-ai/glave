//! One sandboxed command's own temp folder (its `TMPDIR`), made when it starts and removed,
//! with everything the command left in it, once it ends however it ends; and those a glave
//! killed outright left, which the next glave to prepare a sandbox removes (#562). Each is
//! named for the glave that made it, so one whose glave is gone is nobody's.

use std::path::{Path, PathBuf};

use rustix::io::Errno;
use rustix::process::{Pid, test_kill_process};

/// The most tries at a name no other command holds.
const ATTEMPTS: u32 = 1_000;

/// A command's temp folder, removed when dropped.
#[derive(Debug)]
pub(crate) struct RunTemp {
    path: PathBuf,
}

impl RunTemp {
    /// Makes a new one, private to this user, in `parent`, named `<prefix><pid>-<n>`.
    pub(super) async fn new(parent: &Path, prefix: &str) -> std::io::Result<Self> {
        let pid = std::process::id();
        for attempt in 0..ATTEMPTS {
            let path = parent.join(format!("{prefix}{pid}-{attempt}"));
            let mut builder = tokio::fs::DirBuilder::new();
            #[cfg(unix)]
            builder.mode(0o700);
            match builder.create(&path).await {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!(
                "could not make a temp folder for the sandbox in {}",
                parent.display()
            ),
        ))
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for RunTemp {
    fn drop(&mut self) {
        // Best effort, and blocking: what a command leaves in its temp folder is small, as
        // package caches go to glave's own cache folder when there is one.
        let _ = glave_ports::remove_tree(&self.path);
    }
}

/// Removes the folders in `parent` named `<prefix><pid>-<n>` whose glave, `pid`, is gone: what
/// a glave killed outright, which removes none of its own, left. A folder whose number is a
/// running process's now stays, as that process might be a glave; it goes once that process
/// has. Best effort, and blocking.
pub(super) fn remove_left(parent: &Path, prefix: &str) {
    let Ok(listed) = std::fs::read_dir(parent) else {
        return;
    };
    for entry in listed.filter_map(Result::ok) {
        let name = entry.file_name();
        let Some(glave) = name.to_str().and_then(|name| glave_of(name, prefix)) else {
            continue;
        };
        if gone(glave) {
            let _ = glave_ports::remove_tree(&entry.path());
        }
    }
}

/// The glave a folder called `name` was made by, when it is named `<prefix><pid>-<n>`.
fn glave_of(name: &str, prefix: &str) -> Option<i32> {
    let (pid, n) = name.strip_prefix(prefix)?.split_once('-')?;
    n.parse::<u32>().ok()?;
    pid.parse().ok()
}

/// Whether no process numbered `pid` runs: one another user runs is still there.
fn gone(pid: i32) -> bool {
    match Pid::from_raw(pid) {
        Some(pid) => test_kill_process(pid) == Err(Errno::SRCH),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glave_ports::testing::TempDir;

    /// No process can have this number: past the most either system gives.
    const NO_PROCESS: u32 = 999_999_999;

    #[test]
    fn only_folders_of_glaves_that_are_gone_are_removed() {
        let dir = TempDir::new("run-temp-left").unwrap();
        let own = std::process::id();
        let named = [
            format!("glave-sandbox-{NO_PROCESS}-0"),
            format!("glave-sandbox-{NO_PROCESS}-12"),
            format!("glave-sandbox-{own}-0"),
            format!("glave-sandbox-{NO_PROCESS}"),
            format!("glave-sandbox-{NO_PROCESS}-x"),
            format!("other-{NO_PROCESS}-0"),
        ];
        for name in &named {
            std::fs::create_dir(dir.path().join(name)).unwrap();
        }
        std::fs::write(dir.path().join(&named[0]).join("left"), "x").unwrap();
        remove_left(dir.path(), "glave-sandbox-");
        let mut left: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        let mut kept = named[2..].to_vec();
        kept.sort();
        assert_eq!(left, kept);
    }
}
