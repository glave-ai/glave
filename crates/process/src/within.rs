//! A program run to its end within a time limit, blocking the calling thread, for code that
//! runs outside the async runtime: in a process group of its own, its output read beside it,
//! so it never waits on a full pipe, and its whole group killed once the limit passes, so
//! nothing it started holds its output open.

use std::os::unix::process::CommandExt as _;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal};

use crate::group;

/// How often a program run this way is checked for its end.
const POLL: Duration = Duration::from_millis(5);

/// Runs `command` to its end, with what it printed on both its outputs; `None` when it was
/// still running after `limit`, and was killed. Whatever it leaves running in its group when
/// it ends, or when it is killed, is killed with it, so the reading of its output ends too.
pub(crate) fn output(command: &mut Command, limit: Duration) -> std::io::Result<Option<Output>> {
    let started = Instant::now();
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()?;
    let leads = i32::try_from(child.id()).ok().and_then(group::of);
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    std::thread::scope(|scope| {
        let stdout = stdout.map(|pipe| scope.spawn(move || drained(pipe)));
        let stderr = stderr.map(|pipe| scope.spawn(move || drained(pipe)));
        let status = ended(&mut child, started, limit);
        if let Some(group) = leads {
            stop_left(group);
        }
        let stdout = stdout.map_or_else(Vec::new, |reading| reading.join().unwrap_or_default());
        let stderr = stderr.map_or_else(Vec::new, |reading| reading.join().unwrap_or_default());
        Ok(status?.map(|status| Output {
            status,
            stdout,
            stderr,
        }))
    })
}

/// How `child` ended, or `None` once `limit` from `started` passed and it was killed.
fn ended(
    child: &mut Child,
    started: Instant,
    limit: Duration,
) -> std::io::Result<Option<ExitStatus>> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(Some(status)),
            Ok(None) if started.elapsed() < limit => std::thread::sleep(POLL),
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

/// Everything `pipe` gives until it closes.
fn drained(mut pipe: impl std::io::Read) -> Vec<u8> {
    let mut bytes = Vec::new();
    let _ = pipe.read_to_end(&mut bytes);
    bytes
}
