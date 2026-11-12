//! The copy of the file kept beside it from before a migration that reshaped it
//! (`specs/storage.md`, "Migrations"): taken while the migration holds the file's write lock, and
//! let go of once a key is destroyed or a project key rotated, since it still holds what that key
//! sealed, or once it is a week old.

use std::io;
use std::path::{Path, PathBuf};

use glave_ports::Clock;
use jiff::{SignedDuration, Timestamp};
use rusqlite::Connection;

use crate::store::StoreError;

/// How long a copy is kept at most.
const KEPT_FOR: SignedDuration = SignedDuration::from_hours(7 * 24);

/// Where the copy of `file` is kept: beside it, one, each copy replacing the one before.
pub(crate) fn copy_of(file: &Path) -> PathBuf {
    let mut name = file.as_os_str().to_owned();
    name.push(".backup");
    PathBuf::from(name)
}

/// Copies the database at `file`, as it is, to [`copy_of`] it, written whole under another name
/// and then put in place, after removing what a copy stopped partway left. The caller holds the
/// file's write lock, so what is copied is what its migration starts from.
pub(crate) fn take(file: &Path) -> Result<(), StoreError> {
    let copy = copy_of(file);
    let taken = glave_ports::file::remove_unfinished(&copy).and_then(|()| {
        let source = Connection::open(file).map_err(io::Error::other)?;
        glave_ports::file::replace_with(&copy, |temporary| {
            let name = temporary.to_str().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{} is not UTF-8", temporary.display()),
                )
            })?;
            source
                .execute("VACUUM INTO ?1", [name])
                .map(drop)
                .map_err(io::Error::other)
        })
    });
    taken.map_err(|e| {
        StoreError::Schema(format!(
            "could not copy glave's data before bringing it up to date: {e}"
        ))
    })
}

/// Removes the copy at `copy`, if there is one.
pub(crate) fn forget(copy: &Path) -> io::Result<()> {
    match std::fs::remove_file(copy) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        removed => removed,
    }
}

/// Removes the copy at `copy` once `clock` says it is a week old, saying whether it did.
pub(crate) fn forget_if_old(copy: &Path, clock: &dyn Clock) -> io::Result<bool> {
    let taken = match std::fs::metadata(copy).and_then(|kept| kept.modified()) {
        Ok(taken) => Timestamp::try_from(taken).map_err(io::Error::other)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if clock.now().duration_since(taken) < KEPT_FOR {
        return Ok(false);
    }
    forget(copy).map(|()| true)
}
