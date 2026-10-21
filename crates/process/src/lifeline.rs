//! The lifeline: what glave starts stops at once when glave ends, however it ends, SIGKILL
//! included (`specs/one-server.md`, "No orphaned processes").
//!
//! A helper of glave's own (glave itself, started again as [`HELPER_ARG`]) runs beside it, in
//! a process group of its own, reading a pipe only glave writes: a line as each process group
//! starts, one when glave reaps its leader, and one once glave lets go of it. When glave ends, the system closes glave's end of
//! the pipe, whatever ended it; the helper reads the end and stops every group glave still
//! held ([`helper`]). This is glave's end: the helper, started with the lifeline and started
//! again (told what runs) if it is gone, each time only once it says it reads the pipe.

pub(crate) mod helper;
#[cfg(test)]
mod tests;

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::io::{PipeWriter, Write as _};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock, RwLockReadGuard};
use std::time::Duration;

use tokio::io::AsyncReadExt as _;
use tokio::process::ChildStdout;

use crate::error::ProcessError;

/// The argument that makes a program glave's lifeline helper, with the version of what it
/// reads: a newer glave that changes the lines changes the number. A flag, so a glave from
/// before the lifeline, found at glave's path after a downgrade, refuses it and runs nothing.
pub(crate) const HELPER_ARG: &str = "--glave-lifeline-1";

/// What the helper says once it reads glave's pipe.
pub(crate) const READY: &[u8] = b"glave-lifeline 1\n";

/// This program, to start again as a helper of its own: the lifeline's (it passes this to
/// [`crate::Processes::with_lifeline`]), or another. On Linux the file the system runs it
/// from, which still starts after an upgrade or a rebuild replaced or removed the one at its
/// path.
pub fn this_program() -> std::io::Result<PathBuf> {
    if cfg!(target_os = "linux") {
        Ok(PathBuf::from("/proc/self/exe"))
    } else {
        std::env::current_exe()
    }
}

/// How long a helper may take to say it is ready: a guard against a hang, not a speed limit.
const READY_WITHIN: Duration = Duration::from_secs(30);

/// glave's end of the lifeline, shared by every clone of one [`crate::Processes`].
#[derive(Debug)]
pub(crate) struct Lifeline {
    /// The program that runs the helper: glave itself.
    program: PathBuf,
    state: Mutex<State>,
    /// Held to read while a program starts, and to write while the helper and its pipe are
    /// made: on macOS a pipe is made, then marked to close when a program starts, in two steps,
    /// and a program started between them would hold the pipe open. Every program started
    /// through these processes starts holding it.
    forking: RwLock<()>,
}

#[derive(Debug, Default)]
struct State {
    /// The helper and glave's end of the pipe it reads.
    helper: Option<Helper>,
    /// The groups glave holds, by their leader: told again to a helper started afresh.
    live: BTreeSet<i32>,
}

/// A helper just started, before it says it is ready: its process and what it says.
struct Launched {
    pid: u32,
    said: ChildStdout,
}

#[derive(Debug)]
struct Helper {
    /// Reaped by tokio once let go of; it ends by itself when the pipe closes.
    process: tokio::process::Child,
    pipe: PipeWriter,
}

impl Lifeline {
    /// Starts the helper `program` (glave itself) and waits until it says it reads the pipe, so
    /// a program that cannot be a helper is refused before anything starts.
    pub(crate) async fn start(program: PathBuf) -> Result<Self, ProcessError> {
        let lifeline = Self {
            program,
            state: Mutex::new(State::default()),
            forking: RwLock::new(()),
        };
        let launched = {
            let mut state = lifeline.lock();
            lifeline.launch(&mut state)
        };
        lifeline
            .ready(launched.map_err(|error| lifeline.failed(error))?)
            .await?;
        Ok(lifeline)
    }

    /// Held while a program starts: the helper's pipe is not made meanwhile.
    pub(crate) fn starting(&self) -> RwLockReadGuard<'_, ()> {
        self.forking.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// Tells the helper that glave holds the group `leader` leads, until the tie drops. A
    /// helper that is gone is started again and told every group glave holds; when what is at
    /// glave's path no longer starts as one (removed, or replaced by another version), this
    /// fails, as starting the lifeline would, and glave lets go of `leader` again.
    pub(crate) async fn tie(self: &Arc<Self>, leader: i32) -> Result<Tie, ProcessError> {
        // Dropped on failure, which lets go of `leader` again.
        let tie = Tie {
            lifeline: Arc::clone(self),
            leader,
        };
        let launched = {
            let mut state = self.lock();
            state.live.insert(leader);
            if state.tell(&format!("+{leader}\n")) {
                None
            } else {
                Some(self.launch(&mut state))
            }
        };
        if let Some(launched) = launched {
            self.ready(launched.map_err(|error| self.failed(error))?)
                .await?;
        }
        Ok(tie)
    }

