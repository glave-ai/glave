//! A program run to its end within a time limit, blocking the calling thread, for code that
//! runs outside the async runtime: in a process group of its own, its output read as it comes,
//! so it never waits on a full pipe, and its whole group killed once the limit passes. Once
//! the group has ended, what waits in its pipes is read, and nothing more is waited for: the
//! pipes may never close. macOS makes a pipe, then marks it to close when a program starts, in
//! two steps, so a program another thread starts between them holds a copy for as long as it
//! runs: an MCP server glave started beside `ps` held one, and the turn waited for it (#1323).

use std::fs::File;
use std::io::Read as _;
use std::os::fd::OwnedFd;
use std::os::unix::process::CommandExt as _;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::io::Errno;
use rustix::process::{Pid, Signal};

use crate::group;

/// The longest a program run this way goes unchecked for its end, while its output is waited
/// for.
const POLL: Duration = Duration::from_millis(5);

/// Runs `command` to its end, with what it printed on both its outputs; `None` when it was
/// still running after `limit`, and was killed. Whatever it leaves running in its group when
/// it ends, or when it is killed, is killed with it.
pub(crate) fn output(command: &mut Command, limit: Duration) -> std::io::Result<Option<Output>> {
    let started = Instant::now();
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()?;
    let leads = i32::try_from(child.id()).ok().and_then(group::of);
    let mut printed = [
        Printed::of(child.stdout.take()),
        Printed::of(child.stderr.take()),
    ];
    let status = ended(&mut child, started, limit, &mut printed);
    if let Some(group) = leads {
        stop_left(group);
    }
    // What it printed before its group ended waits in the pipes; their end is not waited for.
    while read(&mut printed, Duration::ZERO) {}
    let [stdout, stderr] = printed.map(|printed| printed.bytes);
    Ok(status?.map(|status| Output {
        status,
        stdout,
        stderr,
    }))
}

/// How `child` ended, or `None` once `limit` from `started` passed and it was killed; what it
/// prints meanwhile is read into `printed`.
fn ended(
    child: &mut Child,
    started: Instant,
    limit: Duration,
    printed: &mut [Printed; 2],
) -> std::io::Result<Option<ExitStatus>> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(Some(status)),
            Ok(None) if started.elapsed() < limit => {
                read(printed, POLL);
            }
            other => {
                let _ = child.kill();
                let _ = child.wait();
                return other.map(|_| None);
            }
        }
    }
}

/// Kills whatever is still in `group`, its leader reaped: what it left running.
fn stop_left(group: Pid) {
    if group::exists(group) {
        group::signal(group, Signal::KILL);
    }
}

/// What a program printed on one of its outputs so far, and the output's pipe until it closes.
struct Printed {
    pipe: Option<File>,
    bytes: Vec<u8>,
}

impl Printed {
    fn of(pipe: Option<impl Into<OwnedFd>>) -> Self {
        Self {
            pipe: pipe.map(|pipe| File::from(pipe.into())),
            bytes: Vec::new(),
        }
    }

    /// Takes what its pipe has, which it was found to have: some bytes, or its end.
    fn take(&mut self) {
        let Some(pipe) = &mut self.pipe else {
            return;
        };
        let mut chunk = [0_u8; 8192];
        match pipe.read(&mut chunk) {
            Ok(0) => self.pipe = None,
            Ok(read) => self
                .bytes
                .extend_from_slice(chunk.get(..read).unwrap_or_default()),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => self.pipe = None,
        }
    }
}

/// Reads what the open pipes of `printed` have, waiting up to `wait` for any of them to have
/// something: whether any had, bytes or its end. Pipes that cannot be watched are read no more.
fn read(printed: &mut [Printed; 2], wait: Duration) -> bool {
    let Ok(wait) = Timespec::try_from(wait) else {
        return false;
    };
    let polled = {
        let mut watched: Vec<PollFd<'_>> = printed
            .iter()
            .filter_map(|output| output.pipe.as_ref())
            .map(|pipe| PollFd::new(pipe, PollFlags::IN))
            .collect();
        loop {
            match poll(&mut watched, Some(&wait)) {
                Err(Errno::INTR) => {}
                polled => {
                    break polled.map(|_| {
                        let has = |fd: &PollFd<'_>| !fd.revents().is_empty();
                        watched.iter().map(has).collect::<Vec<_>>()
                    });
                }
            }
        }
    };
    let Ok(has) = polled else {
        for output in printed.iter_mut() {
            output.pipe = None;
        }
        return false;
    };
    let mut has = has.into_iter();
    let mut any = false;
    for output in printed.iter_mut().filter(|output| output.pipe.is_some()) {
        if has.next() == Some(true) {
            output.take();
            any = true;
        }
    }
    any
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    #[test]
    fn what_waits_in_a_pipe_another_program_holds_open_is_read_without_waiting_for_its_end() {
        // The writer stays open, as a program that took a copy of the pipe would hold it.
        let (reader, mut writer) = std::io::pipe().unwrap();
        writer.write_all(b"left").unwrap();
        let mut printed = [Printed::of(Some(reader)), Printed::of(None::<OwnedFd>)];
        while read(&mut printed, Duration::ZERO) {}
        let [out, err] = printed;
        assert_eq!((&out.bytes[..], &err.bytes[..]), (&b"left"[..], &b""[..]));
        assert!(
            out.pipe.is_some(),
            "its end never came, and was not waited for"
        );
        drop(writer);
    }
}
