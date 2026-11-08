//! Where a test's Unix socket goes: in the test's folder, or, where that path would be too long
//! for a socket's, in a short folder of its own.

use std::ops::Deref;
use std::path::{Path, PathBuf};

use super::TempDir;
use crate::MAX_SOCKET_PATH;

/// Where a socket goes when its folder is too deep for one: short on every system glave runs on.
const SHORT: &str = "/tmp";

/// The path of a Unix socket a test makes, its file removed when this drops. It reads as the
/// [`Path`] it is.
///
/// It is in the folder it is made for when that path is short enough for a socket's
/// ([`MAX_SOCKET_PATH`]), and otherwise alone in a new folder in `/tmp`, removed with it: a
/// [`TempDir`] in macOS's temp folder (`/private/var/folders/…/T`, 56 bytes) leaves too little
/// room for a socket's name.
pub struct SocketPath {
    path: PathBuf,
    /// The folder made for it where its own was too deep, removed after its file.
    _short: Option<TempDir>,
}

impl SocketPath {
    /// The path of a socket named `name` for the folder `dir`: `dir/name`, or a path in a new
    /// folder made now in `/tmp` where that one is too long.
    pub fn new(dir: &Path, name: &str) -> std::io::Result<Self> {
        let path = dir.join(name);
        if path.as_os_str().len() <= MAX_SOCKET_PATH {
            return Ok(Self { path, _short: None });
        }
        let short = TempDir::within(Path::new(SHORT), "socket")?;
        Ok(Self {
            path: short.join(name),
            _short: Some(short),
        })
    }
}

impl Deref for SocketPath {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<Path> for SocketPath {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl Drop for SocketPath {
    fn drop(&mut self) {
        // A listener leaves its socket's file, and one killed cannot remove it. Best effort: a
        // drop cannot fail the test, and the folder holding it goes in any case.
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::net::UnixListener;
    use std::path::Path;

    use super::{MAX_SOCKET_PATH, SHORT, SocketPath, TempDir};

    #[test]
    fn a_socket_too_deep_for_its_folder_listens_in_a_short_one_of_its_own_that_goes_with_it() {
        let dir = TempDir::new("socket-test").unwrap();
        // Deeper than any socket's path may be, wherever the system's temp folder is.
        let deep = dir.join("d".repeat(MAX_SOCKET_PATH));
        std::fs::create_dir(&deep).unwrap();
        let (first, second) = (
            SocketPath::new(&deep, "glave.sock").unwrap(),
            SocketPath::new(&deep, "glave.sock").unwrap(),
        );
        assert!(first.starts_with(SHORT), "{}", first.display());
        assert!(first.ends_with("glave.sock"), "{}", first.display());
        assert!(first.as_os_str().len() <= MAX_SOCKET_PATH);
        assert_ne!(*first, *second, "each in a folder of its own");
        let listener = UnixListener::bind(&first).unwrap();
        let folder = first.parent().unwrap().to_owned();
        drop(listener);
        drop(first);
        assert!(!folder.exists(), "{}", folder.display());
        assert!(second.parent().unwrap().is_dir());
    }

    #[test]
    fn a_socket_that_fits_stays_in_its_folder_and_its_file_goes_when_it_does() {
        let dir = TempDir::within(Path::new(SHORT), "socket-test").unwrap();
        let socket = SocketPath::new(&dir, "glave.sock").unwrap();
        assert_eq!(*socket, dir.join("glave.sock"));
        drop(UnixListener::bind(&socket).unwrap());
        assert!(socket.exists(), "a listener leaves its file");
        drop(socket);
        assert!(!dir.join("glave.sock").exists());
        assert!(dir.is_dir(), "its folder stays");
    }
}
