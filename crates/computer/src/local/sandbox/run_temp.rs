//! One sandboxed command's own temp folder (its `TMPDIR`), made when it starts and removed,
//! with everything the command left in it, once it ends however it ends.

use std::path::{Path, PathBuf};

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
