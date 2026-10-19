//! The few files glave keeps on this machine rather than in the store (`specs/invariants.md`,
//! "Where state lives"), written so a crash leaves either no file or a whole one, never part of
//! one: each is written to a temporary file beside it, flushed to disk, put in place in one
//! step, and its folder flushed so the new name lasts. Each is readable only by the person.

use std::fs::{File, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use crate::env::{Entropy as _, OsEntropy};

/// The extension of a temporary file being written, before it is put in place. One a crash
/// left is never read as the file it was for.
pub const WRITING: &str = "writing";

/// Creates the file at `path` holding `contents`, for a file written once and then only read
/// (the root key, this computer's id). Fails with [`io::ErrorKind::AlreadyExists`], leaving it
/// as it is, when there is one already, so two glaves creating it at once agree on one.
pub fn create(path: &Path, contents: &[u8]) -> io::Result<()> {
    write(path, contents, Place::New, Step::Flushed)
}

/// Writes the file at `path` holding `contents`, replacing any there: a reader sees the old
/// file or the new one, whole.
pub fn replace(path: &Path, contents: &[u8]) -> io::Result<()> {
    write(path, contents, Place::Over, Step::Flushed)
}

/// Where a test stops [`create`], as a crash there would.
#[cfg(any(test, feature = "conformance"))]
#[derive(Debug, Clone, Copy)]
pub enum Stop {
    /// The temporary file is written and flushed, not yet put in place.
    Written,
    /// The file is in place, its folder not yet flushed.
    Placed,
}

/// [`create`], stopped at `stop`, leaving what a crash there would.
#[cfg(any(test, feature = "conformance"))]
pub fn create_until(path: &Path, contents: &[u8], stop: Stop) -> io::Result<()> {
    let until = match stop {
        Stop::Written => Step::Written,
        Stop::Placed => Step::Placed,
    };
    write(path, contents, Place::New, until)
}

/// Whether a write may replace a file already there.
#[derive(Clone, Copy)]
enum Place {
    New,
    Over,
}

/// How far a write goes, in order.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Written,
    Placed,
    Flushed,
}

fn write(path: &Path, contents: &[u8], place: Place, until: Step) -> io::Result<()> {
    let temporary = temporary_for(path)?;
    if let Err(error) = write_temporary(&temporary, contents) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    if until == Step::Written {
        return Ok(());
    }
    let placed = match place {
        // A link fails when the name is taken, where a rename would replace the file there.
        Place::New => std::fs::hard_link(&temporary, path),
        Place::Over => std::fs::rename(&temporary, path),
    };
    // Gone after a rename; after a link, only a second name for the file, or nothing placed.
    let _ = std::fs::remove_file(&temporary);
    placed?;
    if until == Step::Placed {
        return Ok(());
    }
    File::open(folder_of(path))?.sync_all()
}

/// A new name beside `path` for writing it: hidden, random, and ending in [`WRITING`].
fn temporary_for(path: &Path) -> io::Result<PathBuf> {
    let name = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} names no file", path.display()),
        )
    })?;
    let random = OsEntropy.bytes().map_err(io::Error::other)?;
    let suffix = random.iter().fold(String::new(), |mut suffix, byte| {
        use std::fmt::Write as _;
        let _ = write!(suffix, "{byte:02x}");
        suffix
    });
    let mut temporary = std::ffi::OsString::from(".");
    temporary.push(name);
    temporary.push(format!(".{suffix}.{WRITING}"));
    Ok(path.with_file_name(temporary))
}

/// Writes `contents` to a new file at `path`, private to the person, and flushes it to disk.
fn write_temporary(path: &Path, contents: &[u8]) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(contents)?;
    file.sync_all()
}

/// The folder `path` is in.
fn folder_of(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("glave-file-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The names in `dir`, sorted.
    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn a_created_file_is_whole_private_and_never_replaced() {
        let dir = folder("create");
        let path = dir.join("root.key");
        create(&path, b"first").unwrap();
        let again = create(&path, b"second").unwrap_err();
        assert_eq!(again.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        #[cfg(unix)]
        assert_eq!(mode(&path), 0o600);
        assert_eq!(names(&dir), ["root.key"], "no temporary file is left");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_replaced_file_is_whole_private_and_the_new_one() {
        let dir = folder("replace");
        let path = dir.join("serve.token");
        replace(&path, b"first").unwrap();
        replace(&path, b"second").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        #[cfg(unix)]
        assert_eq!(mode(&path), 0o600);
        assert_eq!(names(&dir), ["serve.token"], "no temporary file is left");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_create_stopped_at_any_step_leaves_no_file_or_a_whole_one() {
        let dir = folder("stopped");
        let path = dir.join("computer");
        create_until(&path, b"never placed", Stop::Written).unwrap();
        assert!(!path.exists(), "a temporary file is never the file");
        let left = names(&dir);
        assert_eq!(left.len(), 1);
        assert!(left[0].starts_with(".computer.") && left[0].ends_with(".writing"));
        // The next start creates it as if nothing had happened.
        create(&path, b"whole").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"whole");

        let placed = dir.join("placed");
        create_until(&placed, b"whole too", Stop::Placed).unwrap();
        assert_eq!(std::fs::read(&placed).unwrap(), b"whole too");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_path_that_names_no_file_is_refused() {
        let refused = replace(Path::new("/"), b"x").unwrap_err();
        assert_eq!(refused.kind(), io::ErrorKind::InvalidInput);
    }
}
