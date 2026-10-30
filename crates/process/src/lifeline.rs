//! The lifeline: what glave starts stops at once when glave ends, however it ends, SIGKILL
//! included (`specs/one-server.md`, "No orphaned processes").
//!
//! A helper of glave's own (glave itself, started again as [`HELPER_ARG`]) runs beside it, in
//! a process group of its own, reading a pipe only glave writes: a line as each process group
//! starts, one when glave reaps its leader, and one once glave lets go of it. When glave ends, the system closes glave's end of
//! the pipe, whatever ended it; the helper reads the end and stops every group glave still
//! held ([`helper`]). This is glave's end: the helper, started with the lifeline and started
//! again (told what runs) if it is gone, each time only once it says it reads the pipe.

pub(crate) mod gate;
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
use tokio::sync::watch;

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
    /// How many helpers glave has started: each one's number, so that finding one not ready
    /// never touches another started since.
    launches: u64,
}

#[derive(Debug)]
struct Helper {
    /// Reaped by tokio once let go of; it ends by itself when the pipe closes.
    process: tokio::process::Child,
    pipe: PipeWriter,
    /// Its number among the helpers glave started.
    launch: u64,
    /// Whether it said it is ready: none yet, then true or false; closed with none if the
    /// program that started it was given up first.
    ready: watch::Receiver<Option<bool>>,
}

/// Why a program started as the helper is not one: no ready line, or another.
const NOT_READY: &str =
    "it did not say it was ready: the program there is of another version, or damaged";

/// Why a program waited for helpers it never heard from: each was started for another program
/// whose start was given up before the helper answered.
const GIVEN_UP: &str = "it was started for other programs that were stopped before it answered";

/// How many helpers one program waits for when those it waited for were given up unanswered.
const WAITS: usize = 3;

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
        launched
            .map_err(|error| lifeline.failed(error))?
            .ready()
            .await?;
        Ok(lifeline)
    }

    /// The program that starts each program once glave holds it: glave itself.
    pub(crate) fn starter(&self) -> &std::path::Path {
        &self.program
    }

    /// Held while a program starts: the helper's pipe is not made meanwhile.
    pub(crate) fn starting(&self) -> RwLockReadGuard<'_, ()> {
        self.forking.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// Tells the helper that glave holds the group `leader` leads, until the tie drops. A
    /// helper that is gone is started again and told every group glave holds. Returns once the
    /// helper said it is ready; fails, as starting the lifeline would, when it does not (glave's
    /// path holds no helper now: removed, or replaced by another version), and glave lets go
    /// of `leader` again.
    pub(crate) async fn tie(self: &Arc<Self>, leader: i32) -> Result<Tie, ProcessError> {
        // Dropped on failure, which lets go of `leader` again.
        let tie = Tie {
            lifeline: Arc::clone(self),
            leader,
        };
        for _ in 0..WAITS {
            if self.hold(leader).await? {
                return Ok(tie);
            }
        }
        Err(self.failed(std::io::Error::other(GIVEN_UP)))
    }

    /// Tells the helper that glave holds `leader`, starting one if it is gone, and waits until
    /// the helper said it is ready: false when the one waited for was started by another
    /// program whose start was given up before it answered, which leaves no helper.
    async fn hold(self: &Arc<Self>, leader: i32) -> Result<bool, ProcessError> {
        let (launched, pending) = {
            let mut state = self.lock();
            state.live.insert(leader);
            if state.tell(&format!("+{leader}\n")) {
                // Told a helper another program just started: this waits for it too.
                let pending = state
                    .helper
                    .as_ref()
                    .map(|helper| helper.ready.clone())
                    .filter(|ready| *ready.borrow() != Some(true));
                (None, pending)
            } else {
                (Some(self.launch(&mut state)), None)
            }
        };
        if let Some(launched) = launched {
            launched
                .map_err(|error| self.failed(error))?
                .ready()
                .await?;
        }
        let Some(mut pending) = pending else {
            return Ok(true);
        };
        match pending.wait_for(Option::is_some).await.as_deref() {
            Ok(Some(true)) => Ok(true),
            Ok(_) => Err(self.failed(std::io::Error::other(NOT_READY))),
            Err(_) => Ok(false),
        }
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
    /// holds. It is glave's helper from now on, until [`Launched::ready`] finds it is not.
    fn launch(&self, state: &mut State) -> std::io::Result<Launched<'_>> {
        // Named as this program is, where Linux would show `/proc/self/exe`.
        let name = std::env::args_os()
            .next()
            .unwrap_or_else(|| self.program.clone().into_os_string());
        let (mut process, mut pipe) = {
            let _alone = self.forking.write().unwrap_or_else(PoisonError::into_inner);
            let (read, pipe) = std::io::pipe()?;
            // glave's end alone: the helper reads its own end as any program does.
            rustix::io::ioctl_fionbio(&pipe, true)?;
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
        // Taken before it is told anything: given up here, it reads the end holding nothing.
        let said = process
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("its output was not kept"))?;
        let mut told = String::new();
        for leader in &state.live {
            let _ = writeln!(told, "+{leader}");
        }
        // Into a pipe that takes 16 KB at least, more than a thousand groups, before the helper
        // reads. One that cannot be told is killed, so it never reads the end of what it was
        // told and stops those groups.
        if let Err(error) = pipe.write_all(told.as_bytes()) {
            let _ = process.start_kill();
            return Err(error);
        }
        state.launches += 1;
        let (ready, waited) = watch::channel(None);
        state.helper = Some(Helper {
            process,
            pipe,
            launch: state.launches,
            ready: waited,
        });
        Ok(Launched {
            lifeline: self,
            launch: state.launches,
            said,
            ready,
            answered: false,
        })
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
    /// Writes `line` to the helper; false when it cannot be: the helper is gone, or reads
    /// nothing (stopped, its pipe full). The pipe never blocks, so this never waits. A helper
    /// that cannot be told is killed and forgotten, so one stopped cannot wake later, read the
    /// pipe's end and stop what glave still holds, and the next program starts another.
    fn tell(&mut self, line: &str) -> bool {
        let Some(helper) = &mut self.helper else {
            return false;
        };
        // A line is shorter than what a pipe takes in one piece, so it is written whole or
        // not at all.
        let told = !matches!(helper.process.try_wait(), Ok(Some(_)))
            && helper.pipe.write_all(line.as_bytes()).is_ok();
        if !told && let Some(mut helper) = self.helper.take() {
            let _ = helper.process.start_kill();
        }
        told
    }
}

