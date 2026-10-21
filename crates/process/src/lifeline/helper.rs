//! The lifeline's helper: glave itself, started as `glave --glave-lifeline-1` with the pipe as
//! its standard input. It says it is ready, then reads each group glave holds (`+<leader>`) and
//! each it lets go of (`-<leader>`), doing nothing else. At the pipe's end, which comes when
//! glave ends however it ends, it stops every group glave still held: SIGTERM to the group and
//! its leader, then SIGKILL to what is left after the grace, as the reaper does at the next
//! start; then it exits.
//!
//! No number is taken for another process's: glave holds a group only while its leader is
//! glave's unreaped child, which keeps the number from being given to another process, and
//! lets go once it reaps the leader ([`crate::Child::wait`]). After glave ends, a group's
//! number stays its own while anything is in it, and the helper acts at once.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::{BufRead, Write as _};
use std::process::ExitCode;

use rustix::process::{Signal, kill_process};

use super::{HELPER_ARG, READY};
use crate::group;

/// Runs this process as the lifeline's helper when `args` (the whole command line) are the
/// helper's: it returns, with the code to exit with, once glave has ended and what it left
/// running is stopped. Returns `None` when they are not. A program that passes its own path
/// to [`crate::Processes::with_lifeline`] calls this first thing in `main`.
pub fn lifeline_helper(args: &[OsString]) -> Option<ExitCode> {
    if args.get(1).is_none_or(|first| first != HELPER_ARG) {
        return None;
    }
    // A glave that no longer waits for this hears nothing, which is fine.
    let mut said = std::io::stdout();
    let _ = said.write_all(READY).and_then(|()| said.flush());
    stop(&follow(std::io::stdin().lock()));
    Some(ExitCode::SUCCESS)
}

/// One line from glave.
#[derive(Debug, PartialEq, Eq)]
enum Told {
    /// glave holds the group this leads.
    Holds(i32),
    /// glave let go of it.
    LetGo(i32),
}

fn told(line: &[u8]) -> Option<Told> {
    let (what, leader) = line.split_first()?;
    let leader = std::str::from_utf8(leader).ok()?.parse().ok()?;
    match what {
        b'+' => Some(Told::Holds(leader)),
        b'-' => Some(Told::LetGo(leader)),
        _ => None,
    }
}

/// Reads glave's lines to the pipe's end: the groups glave still held then, by their leader.
/// A line it cannot read is skipped; the pipe failing to read ends the reading, as its end
/// would.
pub(crate) fn follow(reader: impl BufRead) -> BTreeSet<i32> {
    let mut held = BTreeSet::new();
    for next in reader.split(b'\n') {
        let Ok(line) = next else { break };
        match told(&line) {
            Some(Told::Holds(leader)) => {
                held.insert(leader);
            }
            Some(Told::LetGo(leader)) => {
                held.remove(&leader);
            }
            None => {}
        }
    }
    held
}

/// Stops the groups of `held` and their leaders. A leader that starts a session of its own
/// (the Linux sandbox's helper) leads no group until it has, so it is signalled itself too.
pub(crate) fn stop(held: &BTreeSet<i32>) {
    let leaders: Vec<_> = held
        .iter()
        .filter_map(|leader| group::of(*leader))
        .collect();
    if leaders.is_empty() {
        return;
    }
    for leader in &leaders {
        let _ = kill_process(*leader, Signal::TERM);
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build();
    match runtime {
        Ok(runtime) => runtime.block_on(group::stop_all(&leaders)),
        // Without a timer there is no grace to give: they are stopped at once.
        Err(_) => {
            for leader in leaders {
                group::signal(leader, Signal::KILL);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_say_what_glave_holds_and_lets_go_of() {
        assert_eq!(told(b"+42"), Some(Told::Holds(42)));
        assert_eq!(told(b"-42"), Some(Told::LetGo(42)));
        assert_eq!(told(b"42"), None);
        assert_eq!(told(b"+"), None);
        assert_eq!(told(b"+x"), None);
        assert_eq!(told(b""), None);
        assert_eq!(told(b"+\xff1"), None);
    }

    #[test]
    fn what_glave_still_held_at_the_pipe_s_end_is_what_is_stopped() {
        let lines = b"+1\n+2\n\xff\xfe not a line\n-1\n+3\n";
        assert_eq!(follow(&lines[..]), BTreeSet::from([2, 3]));
    }
}