    /// glave lets go of the group `leader` leads. A helper that is gone hears it from the next
    /// one, which is told only what glave holds.
    fn release(&self, leader: i32) {
        let mut state = self.lock();
        state.live.remove(&leader);
        state.tell(&format!("-{leader}\n"));
    }

    /// Starts the helper in a process group of its own, so no signal meant for glave's group
    /// (Ctrl-C, a test runner stopping glave's) reaches it, and tells it every group glave
    /// holds. What it says is handed back, for [`Lifeline::ready`].
    fn launch(&self, state: &mut State) -> std::io::Result<Launched> {
        // Named as this program is, where Linux would show `/proc/self/exe`.
        let name = std::env::args_os()
            .next()
            .unwrap_or_else(|| self.program.clone().into_os_string());
        let (mut process, mut pipe) = {
            let _alone = self.forking.write().unwrap_or_else(PoisonError::into_inner);
            let (read, pipe) = std::io::pipe()?;
            let process = tokio::process::Command::new(&self.program)
                .arg0(name)
                .arg(HELPER_ARG)
                .stdin(Stdio::from(read))
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .process_group(0)
                .spawn()?;
            (process, pipe)
        };
        let mut told = String::new();
        for leader in &state.live {
            let _ = writeln!(told, "+{leader}");
        }
        pipe.write_all(told.as_bytes())?;
        let (Some(pid), Some(said)) = (process.id(), process.stdout.take()) else {
            return Err(std::io::Error::other("it ended as it started"));
        };
        state.helper = Some(Helper { process, pipe });
        Ok(Launched { pid, said })
    }

    /// Waits until the helper `launched` says it reads the pipe. One that does not (another
    /// program, or another version of glave, now at glave's path) is stopped and forgotten, so
    /// the next program glave starts tries again rather than tell it anything.
    async fn ready(&self, launched: Launched) -> Result<(), ProcessError> {
        let Launched { pid, mut said } = launched;
        let mut heard = Vec::new();
        let read = tokio::time::timeout(READY_WITHIN, async {
            let mut byte = [0_u8; 1];
            while heard.len() < READY.len() && said.read(&mut byte).await? == 1 {
                heard.extend_from_slice(&byte);
            }
            std::io::Result::Ok(())
        })
        .await;
        let error = match read {
            Ok(Ok(())) if heard == READY => return Ok(()),
            Ok(Err(error)) => error,
            _ => std::io::Error::other(
                "it did not say it was ready: the program there is of another version, or damaged",
            ),
        };
        let mut state = self.lock();
        if state.helper.as_ref().and_then(|helper| helper.process.id()) == Some(pid)
            && let Some(mut helper) = state.helper.take()
        {
            let _ = helper.process.start_kill();
        }
        Err(self.failed(error))
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn failed(&self, source: std::io::Error) -> ProcessError {
        ProcessError::Lifeline {
            helper: self.program.clone(),
            source,
        }
    }
}

impl State {
    /// Writes `line` to the helper; false, and the helper forgotten, when it cannot be: the
    /// helper is gone. A line is a few bytes the helper reads at once, doing nothing else
    /// until glave ends, so this never waits.
    fn tell(&mut self, line: &str) -> bool {
        let Some(helper) = &mut self.helper else {
            return false;
        };
        let told = !matches!(helper.process.try_wait(), Ok(Some(_)))
            && helper.pipe.write_all(line.as_bytes()).is_ok();
        if !told {
            self.helper = None;
        }
        told
    }
}

/// A process group glave holds, tied to it by the lifeline; the helper is told glave let go
/// of it when this drops.
#[derive(Debug)]
pub(crate) struct Tie {
    lifeline: Arc<Lifeline>,
    leader: i32,
}

impl Tie {
    /// glave reaped the group's leader, whose number may be another's from now on: the helper
    /// is to stop the group alone.
    pub(crate) fn reaped(&self) {
        self.lifeline.lock().tell(&format!("~{}\n", self.leader));
    }
}

impl Drop for Tie {
    fn drop(&mut self) {
        self.lifeline.release(self.leader);
    }
}
