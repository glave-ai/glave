//! Whether a sandbox started the command it was made for (#1533). bubblewrap cannot make the
//! sandbox when a place it was asked to hide goes away as it mounts over it (an ssh session's
//! forwarded agent ending, an editor or tmux closing): glave hides each place as it was when it
//! planned the command's mounts. bubblewrap then says why on its error output and ends without
//! running the command, which a sandbox made again from a fresh look, no longer listing the
//! place, starts.
//!
//! So the command's shell, once inside, first writes a line on that output, with a token made
//! for this launch alone. The output is not the sandbox's alone, though: the shell writes its
//! own startup errors there, and a `BASH_ENV` file and the functions the environment exports
//! run before the line, and may keep it from coming (by sending the shell's errors elsewhere,
//! by standing in for what writes the line, or by ending the shell). So no line proves
//! nothing: only an end where the wrapper said it could not start the command proves the
//! command never ran, and only bubblewrap's such end is tried again. Anything else is the
//! command's own, run once: a late line, an output that cannot be read, or an end where the
//! wrapper said nothing of its own leaves the command to end as it does, as it did before the
//! line.

use std::fmt::Write as _;
use std::time::Duration;

use glave_ports::computer::ComputerError;
use glave_ports::{Entropy, EntropyError};
use tokio::io::AsyncReadExt as _;
use tokio::process::ChildStderr;

use crate::local::sandbox::Refuses;

/// How many sandboxes are made for one command, each from a fresh look, while bubblewrap says
/// the last could not start it.
const SANDBOXES: usize = 3;

/// How long a sandbox has to start a command, which takes it milliseconds: a guard against a
/// hang, not a speed limit. A command whose line has not come by then runs on.
pub(super) const WITHIN: Duration = Duration::from_secs(10);

/// The most of what a sandbox writes that is kept, to say why it started nothing.
const MOST_SAID: usize = 4096;

/// The line one launch's shell writes once inside the sandbox: a token of its own, so that
/// nothing that runs or is named before it can stand in for it.
pub(super) struct Line(String);

impl Line {
    /// A line with a token from `entropy`.
    pub(super) fn new(entropy: &dyn Entropy) -> Result<Self, EntropyError> {
        let mut line = "glave-sandbox-started-".to_owned();
        for byte in entropy.bytes()? {
            let _ = write!(line, "{byte:02x}");
        }
        line.push('\n');
        Ok(Self(line))
    }

    /// What the command's shell runs first: the line, on the sandbox's error output, by the
    /// shell's own `printf`, which a function the environment exports does not stand in for.
    pub(super) fn said_first(&self) -> String {
        format!("builtin printf '%s\\n' {} >&2; ", self.0.trim_end())
    }

    fn heard_in(&self, said: &[u8]) -> bool {
        let line = self.0.as_bytes();
        said.windows(line.len()).any(|window| window == line)
    }
}

/// How a launch's start came out.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Start {
    /// The command runs, or may have: it is the command's own from here.
    Runs,
    /// The wrapper ended without starting it, saying this; `again` when a sandbox made again
    /// from a fresh look may start it.
    NeverRan { said: String, again: bool },
}

/// Waits, at most `within`, until the shell in the sandbox `refuses` describes has said the
/// line on `output`, or the output has ended.
pub(super) async fn wait(
    output: &mut ChildStderr,
    line: &Line,
    refuses: Refuses,
    within: Duration,
) -> Start {
    let mut said = Vec::new();
    let heard = tokio::time::timeout(within, heard(output, line, &mut said)).await;
    // Late, or unreadable: whatever runs, runs on.
    let Ok(Heard::Ended) = heard else {
        return Start::Runs;
    };
    let said = String::from_utf8_lossy(&said).trim().to_owned();
    if said.starts_with(refuses.saying) {
        Start::NeverRan {
            said,
            again: refuses.passing,
        }
    } else {
        Start::Runs
    }
}

/// How reading the sandbox's output ended.
enum Heard {
    /// The line came.
    Line,
    /// The output ended first.
    Ended,
    /// It could not be read.
    Unread,
}

/// Reads `output` into `said`, its last [`MOST_SAID`] bytes, until the line comes or the
/// output ends.
async fn heard(output: &mut ChildStderr, line: &Line, said: &mut Vec<u8>) -> Heard {
    let mut chunk = [0_u8; 1024];
    loop {
        let read = match output.read(&mut chunk).await {
            Ok(0) => return Heard::Ended,
            Ok(read) => read,
            Err(_) => return Heard::Unread,
        };
        said.extend_from_slice(chunk.get(..read).unwrap_or_default());
        if line.heard_in(said) {
            return Heard::Line;
        }
        let over = said.len().saturating_sub(MOST_SAID);
        said.drain(..over);
    }
}

/// What one try at starting a command in a sandbox came to.
pub(super) enum Tried<T> {
    /// It runs, or may have.
    Runs(T),
    /// It never ran: the wrapper said this, and whether one made again may start it.
    NeverRan { said: String, again: bool },
}

/// Tries `attempt` until a sandbox starts the command: one bubblewrap said it could not start
/// is made again from a fresh look, [`SANDBOXES`] times at most, and fails then with what it
/// said last; any other that never ran fails at once with what it said, and one that runs or
/// may have is never tried again.
pub(super) async fn until_started<T, Attempt, Attempted>(
    mut attempt: Attempt,
) -> Result<T, ComputerError>
where
    Attempt: FnMut() -> Attempted,
    Attempted: Future<Output = Result<Tried<T>, ComputerError>>,
{
    let mut last = String::new();
    for _ in 0..SANDBOXES {
        match attempt().await? {
            Tried::Runs(runs) => return Ok(runs),
            Tried::NeverRan { said, again: true } => last = said,
            Tried::NeverRan { said, again: false } => return Err(not_started(&said)),
        }
    }
    Err(not_started(&last))
}

/// Why a command did not start, with what its sandbox said.
fn not_started(said: &str) -> ComputerError {
    ComputerError::Failed(if said.is_empty() {
        "the sandbox could not start the command, and did not say why".to_owned()
    } else {
        format!("the sandbox could not start the command: {said}")
    })
}

#[cfg(test)]
mod tests;
