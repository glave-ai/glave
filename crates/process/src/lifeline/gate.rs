//! A program starts only once glave holds it (#996). With a lifeline, glave starts itself as
//! `glave --glave-start-1 <input> <program> <arguments>…`, in the program's folder,
//! environment and process group, its input a pipe glave writes `go` to once the lifeline's
//! helper holds that group and the ledger records it; this then becomes the program, in the
//! same process, reading that pipe on (`pipe`) or nothing (`none`, the system's null device,
//! as a program started directly would). A glave that ends before then, however it ends,
//! closes the pipe with nothing said, and the program never runs: nothing it would have
//! started is left for the helper to miss.

use std::ffi::OsString;
use std::os::fd::{AsFd as _, BorrowedFd};
use std::os::unix::process::CommandExt as _;
use std::process::{ExitCode, Stdio};
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::io::Errno;

use super::{READY_WITHIN, WAITS};

/// The argument that makes glave start a program once it says so, with the version of what
/// it reads.
pub(crate) const START_ARG: &str = "--glave-start-1";

/// What the program reads once it runs: the pipe glave said go on, or nothing.
pub(crate) const PIPE: &str = "pipe";
pub(crate) const NONE: &str = "none";

/// What glave writes once the program may run.
pub(crate) const GO: &[u8] = b"go\n";

/// How long glave may take to hold a program: the helper started again for it as many times
/// as glave tries, each within its limit, and a minute to record it.
const GO_WITHIN: Duration = Duration::from_secs(
    READY_WITHIN
        .as_secs()
        .saturating_mul(WAITS as u64)
        .saturating_add(60),
);

/// Ended with when glave never said go, or was not understood: the program did not run.
pub(crate) const NOT_STARTED: u8 = 125;

/// Ended with when it could not become the program.
const NOT_RUN: u8 = 127;

/// Waits for glave to say go on this process's input, then becomes the program `rest` names,
/// with its arguments and the input `rest` begins with; the code to end with when it does not.
pub(crate) fn start(rest: &[OsString]) -> ExitCode {
    let Some((input, rest)) = rest.split_first() else {
        return ExitCode::from(NOT_STARTED);
    };
    let Some((program, arguments)) = rest.split_first() else {
        return ExitCode::from(NOT_STARTED);
    };
    if (input != PIPE && input != NONE) || !told_go(std::io::stdin().as_fd(), GO_WITHIN) {
        return ExitCode::from(NOT_STARTED);
    }
    let mut becomes = std::process::Command::new(program);
    becomes.args(arguments);
    if input == NONE {
        becomes.stdin(Stdio::null());
    }
    let _failed = becomes.exec();
    ExitCode::from(NOT_RUN)
}

/// Whether glave wrote [`GO`] on `input` within `within`. Read a byte at a time, so nothing
/// after it, which is the program's, is taken.
fn told_go(input: BorrowedFd<'_>, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    let mut heard = Vec::with_capacity(GO.len());
    while heard.len() < GO.len() {
        let left = deadline.saturating_duration_since(Instant::now());
        let Ok(timeout) = Timespec::try_from(left) else {
            return false;
        };
        let mut ready = [PollFd::new(&input, PollFlags::IN)];
        match poll(&mut ready, Some(&timeout)) {
            Ok(0) => return false,
            Ok(_) => {}
            Err(Errno::INTR) => continue,
            Err(_) => return false,
        }
        let mut byte = [0_u8; 1];
        match rustix::io::read(input, &mut byte) {
            Ok(1) => heard.extend_from_slice(&byte),
            Err(Errno::INTR) => continue,
            // Its end, with nothing said: glave is gone.
            Ok(_) | Err(_) => return false,
        }
        if !GO.starts_with(&heard) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};

    use super::*;

    #[test]
    fn go_is_read_alone_and_anything_else_or_the_end_is_no() {
        let (mut read, mut write) = std::io::pipe().unwrap();
        write.write_all(b"go\nthe program's own").unwrap();
        assert!(told_go(read.as_fd(), Duration::from_secs(5)));
        drop(write);
        let mut left = String::new();
        read.read_to_string(&mut left).unwrap();
        assert_eq!(left, "the program's own", "nothing after go is taken");

        let (read, mut write) = std::io::pipe().unwrap();
        write.write_all(b"gone\n").unwrap();
        assert!(!told_go(read.as_fd(), Duration::from_secs(5)), "not go");
        let (read, write) = std::io::pipe().unwrap();
        drop(write);
        assert!(!told_go(read.as_fd(), Duration::from_secs(5)), "glave gone");
        let (read, _held) = std::io::pipe().unwrap();
        assert!(
            !told_go(read.as_fd(), Duration::ZERO),
            "nothing said in time"
        );
    }
}
