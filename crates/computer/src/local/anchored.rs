//! Paths resolved inside a workspace one name at a time, from an open handle on its root.
//! Each name is looked up in the folder before it, already open, and never followed if it is
//! a link: the link is read and its target resolved the same way, confined to the root, with
//! `..` going back to a folder already open rather than to whatever `..` names now. So a
//! command that swaps a folder for a link while a path is resolved can make the lookup fail,
//! but never lead it outside. This is the design of cap-std and of Linux's `RESOLVE_BENEATH`,
//! done by hand with `openat` and `O_NOFOLLOW` so it works the same on macOS.

use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Component, Path, PathBuf};

use glave_ports::ComputerError;
use rustix::fs::{AtFlags, FileType, Mode, OFlags, Stat, fstat, openat, readlinkat, statat};
use rustix::io::Errno;

/// How many links one path may go through, as Linux allows.
const MAX_LINKS: usize = 40;

/// How a folder along a path is opened: never through a link.
pub(crate) const FOLDER: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

/// The workspace root, open.
pub(crate) struct Root<'a> {
    pub(crate) path: &'a Path,
    pub(crate) fd: OwnedFd,
}

impl<'a> Root<'a> {
    pub(crate) fn open(path: &'a Path) -> Result<Self, ComputerError> {
        let fd = rustix::fs::open(path, FOLDER, Mode::empty()).map_err(|error| {
            ComputerError::Failed(format!(
                "the workspace {} cannot be opened: {error}",
                path.display()
            ))
        })?;
        Ok(Self { path, fd })
    }
}

/// Where a path resolved inside the root.
pub(crate) struct Found {
    /// The deepest existing folder on the way: the one holding the last name, unless
    /// `missing` folders come between.
    pub(crate) folder: OwnedFd,
    /// Folders that do not exist yet between `folder` and the last name.
    pub(crate) missing: Vec<OsString>,
    /// The last name; none for the root itself.
    pub(crate) name: Option<OsString>,
    /// Every name from the root, links resolved.
    pub(crate) names: Vec<OsString>,
    /// What is there, never a link; none when nothing is there yet.
    pub(crate) stat: Option<Stat>,
}

impl Found {
    /// Whether the path names a folder (the root included).
    pub(crate) fn is_folder(&self) -> bool {
        self.stat
            .as_ref()
            .is_some_and(|stat| FileType::from_raw_mode(stat.st_mode) == FileType::Directory)
    }
}

/// One name still to look up, and whether it came from a link's target.
type Piece = (OsString, bool);

fn pieces(path: &Path, from_link: bool) -> Vec<Piece> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some((name.to_owned(), from_link)),
            Component::ParentDir => Some(("..".into(), from_link)),
            Component::CurDir | Component::RootDir | Component::Prefix(_) => None,
        })
        .collect()
}

pub(crate) fn outside(path: &str, root: &Root<'_>) -> ComputerError {
    ComputerError::Refused(format!(
        "{path} is outside the workspace ({}).",
        root.path.display()
    ))
}

fn failed(path: &str, error: Errno) -> ComputerError {
    ComputerError::Failed(format!("{path}: {error}"))
}

/// A lookup that failed because what was there a moment ago is not what is there now.
fn lost(path: &str, error: Errno) -> ComputerError {
    if matches!(
        error,
        Errno::LOOP | Errno::NOTDIR | Errno::INVAL | Errno::NOENT
    ) {
        changed(path)
    } else {
        failed(path, error)
    }
}

pub(crate) fn changed(path: &str) -> ComputerError {
    ComputerError::Refused(format!(
        "{path} changed while it was being opened; try again."
    ))
}

