//! glave's own folder on commands' `PATH` (see [`crate::LocalComputers::with_glave_on_path`]):
//! it holds `glave`, a link to the glave that runs them, so a command reaches that glave
//! whatever else is installed, and none at all. Made at each start, as Codex links its own
//! binary for its `apply_patch` (`codex-rs/arg0`): a link to the binary's path, so an upgrade
//! that replaces the binary in place keeps it working.

use std::ffi::OsString;
use std::io;
use std::path::Path;

/// Makes `folder` and in it `glave`, a link to `program`, replacing any link there: a new one
/// at another name is put in its place in one step, so a command never finds none.
///
/// The folder is glave's own, but on a person's computer it sits on their disk, where they
/// can make it a link. Followed, a link to the folder `program` is in would make `program` a
/// link to itself, and that computer would never serve again (#1499). So it is looked up from
/// an open handle on the folder holding it, never through a link: a link or a file there is
/// replaced by a folder, what it led to is left as it was, and `glave` is made through that
/// folder's own open handle.
#[cfg(unix)]
pub async fn glave_on_path(folder: &Path, program: &Path) -> io::Result<()> {
    let (folder, program) = (folder.to_owned(), program.to_owned());
    // A few calls to the system, at start.
    tokio::task::spawn_blocking(move || put_on_path(&folder, &program))
        .await
        .map_err(io::Error::other)?
}

#[cfg(unix)]
fn put_on_path(folder: &Path, program: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt as _;

    use rustix::fs::{
        AtFlags, FileType, Mode, mkdirat, openat, renameat, statat, symlinkat, unlinkat,
    };
    use rustix::io::Errno;

    use super::anchored::FOLDER;

    let (Some(holder), Some(name)) = (folder.parent(), folder.file_name()) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a folder glave can make", folder.display()),
        ));
    };
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(holder)?;
    let holder = rustix::fs::open(
        holder,
        FOLDER.difference(rustix::fs::OFlags::NOFOLLOW),
        Mode::empty(),
    )?;
    let own = Mode::from_raw_mode(0o700);
    match statat(&holder, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) if FileType::from_raw_mode(stat.st_mode) == FileType::Directory => {}
        Ok(_) => {
            unlinkat(&holder, name, AtFlags::empty())?;
            mkdirat(&holder, name, own)?;
        }
        Err(Errno::NOENT) => mkdirat(&holder, name, own)?,
        Err(error) => return Err(error.into()),
    }
    let folder = openat(&holder, name, FOLDER, Mode::empty())?;
    let writing = format!(".glave.{}", std::process::id());
    match unlinkat(&folder, writing.as_str(), AtFlags::empty()) {
        Ok(()) | Err(Errno::NOENT) => {}
        Err(error) => return Err(error.into()),
    }
    symlinkat(program, &folder, writing.as_str())?;
    renameat(&folder, writing.as_str(), &folder, "glave")?;
    Ok(())
}

/// Where links are not made this way: glave's folder is not put on commands' `PATH`.
#[cfg(not(unix))]
pub async fn glave_on_path(_folder: &Path, _program: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "glave is put on commands' PATH on macOS and Linux only",
    ))
}

#[cfg(all(test, unix))]
mod tests {
    use glave_ports::block_on;
    use glave_ports::testing::TempDir;

    use super::glave_on_path;

    #[test]
    fn a_link_where_the_folder_goes_is_replaced_and_what_it_led_to_is_left_alone() {
        let dir = TempDir::new("on-path-link").unwrap();
        // glave copied in at wake, and the person's link from glave's folder on `PATH` to its.
        let installed = dir.join("opt");
        std::fs::create_dir(&installed).unwrap();
        let program = installed.join("glave");
        std::fs::write(&program, "the program").unwrap();
        let folder = dir.join("bin");
        std::os::unix::fs::symlink(&installed, &folder).unwrap();
        block_on!(async { glave_on_path(&folder, &program).await.unwrap() });
        let kept = std::fs::symlink_metadata(&program).unwrap();
        assert!(kept.is_file(), "the program is still itself, not a link");
        assert_eq!(std::fs::read_to_string(&program).unwrap(), "the program");
        let made = std::fs::symlink_metadata(&folder).unwrap();
        assert!(made.is_dir(), "glave's folder is a folder again");
        assert_eq!(std::fs::read_link(folder.join("glave")).unwrap(), program);
        assert!(std::fs::symlink_metadata(&installed).unwrap().is_dir());
    }
}

/// `environment`'s `PATH` with `folder` first.
pub(crate) fn first_on_path(environment: &[(OsString, OsString)], folder: &Path) -> OsString {
    let mut first = folder.as_os_str().to_owned();
    if let Some((_, path)) = environment.iter().find(|(name, _)| name == "PATH") {
        first.push(":");
        first.push(path);
    }
    first
}
