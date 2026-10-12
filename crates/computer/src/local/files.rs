//! Reading, writing and describing files in a workspace, confined to its root (every path
//! resolved from an open handle on it, `anchored`) and bounded in size.

use std::io::Read as _;
use std::os::fd::AsFd as _;
use std::path::{Path, PathBuf};

use glave_ports::{
    Bytes, ComputerError, EntryKind, FileWrite, Stat, digest, is_protected, refuse_protected,
};
use rustix::fs::FileType;

use super::anchored::{Found, Root, count, open_to_read, resolve, shown};
use super::replace::{make_folders, replace};

/// The largest file read or written in one call, so it fits in one message on the wire.
pub(crate) const MAX_FILE: u64 = 20 * 1024 * 1024;

pub(crate) fn io(path: &str, error: &std::io::Error) -> ComputerError {
    if error.kind() == std::io::ErrorKind::NotFound {
        ComputerError::NotFound(format!("{path}: no such file"))
    } else if error.kind() == std::io::ErrorKind::StorageFull {
        // Files are replaced whole, so a write that did not fit left the old one as it was.
        ComputerError::Failed(format!(
            "{path}: the computer's disk is full ({error}); {path} is as it was"
        ))
    } else {
        ComputerError::Failed(format!("{path}: {error}"))
    }
}

/// Runs file work off the async threads: it is a run of blocking system calls.
pub(crate) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, ComputerError> + Send + 'static,
) -> Result<T, ComputerError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| ComputerError::Failed(format!("file work stopped: {error}")))?
}

pub(crate) async fn read(root: &Path, path: &str) -> Result<Bytes, ComputerError> {
    let (root, path) = (root.to_owned(), path.to_owned());
    blocking(move || {
        let root = Root::open(&root)?;
        let found = resolve(&root, &path)?;
        read_found(&path, &found).map(Bytes::new)
    })
    .await
}

/// The whole content of the file `found` names.
fn read_found(path: &str, found: &Found) -> Result<Vec<u8>, ComputerError> {
    if found.is_folder() {
        return Err(ComputerError::Refused(format!(
            "{path} is a folder; list it instead"
        )));
    }
    let (Some(name), Some(_)) = (&found.name, &found.stat) else {
        return Err(ComputerError::NotFound(format!("{path}: no such file")));
    };
    let file = open_to_read(found.folder.as_fd(), name).map_err(|error| io(path, &error))?;
    let size = file.metadata().map_err(|error| io(path, &error))?.len();
    if size > MAX_FILE {
        return Err(ComputerError::Refused(format!(
            "{path} is {size} bytes; files over {MAX_FILE} bytes are not read whole"
        )));
    }
    let mut content = Vec::new();
    file.take(MAX_FILE)
        .read_to_end(&mut content)
        .map_err(|error| io(path, &error))?;
    Ok(content)
}

/// Checks every path first, so a refused path means nothing was written. Each file is
/// replaced whole (`replace`), never written in place, through the folders its path resolved
/// to; a file that asks to be protected is refused if those folders lead to a protected path.
pub(crate) async fn write(root: &Path, files: &[FileWrite]) -> Result<(), ComputerError> {
    let (root, files) = (root.to_owned(), files.to_vec());
    blocking(move || {
        let root = Root::open(&root)?;
        let mut resolved = Vec::with_capacity(files.len());
        for file in &files {
            let size = u64::try_from(file.content.as_slice().len()).unwrap_or(u64::MAX);
            if size > MAX_FILE {
                return Err(ComputerError::Refused(format!(
                    "{} is {size} bytes; files over {MAX_FILE} bytes are not written whole",
                    file.path
                )));
            }
            let found = resolve(&root, &file.path)?;
            if file.protect
                && is_protected(
                    root.path,
                    &root.path.join(found.names.iter().collect::<PathBuf>()),
                )
            {
                // The names are those of the folders already open, which the write goes
                // through: no link swapped in later can change where it lands.
                return Err(refuse_protected(&shown(&found.names)));
            }
            if found.is_folder() || found.name.is_none() {
                return Err(ComputerError::Refused(format!(
                    "{} is a folder, not a file.",
                    file.path
                )));
            }
            resolved.push(found);
        }
        for (file, found) in files.iter().zip(resolved) {
            let Some(name) = found.name else {
                continue;
            };
            let folder =
                make_folders(found.folder, &found.missing).map_err(|e| io(&file.path, &e))?;
            replace(folder.as_fd(), &name, file.content.as_slice())
                .map_err(|error| io(&file.path, &error))?;
        }
        Ok(())
    })
    .await
}

/// What is at `path`: nothing, a folder, or a file with its size and, when small enough to
/// read whole, its digest.
pub(crate) async fn stat(root: &Path, path: &str) -> Result<Stat, ComputerError> {
    let (root, path) = (root.to_owned(), path.to_owned());
    blocking(move || {
        let root = Root::open(&root)?;
        let found = resolve(&root, &path)?;
        let shown = shown(&found.names);
        let Some(stat) = found.stat.as_ref() else {
            return Ok(Stat {
                path: shown,
                kind: None,
                size: 0,
                digest: None,
            });
        };
        if found.is_folder() {
            return Ok(Stat {
                path: shown,
                kind: Some(EntryKind::Folder),
                size: 0,
                digest: None,
            });
        }
        let size = count(stat.st_size);
        let regular = FileType::from_raw_mode(stat.st_mode) == FileType::RegularFile;
        let digest = (regular && size <= MAX_FILE)
            .then(|| read_found(&path, &found).ok())
            .flatten()
            .map(|bytes| digest(&bytes));
        Ok(Stat {
            path: shown,
            kind: Some(EntryKind::File),
            size,
            digest,
        })
    })
    .await
}
