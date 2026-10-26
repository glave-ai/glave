//! A test's own folder in the system temp folder, removed with everything in it, read-only
//! folders included, when the test is done with it.

use std::ops::Deref;
use std::path::{Path, PathBuf};

use crate::remove_tree;

/// The most tries at a name no other folder has.
const ATTEMPTS: u32 = 1_000;

/// A new, empty folder of a test's own, removed when dropped. It reads as the [`Path`] it is.
///
/// It sits alone in a folder made for it, `glave-<name>-<pid>-<n>/<name>`, so what a test
/// makes beside it (`dir.with_extension("other")`) is removed with it too.
pub struct TempDir {
    /// The folder made for it, which holds it: taken as it is removed, by
    /// [`TempDir::remove`] or the drop.
    outer: Option<PathBuf>,
    path: PathBuf,
}

impl TempDir {
    /// A folder called `name`, alone in a new folder in the system temp folder, made now. Its
    /// path is resolved (on macOS, `/private/var/…`), as the sandbox and git see it.
    pub fn new(name: &str) -> std::io::Result<Self> {
        Self::within(&std::fs::canonicalize(std::env::temp_dir())?, name)
    }

    /// Like [`TempDir::new`], but outside the system temp folder, which the sandbox lets every
    /// command write: in the workspace's `target/sandbox-tests`.
    pub fn outside_temp(name: &str) -> std::io::Result<Self> {
        let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sandbox-tests");
        std::fs::create_dir_all(&parent)?;
        Self::within(&std::fs::canonicalize(parent)?, name)
    }

    fn within(parent: &Path, name: &str) -> std::io::Result<Self> {
        let pid = std::process::id();
        for attempt in 0..ATTEMPTS {
            let outer = parent.join(format!("glave-{name}-{pid}-{attempt}"));
            match std::fs::create_dir(&outer) {
                Ok(()) => {
                    let path = outer.join(name);
                    // Owned from here on, so a failure below still removes `outer`.
                    let made = Self {
                        outer: Some(outer),
                        path,
                    };
                    std::fs::create_dir(&made.path)?;
                    return Ok(made);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("could not make a folder named glave-{name}-{pid}-<n>"),
        ))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Removes it now, with everything in it, saying why when it cannot: what a test started
    /// may still be writing there. Dropping it removes it too, but cannot fail the test.
    pub fn remove(mut self) -> std::io::Result<()> {
        let Some(outer) = self.outer.take() else {
            return Ok(());
        };
        remove_tree(&outer).map_err(|error| {
            let said = format!("could not remove {}: {error}", outer.display());
            std::io::Error::new(error.kind(), said)
        })
    }
}

/// Shown as its path, which is what it reads as.
impl std::fmt::Debug for TempDir {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.path.fmt(f)
    }
}

impl Deref for TempDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<Path> for TempDir {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

/// So it is an argument for a program (`Command::arg`), and `PathBuf::from(&dir)` works.
impl AsRef<std::ffi::OsStr> for TempDir {
    fn as_ref(&self) -> &std::ffi::OsStr {
        self.path.as_os_str()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        // Best effort: a drop cannot fail the test. One that must know calls `remove`.
        if let Some(outer) = self.outer.take() {
            let _ = remove_tree(&outer);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TempDir;

    #[test]
    fn each_is_new_and_goes_with_what_it_and_its_neighbours_hold() {
        let first = TempDir::new("temp-test").unwrap();
        let second = TempDir::new("temp-test").unwrap();
        assert_ne!(first.path(), second.path());
        assert!(first.is_dir());
        std::fs::create_dir_all(first.join("a/b")).unwrap();
        std::fs::write(first.join("a/b/file"), "x").unwrap();
        let beside = first.with_extension("other");
        std::fs::create_dir(&beside).unwrap();
        let (kept, other) = (first.to_path_buf(), second.to_path_buf());
        drop(first);
        second.remove().unwrap();
        assert!(!kept.exists() && !beside.exists() && !other.exists());
    }

    #[test]
    fn a_folder_it_cannot_remove_is_said() {
        let dir = TempDir::new("temp-test").unwrap();
        let outer = dir.outer.clone().unwrap();
        // A file where its folder was, which no removal of a folder takes.
        crate::remove_tree(&outer).unwrap();
        std::fs::write(&outer, "not a folder").unwrap();
        let error = dir.remove().unwrap_err();
        std::fs::remove_file(&outer).unwrap();
        let said = format!("could not remove {}: ", outer.display());
        assert!(error.to_string().starts_with(&said), "{error}");
    }
}
