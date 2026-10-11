//! Writing a file by replacing it whole: the content goes to a new file beside it, is synced,
//! and is renamed over the old name. The old file is never opened for writing, so a hard link
//! in the workspace to a file outside it leaves that file as it was (the link's name now
//! points at the new file), and a reader never sees half a file. The new file keeps the old
//! one's permissions.

use std::ffi::OsStr;
use std::io::Write as _;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};

use rustix::fs::{
    AtFlags, FileType, Mode, OFlags, fchmod, fstat, mkdirat, openat, renameat, unlinkat,
};
use rustix::io::Errno;

use super::anchored::FOLDER;

/// Opens (making it if needed) each folder in `names` below `folder`, never through a link.
pub(crate) fn make_folders(
    folder: OwnedFd,
    names: &[std::ffi::OsString],
) -> std::io::Result<OwnedFd> {
    let mut here = folder;
    for name in names {
        match mkdirat(&here, name, Mode::from_raw_mode(0o777)) {
            Ok(()) | Err(Errno::EXIST) => {}
            Err(error) => return Err(error.into()),
        }
        here = openat(&here, name, FOLDER, Mode::empty())?;
    }
    Ok(here)
}

/// Replaces the file `name` in `folder` with `content`, or creates it.
pub(crate) fn replace(folder: BorrowedFd<'_>, name: &OsStr, content: &[u8]) -> std::io::Result<()> {
    let mode = existing_mode(folder, name)?;
    let temp = temp_name();
    let flags = OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let fd = openat(folder, &temp, flags, Mode::from_raw_mode(0o666))?;
    let written = fill(fd, mode, content).and_then(|()| Ok(renameat(folder, &temp, folder, name)?));
    if written.is_err() {
        let _ = unlinkat(folder, &temp, AtFlags::empty());
    }
    written
}

/// The permissions of the file being replaced, none when there is none. Opening it to write
/// (without truncating) checks it may be written, as writing in place would; a folder, a link
/// put there since the path was resolved, or anything but a regular file is refused.
fn existing_mode(folder: BorrowedFd<'_>, name: &OsStr) -> std::io::Result<Option<Mode>> {
    let flags = OFlags::WRONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    match openat(folder, name, flags, Mode::empty()) {
        Ok(existing) => {
            let stat = fstat(&existing)?;
            if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
                return Err(std::io::Error::other("not a regular file"));
            }
            Ok(Some(Mode::from_raw_mode(stat.st_mode)))
        }
        Err(Errno::NOENT) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn fill(fd: OwnedFd, mode: Option<Mode>, content: &[u8]) -> std::io::Result<()> {
    if let Some(mode) = mode {
        fchmod(fd.as_fd(), mode)?;
    }
    let mut file = std::fs::File::from(fd);
    file.write_all(content)?;
    file.sync_all()
}

/// A name for the new file that no one else picks.
fn temp_name() -> String {
    let mut random = [0u8; 8];
    // The OS random source does not fail in practice; `O_EXCL` refuses a clash anyway.
    let _ = getrandom::fill(&mut random);
    let hex: String = format!("{:016x}", u64::from_le_bytes(random));
    format!(".glave-write-{hex}.tmp")
}
