//! A thread's network namespace (`specs/sandbox.md`, "The outbound proxy"): made once for a
//! prepared sandbox whose commands are kept to the outbound proxy, in a user namespace of its
//! own, by bubblewrap, around the system's shell, which holds it and does nothing else. glave's
//! own program joins it (`enter.rs`) as the forwarder (`forward.rs`), whose port on its loopback
//! reaches the proxy, and joins it again before bubblewrap starts each of the thread's commands,
//! so they share one loopback, where a server one of them keeps running is reached by the next,
//! and whose one way out is the proxy. glave's program is only ever started from glave's own
//! process, where its path (`/proc/self/exe` on Linux) names it, never inside bubblewrap, where
//! that path names bubblewrap. It lasts as long as the prepared sandbox: dropped, the shell and
//! the forwarder are stopped with their process groups, as they are when glave ends however it
//! ends (the lifeline, and bubblewrap's `--die-with-parent`), and the namespace goes with the
//! last process in it.

use std::ffi::OsString;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use glave_ports::computer::ComputerError;
use glave_process::{Child, Processes, Program};
use tokio::io::{AsyncBufReadExt as _, AsyncRead, AsyncReadExt as _, BufReader};
use tokio::time::timeout;

use super::bubblewrap;
use super::identity::Process;
use crate::local::sandbox::{ENTER_HELPER_ARG, FORWARD_HELPER_ARG};

/// How long the shell and the forwarder each have to say they are ready, which takes them
/// milliseconds.
const READY_WITHIN: Duration = Duration::from_secs(10);

/// The most read of what either says, and of why it did not start.
const MOST_SAID: u64 = 1024;

/// A thread's network namespace, while the shell holds it and the forwarder serves in it.
#[derive(Debug)]
pub(super) struct Namespace {
    /// The shell holding it, and the forwarder in it, as recorded when they started.
    holder: Process,
    forwarder: Process,
    /// The port of its loopback the forwarder listens on.
    port: u16,
    /// Each stopped with its process group when this is dropped.
    _held: (Child, Child),
}

impl Namespace {
    /// Makes one through `processes`, with `bwrap`, and starts in it `helper`, glave's own
    /// program, forwarding to the proxy's `socket`; and checks that bubblewrap can start a
    /// command in it. Where any of it can't, commands kept to the proxy are refused.
    pub(super) async fn start(
        processes: &Processes,
        bwrap: &Path,
        helper: &Path,
        socket: &Path,
    ) -> Result<Self, ComputerError> {
        let mut shell = Program::new(bwrap);
        // Its input stays open while glave holds it, so the shell waits on it.
        shell.args(bubblewrap::holding()).piped_input();
        let (shell, said) = started_saying(processes, shell).await?;
        let holder = said
            .trim()
            .parse()
            .ok()
            .and_then(|pid| Process::of(pid).ok())
            .map(|(holder, _)| holder);
        // Never this computer's own network, whatever bubblewrap did.
        let own = Process::of(std::process::id())
            .ok()
            .map(|(own, _)| own.network);
        let Some(holder) = holder.filter(|holder| Some(holder.network) != own) else {
            return Err(refused(&format!("the shell holding it said {said:?}")));
        };
        let mut forwarding = Program::new(helper);
        forwarding
            .args(entering(&holder))
            .arg(helper)
            .arg(FORWARD_HELPER_ARG)
            .arg(socket)
            .no_input();
        let (forwarding, said) = started_saying(processes, forwarding).await?;
        let forwarder = forwarding
            .id()
            .and_then(|pid| Process::of(pid).ok())
            .map(|(forwarder, _)| forwarder)
            .filter(|forwarder| {
                (forwarder.user, forwarder.network) == (holder.user, holder.network)
            });
        let (Some(forwarder), Ok(port)) = (forwarder, said.trim().parse()) else {
            return Err(refused(&format!("the forwarder said {said:?}")));
        };
        let namespace = Self {
            holder,
            forwarder,
            port,
            _held: (shell, forwarding),
        };
        let (bwrap, helper, entering) = (bwrap.to_owned(), helper.to_owned(), entering(&holder));
        tokio::task::spawn_blocking(move || bubblewrap::starts_in(&bwrap, &helper, &entering))
            .await
            .map_err(|error| ComputerError::Failed(error.to_string()))?
            .map_err(|said| refused(&format!("bubblewrap cannot start a command in it: {said}")))?;
        Ok(namespace)
    }

    /// The port of its loopback where the proxy is reached.
    pub(super) fn port(&self) -> u16 {
        self.port
    }

    /// The arguments for glave's own program that join it, up to the program to run there.
    pub(super) fn entering(&self) -> Vec<OsString> {
        entering(&self.holder)
    }

    /// Whether a command can still join it and reach the proxy: the shell and the forwarder are
    /// still the processes recorded, in it, though either may have been stopped from outside
    /// glave.
    pub(super) fn lasts(&self) -> bool {
        self.holder.namespaces().is_some() && self.forwarder.namespaces().is_some()
    }
}

/// The arguments for glave's own program that join `holder`'s namespaces.
fn entering(holder: &Process) -> Vec<OsString> {
    let named = holder.args().map(OsString::from);
    [OsString::from(ENTER_HELPER_ARG)]
        .into_iter()
        .chain(named)
        .chain([OsString::from("--")])
        .collect()
}

/// What commands kept to the proxy get where the thread's network can't be made, saying why.
fn refused(why: &str) -> ComputerError {
    ComputerError::Unsupported(format!(
        "commands cannot be kept to glave's outbound proxy on this computer: the thread's \
         network could not be made: {why}"
    ))
}

/// Starts `program` through `processes` with nothing of glave's environment, and the first line
/// it says, once it has said one; or, stopped, why it did not.
async fn started_saying(
    processes: &Processes,
    mut program: Program,
) -> Result<(Child, String), ComputerError> {
    program
        .env_clear()
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = processes
        .spawn(&mut program)
        .await
        .map_err(|error| refused(&error.to_string()))?;
    let said = match child.stdout.take() {
        Some(stdout) => timeout(READY_WITHIN, first_line(stdout)).await.ok(),
        None => None,
    };
    if let Some(line) = said.flatten().filter(|line| !line.trim().is_empty()) {
        return Ok((child, line));
    }
    child.stop().await;
    Err(refused(&why_not(&mut child).await))
}

/// The first line `stdout` gives, at most [`MOST_SAID`] bytes of it.
async fn first_line(stdout: impl AsyncRead + Unpin) -> Option<String> {
    let mut line = String::new();
    BufReader::new(stdout.take(MOST_SAID))
        .read_line(&mut line)
        .await
        .ok()?;
    Some(line)
}

/// What a program that did not start said, once it has ended.
async fn why_not(child: &mut Child) -> String {
    let mut said = Vec::new();
    if let Some(stderr) = child.stderr.take() {
        let _ = timeout(READY_WITHIN, stderr.take(MOST_SAID).read_to_end(&mut said)).await;
    }
    let said = String::from_utf8_lossy(&said);
    match said.trim() {
        "" => "it said nothing".to_owned(),
        said => said.to_owned(),
    }
}
