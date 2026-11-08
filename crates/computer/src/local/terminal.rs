//! A person's terminals on this machine (`specs/computers.md`, "Terminals"): their shell on a
//! pseudo-terminal, started by glave's own program (`terminal_helper`), which makes the
//! terminal its own in a session of its own, then becomes the shell, through glave-process,
//! so it is recorded and stopped with glave however glave ends. What it prints is read as it
//! comes into a ring of the latest [`TERMINAL_KEPT`] bytes. A terminal is hung up when it is
//! closed, or when nothing read or wrote it for [`IDLE`].

mod helper;
mod ring;

pub use helper::terminal_helper;
pub(crate) use helper::TERMINAL_HELPER_ARG;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::ErrorKind;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use glave_ports::computer::{
    Bytes, ComputerError, ComputerId, IDLE, MAX_TERMINALS, MAX_WAIT, Screenful, TERMINAL_CHUNK,
    TERMINAL_KEPT, TerminalId, TerminalSize,
};
use glave_process::{Child, Processes, Program};
use rustix::process::{Pid, Signal, kill_process_group};
use tokio::net::unix::pipe;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use ring::Ring;

/// How much is read from a terminal at once.
const READ_AT_ONCE: usize = 16 * 1024;

/// How long what a shell printed as it exited has to be read before its terminal ends.
const DRAIN: Duration = Duration::from_millis(500);

/// How long a write waits for a shell that takes nothing in.
const TAKES_WITHIN: Duration = Duration::from_secs(30);

/// How long closing a terminal waits for it to be hung up: two of glave-process's graces, and
/// a moment.
const CLOSES_WITHIN: Duration = Duration::from_secs(10);

/// What a terminal starts with: glave's own program, the processes it is recorded among, and
/// the environment its shell gets.
pub(crate) struct Start<'a> {
    pub(crate) helper: Option<&'a Path>,
    pub(crate) processes: &'a Processes,
    pub(crate) environment: &'a [(OsString, OsString)],
}

/// This machine's open terminals.
#[derive(Debug, Default)]
pub(crate) struct Terminals {
    open: Mutex<BTreeMap<TerminalId, Terminal>>,
}

/// One open terminal: its computer, what it shares with its pump, and the pump.
#[derive(Debug)]
struct Terminal {
    computer: ComputerId,
    shared: Arc<Shared>,
    /// Reads what its shell prints and owns the shell; aborted, which stops the shell's group,
    /// when the terminal is dropped.
    pump: JoinHandle<()>,
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.pump.abort();
    }
}

/// What a terminal's callers and its pump share.
#[derive(Debug)]
struct Shared {
    /// The pseudo-terminal's side glave holds, as it is written (its pump reads it).
    input: pipe::Sender,
    printed: Mutex<Ring>,
    /// Told whenever it printed or ended.
    changed: Notify,
    /// Told to hang it up.
    hang_up: Notify,
    /// When something last read or wrote it.
    used: Mutex<Instant>,
}

impl Shared {
    fn touch(&self) {
        *self.used.lock().unwrap_or_else(PoisonError::into_inner) = Instant::now();
    }

    fn idle_at(&self) -> Instant {
        *self.used.lock().unwrap_or_else(PoisonError::into_inner) + IDLE
    }

    fn ring(&self) -> std::sync::MutexGuard<'_, Ring> {
        self.printed.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn print(&self, bytes: &[u8]) {
        self.ring().push(bytes);
        self.changed.notify_waiters();
    }

    fn ended(&self) {
        self.ring().end();
        self.changed.notify_waiters();
    }
}

