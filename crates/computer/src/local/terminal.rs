//! A person's terminals on this machine (`specs/computers.md`, "Terminals"): their login shell
//! on a pseudo-terminal, started by glave's own program (`terminal_helper`), which makes the
//! terminal its own in a session of its own, then becomes the shell, through glave-process, so
//! it is recorded and stopped with glave. What it prints is read as it comes into a ring of the
//! latest [`TERMINAL_KEPT`] bytes. A terminal ends, with everything in its session, when it is
//! closed, its shell exits, it is dropped, or nothing used it for [`IDLE`].

mod helper;
mod pump;
mod ring;
mod session;
mod shell;

pub(crate) use helper::TERMINAL_HELPER_ARG;
pub use helper::terminal_helper;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use glave_ports::computer::{
    Bytes, ComputerError, ComputerId, IDLE, MAX_OWNED_TERMINALS, MAX_TERMINALS, MAX_WAIT,
    Screenful, TERMINAL_CHUNK, TERMINAL_KEPT, TerminalId, TerminalInfo, TerminalSize,
};
use glave_process::{Processes, Program};
use rustix::process::{Pid, Signal, kill_process, kill_process_group};
use tokio::net::unix::pipe;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use ring::Ring;
use session::Session;

/// How long a write waits for a shell that takes nothing in.
const TAKES_WITHIN: Duration = Duration::from_secs(30);

/// How long closing a terminal waits for it to end: a second's grace, glave-process's two,
/// and a moment.
const CLOSES_WITHIN: Duration = Duration::from_secs(10);

/// What a terminal starts with: glave's own program, the processes it is recorded among, and
/// the environment its shell's few variables come from.
pub(crate) struct Start<'a> {
    pub(crate) helper: Option<&'a Path>,
    pub(crate) processes: &'a Processes,
    pub(crate) environment: &'a [(OsString, OsString)],
}

/// This machine's open terminals, and the places held for those being opened.
#[derive(Debug, Default)]
pub(crate) struct Terminals {
    held: Mutex<Held>,
}

#[derive(Debug, Default)]
struct Held {
    open: BTreeMap<TerminalId, Terminal>,
    /// Those being opened, by owner.
    opening: Vec<String>,
}

impl Held {
    fn owned(&self, owner: &str) -> usize {
        let open = self
            .open
            .values()
            .filter(|each| each.owner == owner)
            .count();
        let opening = self.opening.iter().filter(|each| *each == owner).count();
        open.saturating_add(opening)
    }

    fn all(&self) -> usize {
        self.open.len().saturating_add(self.opening.len())
    }
}

/// One open terminal: whose it is, where it started, what it shares with its pump, and the pump.
#[derive(Debug)]
struct Terminal {
    computer: ComputerId,
    owner: String,
    folder: String,
    shared: Arc<Shared>,
    /// Reads what its shell prints and owns the shell.
    pump: JoinHandle<()>,
}

impl Drop for Terminal {
    /// Dropped before it ended (its computer destroyed, glave ending): everything in its session
    /// goes at once, then its pump, whose shell goes with its group.
    fn drop(&mut self) {
        if !self.shared.ended.load(Ordering::SeqCst) {
            let session = &self.shared.session;
            let _ = kill_process_group(session.leader(), Signal::HUP);
            for member in session.members_now() {
                let _ = kill_process(member, Signal::KILL);
            }
        }
        self.pump.abort();
    }
}

/// What a terminal's callers and its pump share.
#[derive(Debug)]
struct Shared {
    /// The pseudo-terminal's side glave holds, as it is written (its pump reads it).
    input: pipe::Sender,
    session: Session,
    printed: Mutex<Ring>,
    /// Told whenever it printed or ended.
    changed: Notify,
    /// Told to hang it up.
    hang_up: Notify,
    /// When it was last used: something printed, read or written.
    used: Mutex<Instant>,
    /// Whether its pump has ended it, and everything in its session.
    ended: AtomicBool,
}

impl Shared {
    fn touch(&self) {
        *self.used.lock().unwrap_or_else(PoisonError::into_inner) = Instant::now();
    }

    fn idle_at(&self) -> Instant {
        *self.used.lock().unwrap_or_else(PoisonError::into_inner) + IDLE
    }

