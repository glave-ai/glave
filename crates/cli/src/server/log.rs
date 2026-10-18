//! The log of a server started in the background, `run/server.log`: what it prints, capped at
//! 10 MB, with one older file kept (`server.log.1`). The starter points the server's output at
//! the log; the server looks at its size every so often, and when it is over the cap, moves it
//! aside and points its output at a new one.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

/// How big the log may grow before it is moved aside.
const CAP: u64 = 10 * 1024 * 1024;

/// The older log beside `log`: `server.log.1`.
fn older(log: &Path) -> PathBuf {
    let mut name = log.as_os_str().to_owned();
    name.push(".1");
    PathBuf::from(name)
}

/// When `log` is over `cap`, moves it aside, over any older one, and opens a new one: the new
/// file, or none when it was not over.
fn rotate(log: &Path, cap: u64) -> io::Result<Option<File>> {
    use std::os::unix::fs::OpenOptionsExt as _;
    if std::fs::metadata(log).map_or(true, |kept| kept.len() <= cap) {
        return Ok(None);
    }
    std::fs::rename(log, older(log))?;
    OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(log)
        .map(Some)
}

/// Keeps this process's log at `log` under the cap: moves it aside when it is over, and points
/// standard output and error at the new one.
pub async fn keep_capped(log: PathBuf) -> io::Result<()> {
    tokio::task::spawn_blocking(move || {
        if let Some(file) = rotate(&log, CAP)? {
            rustix::stdio::dup2_stdout(&file)?;
            rustix::stdio::dup2_stderr(&file)?;
        }
        Ok(())
    })
    .await
    .map_err(io::Error::other)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_is_moved_aside_once_over_the_cap_and_one_older_file_is_kept() {
        let dir = std::env::temp_dir().join(format!("glave-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("server.log");
        assert!(rotate(&log, 8).unwrap().is_none(), "no log yet");
        std::fs::write(&log, "12345678").unwrap();
        assert!(rotate(&log, 8).unwrap().is_none(), "at the cap, not over");
        std::fs::write(&log, "first log!").unwrap();
        let mut new = rotate(&log, 8).unwrap().unwrap();
        std::io::Write::write_all(&mut new, b"second log").unwrap();
        assert_eq!(std::fs::read_to_string(older(&log)).unwrap(), "first log!");
        assert!(rotate(&log, 8).unwrap().is_some());
        assert_eq!(
            std::fs::read_to_string(older(&log)).unwrap(),
            "second log",
            "only one older file is kept"
        );
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