/// A helper just started, until it says it is ready. Dropped before then (it did not, or
/// the start of the program waiting for it was given up), the helper is stopped and
/// forgotten, so the next program glave starts tries again rather than tell it anything.
/// Programs waiting for it are refused if it did not say it is ready, and start another if
/// they were left unanswered.
struct Launched<'a> {
    lifeline: &'a Lifeline,
    /// Its number among the helpers glave started.
    launch: u64,
    said: ChildStdout,
    /// Whether it said it is ready, for the programs waiting for it.
    ready: watch::Sender<Option<bool>>,
    /// It said it is ready.
    answered: bool,
}

impl Launched<'_> {
    /// Waits until the helper says it reads the pipe.
    async fn ready(mut self) -> Result<(), ProcessError> {
        let mut heard = Vec::new();
        let read = tokio::time::timeout(READY_WITHIN, async {
            let mut byte = [0_u8; 1];
            while heard.len() < READY.len() && self.said.read(&mut byte).await? == 1 {
                heard.extend_from_slice(&byte);
            }
            std::io::Result::Ok(())
        })
        .await;
        let answered = matches!(read, Ok(Ok(()))) && heard == READY;
        self.answered = answered;
        self.ready.send_replace(Some(answered));
        match read {
            _ if answered => Ok(()),
            Ok(Err(error)) => Err(self.lifeline.failed(error)),
            _ => Err(self.lifeline.failed(std::io::Error::other(NOT_READY))),
        }
    }
}

impl Drop for Launched<'_> {
    fn drop(&mut self) {
        if self.answered {
            return;
        }
        let mut state = self.lifeline.lock();
        if state
            .helper
            .as_ref()
            .is_some_and(|helper| helper.launch == self.launch)
            && let Some(mut helper) = state.helper.take()
        {
            let _ = helper.process.start_kill();
        }
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
