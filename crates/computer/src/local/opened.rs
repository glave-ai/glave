//! Opening a file the path checks cannot be raced into escaping. A path is resolved and
//! checked first, but a command running beside the tools could swap a folder for a link to
//! somewhere else between that check and the open. So every file is opened first and then
//! checked: the handle must be the very file (same device and inode) the path resolves to
//! now, inside the workspace. Nothing is truncated or written before that check passes.

use std::path::Path;

use glave_ports::ComputerError;
use tokio::fs::{File, OpenOptions};

use super::paths::resolve;

/// Whether two handles' metadata describe one file: the same device and inode.
#[cfg(unix)]
fn same_file(one: &std::fs::Metadata, other: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    one.dev() == other.dev() && one.ino() == other.ino()
}

/// Other systems cannot tell, so the check fails closed.
#[cfg(not(unix))]
fn same_file(_: &std::fs::Metadata, _: &std::fs::Metadata) -> bool {
    false
}

/// How many names a file has: more than one means writing it changes it elsewhere too.
#[cfg(unix)]
fn names(metadata: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    metadata.nlink()
}

#[cfg(not(unix))]
fn names(_: &std::fs::Metadata) -> u64 {
    1
}

/// Opens `resolved` as `options` say, then checks the handle is still what `path` names
/// inside `root`. A file written must have no other hard links, since a link to a file
/// outside the workspace would let a write change it.
pub(crate) async fn checked(
    root: &Path,
    path: &str,
    resolved: &Path,
    options: &OpenOptions,
    writing: bool,
) -> Result<File, ComputerError> {
    let file = options.open(resolved).await.map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            ComputerError::NotFound(format!("{path}: no such file"))
        } else {
            ComputerError::Failed(format!("{path}: {error}"))
        }
    })?;
    let moved = || ComputerError::Refused(format!("{path} changed while it was being opened."));
    let held = file
        .metadata()
        .await
        .map_err(|error| ComputerError::Failed(format!("{path}: {error}")))?;
    let now = resolve(root, path).await?;
    let named = tokio::fs::metadata(&now).await.map_err(|_| moved())?;
    if !same_file(&held, &named) {
        return Err(moved());
    }
    if writing && !held.is_dir() && names(&held) > 1 {
        return Err(ComputerError::Refused(format!(
            "{path} has other hard links, so writing it could change a file outside the \
             workspace. Write a new file and move it into place instead."
        )));
    }
    Ok(file)
}

/// Whether `path`, as a walk found it under the workspace, is still that very file: no link
/// anywhere along it, and the same file that was read.
pub(crate) fn still_here(root: &Path, path: &Path, handle: &std::fs::File) -> bool {
    let (Ok(real), Ok(opened), Ok(now)) = (
        std::fs::canonicalize(path),
        handle.metadata(),
        std::fs::metadata(path),
    ) else {
        return false;
    };
    real == path && real.starts_with(root) && same_file(&opened, &now)
}
