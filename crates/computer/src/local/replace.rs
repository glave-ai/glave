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

/// Puts content into the new file.
type Fill = fn(&mut std::fs::File, &[u8]) -> std::io::Result<()>;

/// Replaces the file `name` in `folder` with `content`, or creates it.
pub(crate) fn replace(folder: BorrowedFd<'_>, name: &OsStr, content: &[u8]) -> std::io::Result<()> {
    replace_with(folder, name, content, |file, content| {
        file.write_all(content)
    })
}

/// [`replace`], with `put` writing the content: tests pass one that fails as a full disk
/// does, part way through.
fn replace_with(
    folder: BorrowedFd<'_>,
    name: &OsStr,
    content: &[u8],
    put: Fill,
) -> std::io::Result<()> {
    let mode = existing_mode(folder, name)?;
    let temp = temp_name()?;
    let flags = OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let fd = openat(folder, &temp, flags, Mode::from_raw_mode(0o666))?;
    let written =
        fill(fd, mode, content, put).and_then(|()| Ok(renameat(folder, &temp, folder, name)?));
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

fn fill(fd: OwnedFd, mode: Option<Mode>, content: &[u8], put: Fill) -> std::io::Result<()> {
    if let Some(mode) = mode {
        fchmod(fd.as_fd(), mode)?;
    }
    let mut file = std::fs::File::from(fd);
    put(&mut file, content)?;
    file.sync_all()
}

/// A name for the new file that no one else picks, or why the random source gave none.
fn temp_name() -> std::io::Result<String> {
    let mut random = [0u8; 8];
    getrandom::fill(&mut random).map_err(std::io::Error::other)?;
    let hex: String = format!("{:016x}", u64::from_le_bytes(random));
    Ok(format!(".glave-write-{hex}.tmp"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes half the content, then finds the disk full.
    fn full_disk(file: &mut std::fs::File, content: &[u8]) -> std::io::Result<()> {
        file.write_all(content.get(..content.len() / 2).unwrap_or_default())?;
        Err(std::io::Error::from_raw_os_error(
            Errno::NOSPC.raw_os_error(),
        ))
    }

    #[test]
    fn a_full_disk_leaves_the_file_as_it_was_and_says_so() {
        let dir = glave_ports::testing::TempDir::new("full").unwrap();
        std::fs::write(dir.join("a.txt"), "the original\n").unwrap();
        let folder = std::fs::File::open(&dir).unwrap();
        let new = b"a much longer replacement that never fits on the disk\n";
        let failed = replace_with(folder.as_fd(), OsStr::new("a.txt"), new, full_disk).unwrap_err();
        assert_eq!(failed.kind(), std::io::ErrorKind::StorageFull);
        assert_eq!(
            std::fs::read_to_string(dir.join("a.txt")).unwrap(),
            "the original\n",
            "the file is intact"
        );
        let left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(
            left,
            vec![std::ffi::OsString::from("a.txt")],
            "no half-written file left"
        );
        let said = super::super::files::io("a.txt", &failed).to_string();
        assert!(
            said.contains("disk is full") && said.contains("a.txt"),
            "{said}"
        );
        // A new file the disk cannot hold is not left half made either.
        assert!(replace_with(folder.as_fd(), OsStr::new("b.txt"), new, full_disk).is_err());
        assert!(!dir.join("b.txt").exists());
    }
}
