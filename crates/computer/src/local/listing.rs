//! Listing a folder of a workspace: entry by entry through open folders, or the way git sees
//! it, with each entry the walk found looked up again with no link along the way.

use std::ffi::{OsStr, OsString};
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};

use glave_ports::computer::{ComputerError, Entry, EntryKind, Listed, Listing};
use rustix::fs::{AtFlags, Dir, FileType, Mode, openat, statat};

use super::anchored::{FOLDER, Found, Root, changed, count, not_a_folder, plain, resolve, shown};
use super::files::{blocking, io};
use super::paths::relative;
use super::walk;

/// The most entries one listing returns.
pub(crate) const MAX_ENTRIES: usize = 10_000;

pub(crate) async fn list(root: &Path, listing: &Listing) -> Result<Listed, ComputerError> {
    let (root, listing) = (root.to_owned(), listing.clone());
    blocking(move || {
        let anchor = Root::open(&root)?;
        let found = resolve(&anchor, &listing.path)?;
        if listing.skip_ignored {
            return discover(&anchor, &found, &listing);
        }
        let opened = open_folder(&anchor, &found, &listing.path)?;
        let (mut entries, truncated) = entries_below(&opened, &found.names, &listing)?;
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(Listed {
            entries,
            truncated,
            ignored: 0,
        })
    })
    .await
}

/// The folder `found` names, open.
fn open_folder(root: &Root<'_>, found: &Found, path: &str) -> Result<OwnedFd, ComputerError> {
    if found.stat.is_none() {
        return Err(ComputerError::NotFound(format!("{path}: no such file")));
    }
    if !found.is_folder() {
        return Err(not_a_folder(path));
    }
    match &found.name {
        None => root.fd.try_clone().map_err(|error| io(path, &error)),
        Some(name) => openat(&found.folder, name, FOLDER, Mode::empty()).map_err(|_| changed(path)),
    }
}

/// Every entry in `opened` (and below it, if recursive), read through open folders only, so a
/// folder swapped for a link mid-listing is never entered. Whether the listing was cut short.
fn entries_below(
    opened: &OwnedFd,
    base: &[OsString],
    listing: &Listing,
) -> Result<(Vec<Entry>, bool), ComputerError> {
    let failed =
        |error: rustix::io::Errno| ComputerError::Failed(format!("{}: {error}", listing.path));
    let mut entries = Vec::new();
    let mut pending: Vec<Vec<OsString>> = vec![Vec::new()];
    while let Some(below) = pending.pop() {
        let mut folder = opened.try_clone().map_err(|e| io(&listing.path, &e))?;
        for name in &below {
            folder =
                openat(&folder, name, FOLDER, Mode::empty()).map_err(|_| changed(&listing.path))?;
        }
        for entry in Dir::read_from(&folder).map_err(failed)? {
            let entry = entry.map_err(failed)?;
            let name = OsStr::from_bytes(entry.file_name().to_bytes());
            if name == "." || name == ".." {
                continue;
            }
            if entries.len() >= MAX_ENTRIES {
                return Ok((entries, true));
            }
            let Ok(stat) = statat(&folder, name, AtFlags::SYMLINK_NOFOLLOW) else {
                continue;
            };
            let kind = match FileType::from_raw_mode(stat.st_mode) {
                FileType::Symlink => EntryKind::Link,
                FileType::Directory => EntryKind::Folder,
                _ => EntryKind::File,
            };
            let mut deeper = below.clone();
            deeper.push(name.to_owned());
            let names: Vec<OsString> = base.iter().chain(&deeper).cloned().collect();
            // git's own records are listed but not descended into.
            if listing.recursive && kind == EntryKind::Folder && name != ".git" {
                pending.push(deeper);
            }
            entries.push(Entry {
                path: shown(&names),
                kind,
                size: if kind == EntryKind::File {
                    count(stat.st_size)
                } else {
                    0
                },
            });
        }
    }
    Ok((entries, false))
}

/// A listing the way git sees the folder: what it ignores is left out and counted. The walk
/// goes by path, so each entry it finds is looked up again from the root with no link along
/// the way (`plain`) before it is shown.
fn discover(root: &Root<'_>, found: &Found, listing: &Listing) -> Result<Listed, ComputerError> {
    let top = root.path.join(found.names.iter().collect::<PathBuf>());
    let (walked, ignored) = if listing.recursive {
        let files: Vec<(PathBuf, EntryKind)> = walk::files(&top)
            .take(MAX_ENTRIES.saturating_add(1))
            .map(|path| (path, EntryKind::File))
            .collect();
        (files, 0)
    } else {
        walk::entries(&top).map_err(|error| io(&listing.path, &error))?
    };
    let truncated = walked.len() > MAX_ENTRIES;
    let mut entries = Vec::with_capacity(walked.len().min(MAX_ENTRIES));
    for (path, kind) in walked.into_iter().take(MAX_ENTRIES) {
        let Some((_, _, stat)) = plain(root, &path) else {
            continue;
        };
        entries.push(Entry {
            path: relative(root.path, &path),
            kind,
            size: if kind == EntryKind::File {
                count(stat.st_size)
            } else {
                0
            },
        });
    }
    Ok(Listed {
        entries,
        truncated,
        ignored: u64::try_from(ignored).unwrap_or(u64::MAX),
    })
}
