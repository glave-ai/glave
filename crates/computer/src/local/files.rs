//! Reading, writing and listing files in a workspace, confined to its root and bounded in
//! size.

use std::path::{Path, PathBuf};

use glave_ports::{
    Bytes, ComputerError, Entry, EntryKind, FileWrite, Listed, Listing, Stat, digest,
};

use super::git::HISTORY;
use super::paths::{relative, resolve};
use super::walk;

/// The largest file read or written in one call, so it fits in one message on the wire.
pub(crate) const MAX_FILE: u64 = 20 * 1024 * 1024;
/// The most entries one listing returns.
pub(crate) const MAX_ENTRIES: usize = 10_000;

fn io(path: &str, error: &std::io::Error) -> ComputerError {
    if error.kind() == std::io::ErrorKind::NotFound {
        ComputerError::NotFound(format!("{path}: no such file"))
    } else {
        ComputerError::Failed(format!("{path}: {error}"))
    }
}

pub(crate) async fn read(root: &Path, path: &str) -> Result<Bytes, ComputerError> {
    let resolved = resolve(root, path).await?;
    let metadata = tokio::fs::metadata(&resolved)
        .await
        .map_err(|error| io(path, &error))?;
    if metadata.is_dir() {
        return Err(ComputerError::Refused(format!(
            "{path} is a folder; list it instead"
        )));
    }
    if metadata.len() > MAX_FILE {
        return Err(ComputerError::Refused(format!(
            "{path} is {} bytes; files over {MAX_FILE} bytes are not read whole",
            metadata.len()
        )));
    }
    tokio::fs::read(&resolved)
        .await
        .map(Bytes::new)
        .map_err(|error| io(path, &error))
}

/// Checks every path first, so a refused path means nothing was written.
pub(crate) async fn write(root: &Path, files: &[FileWrite]) -> Result<(), ComputerError> {
    let mut resolved = Vec::with_capacity(files.len());
    for file in files {
        let size = u64::try_from(file.content.as_slice().len()).unwrap_or(u64::MAX);
        if size > MAX_FILE {
            return Err(ComputerError::Refused(format!(
                "{} is {size} bytes; files over {MAX_FILE} bytes are not written whole",
                file.path
            )));
        }
        resolved.push(resolve(root, &file.path).await?);
    }
    for (file, path) in files.iter().zip(resolved) {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| io(&file.path, &error))?;
        }
        tokio::fs::write(&path, file.content.as_slice())
            .await
            .map_err(|error| io(&file.path, &error))?;
    }
    Ok(())
}

/// What is at `path`: nothing, a folder, or a file with its size and, when small enough to
/// read whole, its digest.
pub(crate) async fn stat(root: &Path, path: &str) -> Result<Stat, ComputerError> {
    let resolved = resolve(root, path).await?;
    let shown = relative(root, &resolved);
    let Ok(metadata) = tokio::fs::metadata(&resolved).await else {
        return Ok(Stat {
            path: shown,
            kind: None,
            size: 0,
            digest: None,
        });
    };
    if metadata.is_dir() {
        return Ok(Stat {
            path: shown,
            kind: Some(EntryKind::Folder),
            size: 0,
            digest: None,
        });
    }
    let digest = if metadata.len() <= MAX_FILE {
        tokio::fs::read(&resolved)
            .await
            .ok()
            .map(|bytes| digest(&bytes))
    } else {
        None
    };
    Ok(Stat {
        path: shown,
        kind: Some(EntryKind::File),
        size: metadata.len(),
        digest,
    })
}

pub(crate) async fn list(root: &Path, listing: &Listing) -> Result<Listed, ComputerError> {
    let start = resolve(root, &listing.path).await?;
    if listing.skip_ignored {
        return discover(root, start, listing).await;
    }
    let mut entries = Vec::new();
    let mut folders = vec![start];
    let mut truncated = false;
    while let Some(folder) = folders.pop() {
        let mut reading = tokio::fs::read_dir(&folder)
            .await
            .map_err(|error| io(&listing.path, &error))?;
        while let Some(found) = reading
            .next_entry()
            .await
            .map_err(|error| io(&listing.path, &error))?
        {
            if entries.len() >= MAX_ENTRIES {
                truncated = true;
                break;
            }
            let path = found.path();
            let metadata = tokio::fs::symlink_metadata(&path)
                .await
                .map_err(|error| io(&listing.path, &error))?;
            let kind = if metadata.is_symlink() {
                EntryKind::Link
            } else if metadata.is_dir() {
                EntryKind::Folder
            } else {
                EntryKind::File
            };
            if listing.recursive && kind == EntryKind::Folder && !is_history(root, &path) {
                folders.push(path.clone());
            }
            entries.push(Entry {
                path: relative(root, &path),
                kind,
                size: if kind == EntryKind::File {
                    metadata.len()
                } else {
                    0
                },
            });
        }
        if truncated {
            break;
        }
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Listed {
        entries,
        truncated,
        ignored: 0,
    })
}

/// A listing the way git sees the folder: what it ignores is left out and counted.
async fn discover(root: &Path, start: PathBuf, listing: &Listing) -> Result<Listed, ComputerError> {
    let shown = listing.path.clone();
    let recursive = listing.recursive;
    let walked = tokio::task::spawn_blocking(move || {
        if recursive {
            let entries: Vec<(PathBuf, EntryKind)> = walk::files(&start)
                .take(MAX_ENTRIES.saturating_add(1))
                .map(|path| (path, EntryKind::File))
                .collect();
            Ok((entries, 0))
        } else {
            walk::entries(&start)
        }
    })
    .await
    .map_err(|error| ComputerError::Failed(format!("{shown}: {error}")))?
    .map_err(|error| io(&listing.path, &error))?;
    let (found, ignored) = walked;
    let truncated = found.len() > MAX_ENTRIES;
    let mut entries = Vec::with_capacity(found.len().min(MAX_ENTRIES));
    for (path, kind) in found.into_iter().take(MAX_ENTRIES) {
        let size = if kind == EntryKind::File {
            tokio::fs::symlink_metadata(&path)
                .await
                .map_or(0, |metadata| metadata.len())
        } else {
            0
        };
        entries.push(Entry {
            path: relative(root, &path),
            kind,
            size,
        });
    }
    Ok(Listed {
        entries,
        truncated,
        ignored: u64::try_from(ignored).unwrap_or(u64::MAX),
    })
}

/// Folders a recursive listing does not descend into: git's own records.
fn is_history(root: &Path, path: &Path) -> bool {
    path.file_name().is_some_and(|name| name == ".git") || path == root.join(HISTORY)
}

/// Every file under `root` a save would record, and their total size, stopping past `cap`.
pub(crate) async fn total_size(root: &Path, cap: u64) -> Result<u64, ComputerError> {
    let mut total = 0u64;
    let mut folders: Vec<PathBuf> = vec![root.to_owned()];
    while let Some(folder) = folders.pop() {
        let mut reading = tokio::fs::read_dir(&folder)
            .await
            .map_err(|error| io(&folder.to_string_lossy(), &error))?;
        while let Some(found) = reading
            .next_entry()
            .await
            .map_err(|error| io(&folder.to_string_lossy(), &error))?
        {
            let path = found.path();
            let Ok(metadata) = tokio::fs::symlink_metadata(&path).await else {
                continue;
            };
            if metadata.is_dir() {
                if !is_history(root, &path) {
                    folders.push(path);
                }
            } else {
                total = total.saturating_add(metadata.len());
                if total > cap {
                    return Ok(total);
                }
            }
        }
    }
    Ok(total)
}