impl Terminals {
    /// Opens a terminal of `size` at `folder` on `computer`, its shell started as `start` says.
    pub(crate) async fn open(
        &self,
        computer: ComputerId,
        folder: &str,
        size: TerminalSize,
        start: &Start<'_>,
    ) -> Result<TerminalId, ComputerError> {
        let size = size.checked()?;
        let folder = super::paths::folder(folder).await?;
        let Some(helper) = start.helper else {
            return Err(ComputerError::Unsupported(
                "this computer opens terminals only with glave's own program to start them".into(),
            ));
        };
        if self.lock().len() >= MAX_TERMINALS {
            return Err(ComputerError::Refused(
                format!("{MAX_TERMINALS} terminals are already open here; close one first").into(),
            ));
        }
        let (master, terminal) = pseudo_terminal(size).map_err(failed)?;
        let mut program = Program::new(helper);
        program
            .arg(TERMINAL_HELPER_ARG)
            .arg(shell(start.environment))
            .arg("-l")
            .current_dir(&folder)
            .env_clear()
            .envs(start.environment.iter().map(|(name, value)| (name, value)))
            .env("TERM", "xterm-256color")
            .in_terminal(terminal)
            .map_err(failed)?;
        let child = start.processes.spawn(&mut program).await;
        // Its side of the terminal closes here, so the shell's end is read as the terminal's.
        drop(program);
        let child = child.map_err(|error| ComputerError::Failed(error.to_string()))?;
        // Read and written without blocking, as tokio's pipes are, through two handles on it.
        rustix::fs::fcntl_setfl(&master, rustix::fs::OFlags::NONBLOCK).map_err(failed)?;
        let output = pipe::Receiver::from_owned_fd_unchecked(master.try_clone().map_err(failed)?)
            .map_err(failed)?;
        let input = pipe::Sender::from_owned_fd_unchecked(master).map_err(failed)?;
        let shared = Arc::new(Shared {
            input,
            printed: Mutex::new(Ring::new(TERMINAL_KEPT)),
            changed: Notify::new(),
            hang_up: Notify::new(),
            used: Mutex::new(Instant::now()),
        });
        let id = TerminalId::from_uuid(super::new_id()?);
        let pump = tokio::spawn(pump(Arc::clone(&shared), output, child));
        let terminal = Terminal {
            computer,
            shared,
            pump,
        };
        let mut open = self.lock();
        if open.len() >= MAX_TERMINALS {
            return Err(ComputerError::Refused(
                format!("{MAX_TERMINALS} terminals are already open here; close one first").into(),
            ));
        }
        open.insert(id, terminal);
        Ok(id)
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
        match tokio::time::timeout(TAKES_WITHIN, write_all(&shared.input, input)).await {
            Ok(written) => written.map_err(failed),
            Err(_) => Err(ComputerError::Failed(format!(
                "the terminal took nothing in for {}",
                glave_ports::spoken(TAKES_WITHIN)
            ))),
        }
    }