    fn ring(&self) -> MutexGuard<'_, Ring> {
        self.printed.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn print(&self, bytes: &[u8]) {
        self.ring().push(bytes);
        self.touch();
        self.changed.notify_waiters();
    }

    fn end(&self) {
        self.ring().end();
        self.changed.notify_waiters();
    }
}

/// A place held for a terminal of `owner` being opened, let go of once it is open or failed.
struct Opening<'a> {
    terminals: &'a Terminals,
    owner: String,
}

impl Drop for Opening<'_> {
    fn drop(&mut self) {
        let mut held = self.terminals.lock();
        if let Some(at) = held.opening.iter().position(|each| *each == self.owner) {
            held.opening.swap_remove(at);
        }
    }
}

fn refused(most: usize, whose: &str) -> ComputerError {
    ComputerError::Refused(
        format!("{most} terminals are already open {whose}; close one first").into(),
    )
}

impl Terminals {
    /// Opens a terminal of `size` at `folder` on `computer` for `owner`, its shell started as
    /// `start` says.
    pub(crate) async fn open(
        &self,
        computer: ComputerId,
        (folder, owner): (&str, &str),
        size: TerminalSize,
        start: &Start<'_>,
    ) -> Result<TerminalId, ComputerError> {
        let size = size.checked()?;
        let root = super::paths::folder(folder).await?;
        let Some(helper) = start.helper else {
            return Err(ComputerError::Unsupported(
                "this computer opens terminals only with glave's own program to start them".into(),
            ));
        };
        let shell = shell::login_shell(start.environment)?;
        let _place = self.hold(owner)?;
        let (master, terminal, tty) = shell::pseudo_terminal(size).map_err(failed)?;
        let mut program = Program::new(helper);
        program
            .arg(TERMINAL_HELPER_ARG)
            .arg(&shell)
            .args(shell::login_args(&shell))
            .current_dir(&root)
            .env_clear()
            .envs(shell::shell_environment(start.environment))
            .in_terminal(terminal)
            .map_err(failed)?;
        let child = start.processes.spawn(&mut program).await;
        // Its side of the terminal closes here, so the shell's end is read as the terminal's.
        drop(program);
        let child = child.map_err(|error| ComputerError::Failed(error.to_string()))?;
        let leader = child
            .id()
            .and_then(|id| Pid::from_raw(i32::try_from(id).ok()?))
            .ok_or_else(|| failed("its shell has no process id"))?;
        // Read and written without blocking, as tokio's pipes are, through two handles on it.
        rustix::fs::fcntl_setfl(&master, rustix::fs::OFlags::NONBLOCK).map_err(failed)?;
        let output = pipe::Receiver::from_owned_fd_unchecked(master.try_clone().map_err(failed)?)
            .map_err(failed)?;
        let input = pipe::Sender::from_owned_fd_unchecked(master).map_err(failed)?;
        let shared = Arc::new(Shared {
            input,
            session: Session::new(leader, tty),
            printed: Mutex::new(Ring::new(TERMINAL_KEPT)),
            changed: Notify::new(),
            hang_up: Notify::new(),
            used: Mutex::new(Instant::now()),
            ended: AtomicBool::new(false),
        });
        let id = TerminalId::from_uuid(super::new_id()?);
        let pump = tokio::spawn(pump::pump(Arc::clone(&shared), output, child));
        let terminal = Terminal {
            computer,
            owner: owner.to_owned(),
            folder: root.to_string_lossy().into_owned(),
            shared,
            pump,
        };
        self.lock().open.insert(id, terminal);
        Ok(id)
    }

