//! The lifeline's helper: glave itself, started as `glave --glave-lifeline-1` with the pipe as
//! its standard input. It says it is ready, then reads each group glave holds (`+<leader>`),
//! each whose leader glave reaped (`~<leader>`) and each it lets go of (`-<leader>`), doing
//! nothing else. At the pipe's end, which comes when glave ends however it ends, it stops every
//! group glave still held: SIGTERM to the group, and to its leader while glave had not reaped
//! it, then SIGKILL to what is left after the grace, as the reaper does at the next start; then
//! it exits.
//!
//! No number is taken for another process's: a leader glave has not reaped keeps its number,
//! and a group keeps its own while anything is in it. glave lets go of a group once it is gone
//! or stopped, at most half a second after the last of it ends ([`crate::Child`]), and after
//! glave ends the helper acts at once.

use std::collections::BTreeMap;
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
    /// glave reaped its leader: the group alone is glave's.
    Reaped(i32),
    /// glave let go of it.
    LetGo(i32),
}

fn told(line: &[u8]) -> Option<Told> {
    let (what, leader) = line.split_first()?;
    let leader = std::str::from_utf8(leader).ok()?.parse().ok()?;
    match what {
        b'+' => Some(Told::Holds(leader)),
        b'~' => Some(Told::Reaped(leader)),
        b'-' => Some(Told::LetGo(leader)),
        _ => None,
    }
}

/// What glave held when its end closed: each group by its leader, and whether glave had
/// reaped that leader.
pub(crate) type Held = BTreeMap<i32, bool>;

/// Reads glave's lines to the pipe's end: the groups glave still held then. A line it cannot
/// read is skipped; the pipe failing to read ends the reading, as its end would.
pub(crate) fn follow(reader: impl BufRead) -> Held {
    let mut held = Held::new();
    for next in reader.split(b'\n') {
        let Ok(line) = next else { break };
        match told(&line) {
            Some(Told::Holds(leader)) => {
                held.insert(leader, false);
            }
            Some(Told::Reaped(leader)) => {
                if let Some(reaped) = held.get_mut(&leader) {
                    *reaped = true;
                }
            }
            Some(Told::LetGo(leader)) => {
                held.remove(&leader);
            }
            None => {}
        }
    }
    held
}

/// Stops the groups of `held`, and the leaders glave had not reaped. A leader that starts a
/// session of its own (the Linux sandbox's helper) leads no group until it has, so it is
/// signalled itself too, and killed after the grace if it still leads none.
pub(crate) fn stop(held: &Held) {
    let leaders: Vec<_> = held
        .keys()
        .filter_map(|leader| group::of(*leader))
        .collect();
    if leaders.is_empty() {
        return;
    }
    let unreaped: Vec<_> = held
        .iter()
        .filter(|(_, reaped)| !**reaped)
        .filter_map(|(leader, _)| group::of(*leader))
        .collect();
    let ungrouped: Vec<_> = unreaped
        .iter()
        .copied()
        .filter(|leader| !group::exists(*leader))
        .collect();
    for leader in &unreaped {
        let _ = kill_process(*leader, Signal::TERM);
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build();
    match runtime {
        // What outlives SIGKILL's wait is left to the next start, which the ledger tells.
        Ok(runtime) => {
            let _left_behind = runtime.block_on(group::stop_all(&leaders));
        }
        // Without a timer there is no grace to give: they are stopped at once.
        Err(_) => {
            for leader in &leaders {
                group::signal(*leader, Signal::KILL);
            }
        }
    }
    for leader in ungrouped {
        let _ = kill_process(leader, Signal::KILL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_say_what_glave_holds_and_lets_go_of() {
        assert_eq!(told(b"+42"), Some(Told::Holds(42)));
        assert_eq!(told(b"-42"), Some(Told::LetGo(42)));
        assert_eq!(told(b"~42"), Some(Told::Reaped(42)));
        assert_eq!(told(b"42"), None);
        assert_eq!(told(b"+"), None);
        assert_eq!(told(b"+x"), None);
        assert_eq!(told(b""), None);
        assert_eq!(told(b"+\xff1"), None);
    }

    #[test]
    fn what_glave_still_held_at_the_pipe_s_end_is_what_is_stopped() {
        let lines = b"+1\n+2\n\xff\xfe not a line\n-1\n+3\n~3\n~4\n";
        assert_eq!(follow(&lines[..]), Held::from([(2, false), (3, true)]));
    }
}
