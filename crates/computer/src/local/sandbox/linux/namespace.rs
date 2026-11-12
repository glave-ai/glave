//! A thread's network namespace (`specs/sandbox.md`, "The outbound proxy"): made once for a
//! prepared sandbox whose commands are kept to the outbound proxy, in a user namespace of its
//! own, by bubblewrap, and held there by glave's own program, the forwarder (`forward.rs`),
//! whose port on its loopback reaches the proxy. Each of the thread's commands joins it
//! (`enter.rs`) before bubblewrap starts it, so they share one loopback, where a server one of
//! them keeps running is reached by the next, and whose one way out is the proxy. It lasts as
//! long as the prepared sandbox: dropped, the forwarder is stopped with its process group, as
//! it is when glave ends however it ends (the lifeline, and bubblewrap's `--die-with-parent`),
//! and the namespace goes with the last process in it.

use std::ffi::OsString;
use std::os::unix::fs::MetadataExt as _;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use glave_ports::computer::ComputerError;
use glave_process::{Child, Processes, Program};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, BufReader};
use tokio::time::timeout;

use super::bubblewrap;
use crate::local::sandbox::ENTER_HELPER_ARG;

/// How long the forwarder has to say it is ready, which takes it milliseconds.
const READY_WITHIN: Duration = Duration::from_secs(10);

/// The most read of what the forwarder says, and of why it did not start.
const MOST_SAID: u64 = 1024;

/// A thread's network namespace, while its forwarder holds it.
#[derive(Debug)]
pub(super) struct Namespace {
    /// The forwarder's process number, and the inodes of the user and network namespaces it
    /// is in, which name them while they last.
    pid: u32,
    user: u64,
    network: u64,
    /// The port of its loopback the forwarder listens on.
    port: u16,
    /// bubblewrap, and in its process group the forwarder; stopped when this is dropped.
    _holder: Child,
}

impl Namespace {
    /// Makes one through `processes`, with `bwrap`, held by `helper`, glave's own program,
    /// forwarding to the proxy's `socket`; and checks that bubblewrap can start a command in it.
    pub(super) async fn start(
        processes: &Processes,
        bwrap: &Path,
        helper: &Path,
        socket: &Path,
    ) -> Result<Self, ComputerError> {
        let mut program = Program::new(bwrap);
        program
            .args(bubblewrap::holding(helper, socket))
            .env_clear()
            .no_input()
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut holder = processes
            .spawn(&mut program)
            .await
            .map_err(|error| not_made(&error.to_string()))?;
        let said = match holder.stdout.take() {
            Some(stdout) => timeout(READY_WITHIN, first_line(stdout)).await.ok(),
            None => None,
        };
        // Never this computer's own network, whatever bubblewrap did.
        let own = inode(std::process::id(), "net");
        let held = said
            .flatten()
            .as_deref()
            .and_then(ready)
            .and_then(|(pid, port)| {
                let (user, network) = (inode(pid, "user")?, inode(pid, "net")?);
                Some((pid, user, network, port))
            })
            .filter(|&(_, _, network, _)| Some(network) != own);
        let Some((pid, user, network, port)) = held else {
            holder.stop().await;
            return Err(not_made(&why_not(&mut holder).await));
        };
        let namespace = Self {
            pid,
            user,
            network,
            port,
            _holder: holder,
        };
        let (bwrap, entering) = (bwrap.to_owned(), namespace.entering());
        let helper = helper.to_owned();
        tokio::task::spawn_blocking(move || bubblewrap::starts_in(&bwrap, &helper, &entering))
            .await
            .map_err(|error| ComputerError::Failed(error.to_string()))?
            .map_err(|said| {
                ComputerError::Unsupported(format!(
                    "commands cannot be kept to glave's outbound proxy on this computer: \
                     bubblewrap cannot start a command in the thread's network here: {said}"
                ))
            })?;
        Ok(namespace)
    }

    /// The port of its loopback where the proxy is reached.
    pub(super) fn port(&self) -> u16 {
        self.port
    }

    /// The arguments for glave's own program that join it, up to the command to run there.
    pub(super) fn entering(&self) -> Vec<OsString> {
        [
            ENTER_HELPER_ARG.to_owned(),
            self.pid.to_string(),
            self.user.to_string(),
            self.network.to_string(),
            "--".to_owned(),
        ]
        .map(OsString::from)
        .into()
    }

    /// Whether it is still held, so the next command can join it: its forwarder may have been
    /// stopped from outside glave.
    pub(super) fn lasts(&self) -> bool {
        inode(self.pid, "net") == Some(self.network) && inode(self.pid, "user") == Some(self.user)
    }
}

/// What not starting the forwarder fails with.
fn not_made(why: &str) -> ComputerError {
    ComputerError::Failed(format!("could not make the thread's network: {why}"))
}

/// The first line `stdout` gives, at most [`MOST_SAID`] bytes of it.
async fn first_line(stdout: impl tokio::io::AsyncRead + Unpin) -> Option<String> {
    let mut line = String::new();
    BufReader::new(stdout.take(MOST_SAID))
        .read_line(&mut line)
        .await
        .ok()?;
    Some(line)
}

/// The forwarder's process number and port, from the line it says them on.
fn ready(line: &str) -> Option<(u32, u16)> {
    let (pid, port) = line.trim().split_once(' ')?;
    Some((pid.parse().ok()?, port.parse().ok()?))
}

/// The inode of the namespace of `kind` that process `pid` is in, while there is one.
fn inode(pid: u32, kind: &str) -> Option<u64> {
    std::fs::metadata(format!("/proc/{pid}/ns/{kind}"))
        .ok()
        .map(|found| found.ino())
}

/// What a holder that did not start said, once it has ended.
async fn why_not(holder: &mut Child) -> String {
    let mut said = Vec::new();
    if let Some(stderr) = holder.stderr.take() {
        let _ = timeout(READY_WITHIN, stderr.take(MOST_SAID).read_to_end(&mut said)).await;
    }
    let said = String::from_utf8_lossy(&said);
    match said.trim() {
        "" => "it said nothing".to_owned(),
        said => said.to_owned(),
    }
}