    /// Holds a place for one more terminal of `owner`, when there is room for it.
    fn hold(&self, owner: &str) -> Result<Opening<'_>, ComputerError> {
        let mut held = self.lock();
        if held.all() >= MAX_TERMINALS {
            return Err(refused(MAX_TERMINALS, "here"));
        }
        if held.owned(owner) >= MAX_OWNED_TERMINALS {
            return Err(refused(MAX_OWNED_TERMINALS, "for it"));
        }
        held.opening.push(owner.to_owned());
        Ok(Opening {
            terminals: self,
            owner: owner.to_owned(),
        })
    }

    /// The terminals open on `computer` for `owner`, oldest first.
    pub(crate) fn list(&self, computer: ComputerId, owner: &str) -> Vec<TerminalInfo> {
        self.lock()
            .open
            .iter()
            .filter(|(_, each)| each.computer == computer && each.owner == owner)
            .map(|(id, each)| TerminalInfo {
                id: *id,
                folder: each.folder.clone(),
                ended: each.shared.ended.load(Ordering::SeqCst),
            })
            .collect()
    }

    /// Types `input` into the terminal.
    pub(crate) async fn write(
        &self,
        computer: ComputerId,
        id: TerminalId,
        input: &Bytes,
    ) -> Result<(), ComputerError> {
        let input = input.as_slice();
        if input.len() > TERMINAL_CHUNK {
            return Err(ComputerError::Refused(
                format!("a terminal takes at most {TERMINAL_CHUNK} bytes at once").into(),
            ));
        }
        let shared = self.shared(computer, id)?;
        shared.touch();
        match tokio::time::timeout(TAKES_WITHIN, pump::write_all(&shared.input, input)).await {
            Ok(written) => written.map_err(failed),
            Err(_) => Err(ComputerError::Failed(format!(
                "the terminal took nothing in for {}",
                glave_ports::spoken(TAKES_WITHIN)
            ))),
        }
    }

    /// What the terminal printed from `from`, waiting up to `wait` for something when nothing
    /// is there yet. A read that finds something uses the terminal; one that finds nothing
    /// does not.
    pub(crate) async fn read(
        &self,
        computer: ComputerId,
        id: TerminalId,
        from: u64,
        wait: Duration,
    ) -> Result<Screenful, ComputerError> {
        let shared = self.shared(computer, id)?;
        let deadline = Instant::now() + wait.min(MAX_WAIT);
        loop {
            let changed = shared.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            {
                let ring = shared.ring();
                if ring.ready(from) || Instant::now() >= deadline {
                    let read = ring.read(from, TERMINAL_CHUNK);
                    drop(ring);
                    if !read.output.as_slice().is_empty() {
                        shared.touch();
                    }
                    return Ok(read);
                }
            }
            let left = deadline.saturating_duration_since(Instant::now());
            let _ = tokio::time::timeout(left, changed).await;
        }
    }

    /// Gives the terminal another size, which its shell is told of.
    pub(crate) fn resize(
        &self,
        computer: ComputerId,
        id: TerminalId,
        size: TerminalSize,
    ) -> Result<(), ComputerError> {
        let size = size.checked()?;
        let shared = self.shared(computer, id)?;
        shared.touch();
        rustix::termios::tcsetwinsize(&shared.input, shell::winsize(size)).map_err(failed)
    }

    /// Hangs the terminal up, with everything in its session, and forgets it.
    pub(crate) async fn close(
        &self,
        computer: ComputerId,
        id: TerminalId,
    ) -> Result<(), ComputerError> {
        let mut terminal = {
            let mut held = self.lock();
            match held.open.get(&id) {
                Some(terminal) if terminal.computer == computer => held.open.remove(&id),
                _ => None,
            }
        }
        .ok_or_else(|| no_terminal(id))?;
        terminal.shared.hang_up.notify_one();
        let _ = tokio::time::timeout(CLOSES_WITHIN, &mut terminal.pump).await;
        Ok(())
    }

    /// Ends every terminal open on `computer`, as it is destroyed.
    pub(crate) fn close_all(&self, computer: ComputerId) {
        self.lock()
            .open
            .retain(|_, terminal| terminal.computer != computer);
    }

    fn shared(&self, computer: ComputerId, id: TerminalId) -> Result<Arc<Shared>, ComputerError> {
        match self.lock().open.get(&id) {
            Some(terminal) if terminal.computer == computer => Ok(Arc::clone(&terminal.shared)),
            _ => Err(no_terminal(id)),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Held> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn no_terminal(id: TerminalId) -> ComputerError {
    ComputerError::NotFound(format!("no terminal {id}"))
}

fn failed(error: impl std::fmt::Display) -> ComputerError {
    ComputerError::Failed(format!("the terminal failed: {error}"))
}

#[cfg(test)]
mod tests;