/// `path` relative to the root, as written (`..` kept). An absolute path must lead into the
/// root; the part that exists is made canonical first, so `/var` and `/private/var` agree.
fn relative_to(root: &Root<'_>, path: &Path, shown: &str) -> Result<PathBuf, ComputerError> {
    if path.is_relative() {
        return Ok(path.to_owned());
    }
    if let Ok(rest) = path.strip_prefix(root.path) {
        return Ok(rest.to_owned());
    }
    for ancestor in path.ancestors() {
        let Ok(real) = std::fs::canonicalize(ancestor) else {
            continue;
        };
        let (Ok(inside), Ok(rest)) = (real.strip_prefix(root.path), path.strip_prefix(ancestor))
        else {
            return Err(outside(shown, root));
        };
        return Ok(inside.join(rest));
    }
    Err(outside(shown, root))
}

/// Resolves `path` (relative to the root, or absolute inside it). Links inside the root are
/// followed; anything leading outside, a link to nothing, or a loop is refused.
pub(crate) fn resolve(root: &Root<'_>, path: &str) -> Result<Found, ComputerError> {
    if path.trim().is_empty() {
        return Err(ComputerError::Refused("The path is empty.".into()));
    }
    let asked = relative_to(root, Path::new(path), path)?;
    let mut queue: VecDeque<Piece> = pieces(&asked, false).into();
    let mut folders: Vec<(OwnedFd, OsString)> = Vec::new();
    let mut links = 0usize;
    while let Some((name, from_link)) = queue.pop_front() {
        if name == ".." {
            if folders.pop().is_none() {
                return Err(outside(path, root));
            }
            continue;
        }
        let here = current(root, &folders);
        let stat = match statat(here, &name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => stat,
            Err(Errno::NOENT) => {
                return not_there(path, here, &folders, (name, from_link), queue);
            }
            Err(error) => return Err(failed(path, error)),
        };
        match FileType::from_raw_mode(stat.st_mode) {
            FileType::Symlink => {
                links = links.saturating_add(1);
                if links > MAX_LINKS {
                    return Err(ComputerError::Refused(format!(
                        "{path} goes through too many links (a loop?)."
                    )));
                }
                let target = link_target(root, here, &name, path)?;
                if target.is_absolute() {
                    folders.clear();
                }
                let target = relative_to(root, &target, path)?;
                for piece in pieces(&target, true).into_iter().rev() {
                    queue.push_front(piece);
                }
            }
            FileType::Directory if !queue.is_empty() => {
                let opened =
                    openat(here, &name, FOLDER, Mode::empty()).map_err(|e| lost(path, e))?;
                folders.push((opened, name));
            }
            _ if queue.is_empty() => {
                let folder = here.try_clone_to_owned().map_err(|e| io_failed(path, &e))?;
                let names = names_of(&folders, Some(&name));
                return Ok(Found {
                    folder,
                    missing: Vec::new(),
                    name: Some(name),
                    names,
                    stat: Some(stat),
                });
            }
            _ => {
                return Err(ComputerError::Failed(format!(
                    "{path}: {} is a file, not a folder",
                    Path::new(&name).display()
                )));
            }
        }
    }
    // Ended on a folder already open (`.`, `..` or a link to a folder).
    let names = names_of(&folders, None);
    let last = folders.pop();
    let folder = current(root, &folders)
        .try_clone_to_owned()
        .map_err(|e| io_failed(path, &e))?;
    let (name, stat) = match last {
        Some((fd, name)) => (Some(name), fstat(&fd).map_err(|e| failed(path, e))?),
        None => (None, fstat(&root.fd).map_err(|e| failed(path, e))?),
    };
    Ok(Found {
        folder,
        missing: Vec::new(),
        name,
        names,
        stat: Some(stat),
    })
}

/// Where the link `name` in `here` points, as written.
fn link_target(
    root: &Root<'_>,
    here: BorrowedFd<'_>,
    name: &OsStr,
    path: &str,
) -> Result<PathBuf, ComputerError> {
    let target = readlinkat(here, name, Vec::new()).map_err(|e| lost(path, e))?;
    if target.as_bytes().is_empty() {
        return Err(outside(path, root));
    }
    Ok(PathBuf::from(OsStr::from_bytes(target.as_bytes())))
}

