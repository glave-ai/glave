//! The sandbox's folder in the system temp folder: one private folder per user that every
//! glave sandbox keeps read-only, and in it one folder per prepared sandbox, holding each
//! command's own temp folder while it runs (`TMPDIR`) and the empty, unreadable file and
//! folder bubblewrap lays over what commands may not see. No sandboxed command can change those, so none can
//! make another's hiding show something else.

use std::fs::{DirBuilder, File, Permissions};
use std::io::{Error, ErrorKind};
use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

/// One prepared sandbox's folder, removed when dropped.
#[derive(Debug)]
pub(super) struct RunFolder {
    base: PathBuf,
    run: PathBuf,
}

impl RunFolder {
    /// Makes a new one under the system temp folder. Blocking.
    pub(super) fn new() -> std::io::Result<Self> {
        let system = std::fs::canonicalize(std::env::temp_dir())?;
        let uid = rustix::process::getuid().as_raw();
        let base = system.join(format!("glave-sandbox-{uid}"));
        private(&base, uid)?;
        let pid = std::process::id();
        let run = (0_u32..1_000)
            .map(|attempt| base.join(format!("{pid}-{attempt}")))
            .find(|run| DirBuilder::new().mode(0o700).create(run).is_ok())
            .ok_or_else(|| Error::other("could not make a folder for the sandbox"))?;
        let folder = Self { base, run };
        File::create(folder.empty_file())?;
        std::fs::set_permissions(folder.empty_file(), Permissions::from_mode(0o000))?;
        DirBuilder::new()
            .mode(0o000)
            .create(folder.empty_folder())?;
        Ok(folder)
    }

    /// The folder every glave sandbox of this user keeps read-only.
    pub(super) fn base(&self) -> &Path {
        &self.base
    }

    /// Where its commands' own temp folders go.
    pub(super) fn run(&self) -> &Path {
        &self.run
    }

    /// An empty file nobody may read or write, laid over hidden files.
    pub(super) fn empty_file(&self) -> PathBuf {
        self.run.join("empty")
    }

    /// An empty folder nobody may list or enter, laid over hidden folders.
    pub(super) fn empty_folder(&self) -> PathBuf {
        self.run.join("empty.d")
    }
}

impl Drop for RunFolder {
    fn drop(&mut self) {
        // Best effort; the unreadable empty folder is made writable first, as everything is.
        let _ = glave_ports::remove_tree(&self.run);
    }
}

/// Makes `base` this user's private folder, or checks that it already is: a folder, not a
/// link, owned by `uid`, that no one else may enter.
fn private(base: &Path, uid: u32) -> std::io::Result<()> {
    match DirBuilder::new().mode(0o700).create(base) {
        Ok(()) => return Ok(()),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let metadata = std::fs::symlink_metadata(base)?;
    if metadata.is_dir() && metadata.uid() == uid && metadata.mode().trailing_zeros() >= 6 {
        return Ok(());
    }
    Err(Error::new(
        ErrorKind::PermissionDenied,
        format!(
            "{} is not this user's private folder; remove it and try again",
            base.display()
        ),
    ))
}