    /// What the terminal printed from `from`, waiting up to `wait` for something when nothing
    /// is there yet.
    pub(crate) async fn read(
        &self,
        computer: ComputerId,
        id: TerminalId,
        from: u64,
        wait: Duration,
    ) -> Result<Screenful, ComputerError> {
        let shared = self.shared(computer, id)?;
        shared.touch();
        let deadline = Instant::now() + wait.min(MAX_WAIT);
        loop {
            let changed = shared.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            {
                let ring = shared.ring();
                if ring.ready(from) || Instant::now() >= deadline {
                    return Ok(ring.read(from, TERMINAL_CHUNK));
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
        rustix::termios::tcsetwinsize(&shared.input, winsize(size)).map_err(failed)
    }

    /// Hangs the terminal up and forgets it.
    pub(crate) async fn close(
        &self,
        computer: ComputerId,
        id: TerminalId,
    ) -> Result<(), ComputerError> {
        let mut terminal = {
            let mut open = self.lock();
            match open.get(&id) {
                Some(terminal) if terminal.computer == computer => open.remove(&id),
                _ => None,
            }
        }
        .ok_or_else(|| no_terminal(id))?;
        terminal.shared.hang_up.notify_one();
        let _ = tokio::time::timeout(CLOSES_WITHIN, &mut terminal.pump).await;
        Ok(())
    }

    /// Hangs up every terminal open on `computer`, as it is destroyed.
    pub(crate) fn close_all(&self, computer: ComputerId) {
        self.lock().retain(|_, terminal| terminal.computer != computer);
    }

    fn shared(&self, computer: ComputerId, id: TerminalId) -> Result<Arc<Shared>, ComputerError> {
        match self.lock().get(&id) {
            Some(terminal) if terminal.computer == computer => Ok(Arc::clone(&terminal.shared)),
            _ => Err(no_terminal(id)),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<TerminalId, Terminal>> {
        self.open.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn no_terminal(id: TerminalId) -> ComputerError {
    ComputerError::NotFound(format!("no terminal {id}"))
}

fn failed(error: impl std::fmt::Display) -> ComputerError {
    ComputerError::Failed(format!("the terminal failed: {error}"))
}

fn winsize(size: TerminalSize) -> rustix::termios::Winsize {
    rustix::termios::Winsize {
        ws_row: size.rows,
        ws_col: size.cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    }
}

/// A new pseudo-terminal of `size`: the side glave holds, and the side its shell runs in.
fn pseudo_terminal(size: TerminalSize) -> std::io::Result<(OwnedFd, OwnedFd)> {
    use rustix::fs::{Mode, OFlags};
    use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};
    let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY)?;
    // Where the system can't open it so, at once: no program glave starts holds it open.
    rustix::io::fcntl_setfd(&master, rustix::io::FdFlags::CLOEXEC)?;
    grantpt(&master)?;
    unlockpt(&master)?;
    let name = ptsname(&master, Vec::new())?;
    let flags = OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC;
    let terminal = rustix::fs::open(name.as_c_str(), flags, Mode::empty())?;
    rustix::termios::tcsetwinsize(&master, winsize(size))?;
    Ok((master, terminal))
}

/// The person's shell: the one their environment names, else bash, else sh.
fn shell(environment: &[(OsString, OsString)]) -> PathBuf {
    let named = environment
        .iter()
        .find(|(name, _)| name == "SHELL")
        .map(|(_, value)| PathBuf::from(value))
        .filter(|path| path.is_absolute() && path.is_file());
    named
        .or_else(|| Some(PathBuf::from("/bin/bash")).filter(|path| path.is_file()))
        .unwrap_or_else(|| PathBuf::from("/bin/sh"))
}

/// Writes all of `input` to the terminal.
async fn write_all(terminal: &pipe::Sender, input: &[u8]) -> std::io::Result<()> {
    let mut rest = input;
    while !rest.is_empty() {
        terminal.writable().await?;
        match terminal.try_write(rest) {
            Ok(written) => rest = rest.get(written..).unwrap_or_default(),
            Err(error) if error.kind() == ErrorKind::WouldBlock => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// How a terminal's pump stopped reading it.
enum Stopped {
    /// It was closed, or nothing used it for [`IDLE`].
    HungUp,
    /// Its shell exited.
    Exited,
    /// Its shell's side of the terminal closed.
    Closed,
}

/// Reads what the terminal's shell prints into its ring until it is hung up, its shell exits
/// or its side closes, then stops the shell's group and says the terminal ended.
async fn pump(shared: Arc<Shared>, output: pipe::Receiver, mut child: Child) {
    let mut buffer = vec![0_u8; READ_AT_ONCE];
    let stopped = loop {
        let idle_at = shared.idle_at();
        tokio::select! {
            ready = output.readable() => match ready.and_then(|()| output.try_read(&mut buffer)) {
                Ok(read) if read > 0 => shared.print(buffer.get(..read).unwrap_or_default()),
                Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                // Its other side closed: the end (Linux says so as an error).
                Ok(_) | Err(_) => break Stopped::Closed,
            },
            _ = child.wait() => break Stopped::Exited,
            () = shared.hang_up.notified() => break Stopped::HungUp,
            () = tokio::time::sleep_until(idle_at) => {
                if shared.idle_at() <= Instant::now() {
                    break Stopped::HungUp;
                }
            }
        }
    };
    match stopped {
        Stopped::HungUp => hang_up(&shared, &mut child).await,
        Stopped::Exited => {
            let _ = tokio::time::timeout(DRAIN, drain(&shared, &output, &mut buffer)).await;
            child.stop_left().await;
        }
        Stopped::Closed => child.stop().await,
    }
    shared.ended();
}

/// Reads what is left to read, until the terminal's other side closes.
async fn drain(shared: &Shared, output: &pipe::Receiver, buffer: &mut [u8]) {
    loop {
        match output.readable().await.and_then(|()| output.try_read(buffer)) {
            Ok(read) if read > 0 => shared.print(buffer.get(..read).unwrap_or_default()),
            Err(error) if error.kind() == ErrorKind::WouldBlock => {}
            Ok(_) | Err(_) => return,
        }
    }
}

/// Hangs the terminal up as a terminal is hung up: SIGHUP to the shell, which passes it to its
/// jobs, and to whatever runs in front, then the shell's group stopped whole, and whatever ran
/// in front killed.
async fn hang_up(shared: &Shared, child: &mut Child) {
    let shell = child.id().and_then(|id| Pid::from_raw(i32::try_from(id).ok()?));
    let front = rustix::termios::tcgetpgrp(&shared.input).ok();
    for group in [shell, front].into_iter().flatten() {
        let _ = kill_process_group(group, Signal::HUP);
    }
    child.stop().await;
    if let Some(front) = front.filter(|front| Some(*front) != shell) {
        let _ = kill_process_group(front, Signal::KILL);
    }
}

#[cfg(test)]
mod tests;