/// The answer when `name` does not exist in `here`: what is left must be plain names, made
/// as folders on writing, and the gap must not be a link's target.
fn not_there(
    path: &str,
    here: BorrowedFd<'_>,
    folders: &[(OwnedFd, OsString)],
    (name, from_link): Piece,
    queue: VecDeque<Piece>,
) -> Result<Found, ComputerError> {
    if from_link {
        return Err(ComputerError::Refused(format!(
            "{path} goes through a link to something that does not exist, so it cannot be \
             checked to stay inside the workspace."
        )));
    }
    let mut rest: Vec<OsString> = Vec::with_capacity(queue.len().saturating_add(1));
    rest.push(name);
    for (piece, _) in queue {
        if piece == ".." {
            return Err(ComputerError::Refused(format!(
                "{path} uses '..' or '.' through a folder that does not exist."
            )));
        }
        rest.push(piece);
    }
    let name = rest.pop();
    let mut names = names_of(folders, None);
    names.extend(rest.iter().cloned());
    names.extend(name.iter().cloned());
    Ok(Found {
        folder: here.try_clone_to_owned().map_err(|e| io_failed(path, &e))?,
        missing: rest,
        name,
        names,
        stat: None,
    })
}

fn current<'a>(root: &'a Root<'_>, folders: &'a [(OwnedFd, OsString)]) -> BorrowedFd<'a> {
    folders
        .last()
        .map_or_else(|| root.fd.as_fd(), |(fd, _)| fd.as_fd())
}

fn names_of(folders: &[(OwnedFd, OsString)], last: Option<&OsString>) -> Vec<OsString> {
    folders
        .iter()
        .map(|(_, name)| name.clone())
        .chain(last.cloned())
        .collect()
}

fn io_failed(path: &str, error: &std::io::Error) -> ComputerError {
    ComputerError::Failed(format!("{path}: {error}"))
}

/// A path a walk found under the root, looked up again with no link anywhere along it: the
/// folder holding it, open, its name and what is there (a link is reported, not followed).
/// None when it is gone, outside the root, or reached through a link now: whatever a walk
/// was shown while a folder was swapped for a link, only names really inside come through.
pub(crate) fn plain(root: &Root<'_>, path: &Path) -> Option<(OwnedFd, OsString, Stat)> {
    let relative = path.strip_prefix(root.path).ok()?;
    let mut names = Vec::new();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return None;
        };
        names.push(name);
    }
    let (last, folders) = names.split_last()?;
    let mut here = root.fd.try_clone().ok()?;
    for name in folders {
        here = openat(&here, *name, FOLDER, Mode::empty()).ok()?;
    }
    let stat = statat(&here, *last, AtFlags::SYMLINK_NOFOLLOW).ok()?;
    Some((here, (*last).to_owned(), stat))
}

/// A regular file in `folder`, opened to read, never through a link and never blocking on a
/// pipe a command left there.
pub(crate) fn open_to_read(folder: BorrowedFd<'_>, name: &OsStr) -> std::io::Result<std::fs::File> {
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    let fd = openat(folder, name, flags, Mode::empty())?;
    if FileType::from_raw_mode(fstat(&fd)?.st_mode) != FileType::RegularFile {
        return Err(std::io::Error::other("not a regular file"));
    }
    Ok(std::fs::File::from(fd))
}

/// A number from a `stat` field, whatever its type on this system; negative is zero.
pub(crate) fn count(value: impl Into<i128>) -> u64 {
    u64::try_from(value.into()).unwrap_or(0)
}

/// A path's names joined with `/`, `.` for the root.
pub(crate) fn shown(names: &[OsString]) -> String {
    if names.is_empty() {
        return ".".into();
    }
    names
        .iter()
        .map(|name| name.to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}
