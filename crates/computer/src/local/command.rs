//! Running one command at a workspace root: bash in a fresh process group, output streamed
//! as it arrives, stopped with everything it started on a timeout or a cancel.

use std::process::Stdio;
use std::time::{Duration, Instant};

use glave_ports::{Cancellation, Command, ComputerError, Ending, Finished, Output};
use tokio::io::AsyncReadExt as _;
use tokio::process::{Child, ChildStdout};

use super::registry::Opened;
use super::sandbox::Prepared;

const DEFAULT_TIMEOUT_SECS: u64 = 120;
const MAX_TIMEOUT_SECS: u64 = 600;
/// How long to keep reading after the command exits, in case a background child it started
/// still holds the output open.
const DRAIN: Duration = Duration::from_millis(500);
/// How long a stopped command gets between SIGTERM and SIGKILL.
const GRACE: Duration = Duration::from_secs(3);

pub(crate) async fn run(
    opened: &Opened,
    command: &Command,
    output: &dyn Output,
    cancel: &dyn Cancellation,
) -> Result<Finished, ComputerError> {
    if command.script.trim().is_empty() {
        return Err(ComputerError::Refused("the command is empty".into()));
    }
    let limit = command
        .timeout_secs
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
        .clamp(1, MAX_TIMEOUT_SECS);
    let sandbox = match &command.sandbox {
        Some(sandbox) => Some(opened.sandbox(sandbox).await?),
        None => None,
    };
    let started = Instant::now();
    let mut child = bash(opened, &command.script, sandbox.as_deref())
        .spawn()
        .map_err(|error| ComputerError::Failed(format!("could not start bash: {error}")))?;
    let stdout = child.stdout.take();
    let pump = pump(stdout, output);
    let waiting = wait(&mut child, limit, cancel);
    tokio::pin!(pump, waiting);
    let (ending, pumped) = tokio::select! {
        ending = &mut waiting => (ending, false),
        () = &mut pump => (waiting.await, true),
    };
    if !pumped {
        let _ = tokio::time::timeout(DRAIN, pump).await;
    }
    let millis = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok(Finished {
        ending: ending?,
        millis,
    })
}

/// `script` run by bash at the workspace root, in the sandbox if there is one, with the
/// workspace's environment and output that reads well as text.
fn bash(opened: &Opened, script: &str, sandbox: Option<&Prepared>) -> tokio::process::Command {
    let mut command = match sandbox {
        Some(sandbox) => {
            let (program, args) = sandbox.wrapper();
            let mut command = tokio::process::Command::new(program);
            command.args(args).arg("bash");
            command
        }
        None => tokio::process::Command::new("bash"),
    };
    command
        .arg("-c")
        .arg(format!("exec 2>&1\n{script}"))
        .current_dir(&opened.root)
        .env_clear()
        .envs(opened.environment.iter().map(|(name, value)| (name, value)))
        .envs(sandbox.map(|sandbox| ("TMPDIR", sandbox.temp())))
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .env("PAGER", "cat")
        .env("GIT_PAGER", "cat")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    command
}

/// Waits for the command to end, stopping it on a timeout or a cancel.
async fn wait(
    child: &mut Child,
    limit: u64,
    cancel: &dyn Cancellation,
) -> Result<Ending, ComputerError> {
    let waited = glave_ports::unless_cancelled(
        cancel,
        tokio::time::timeout(Duration::from_secs(limit), child.wait()),
    )
    .await;
    match waited {
        Some(Ok(Ok(status))) => Ok(status
            .code()
            .map_or(Ending::Signalled, |code| Ending::Exited { code })),
        Some(Ok(Err(error))) => Err(ComputerError::Failed(format!(
            "could not wait for the command: {error}"
        ))),
        Some(Err(_)) => {
            stop(child).await;
            Ok(Ending::TimedOut { after_secs: limit })
        }
        None => {
            stop(child).await;
            Ok(Ending::Cancelled)
        }
    }
}

/// Stops a command and everything it started: SIGTERM to its process group, then SIGKILL if
/// it is still running after a grace period.
async fn stop(child: &mut Child) {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        signal_group("-TERM", pid).await;
        if tokio::time::timeout(GRACE, child.wait()).await.is_err() {
            signal_group("-KILL", pid).await;
        }
    }
    let _ = child.kill().await;
}

#[cfg(unix)]
async fn signal_group(signal: &str, pid: u32) {
    let _ = tokio::process::Command::new("kill")
        .arg(signal)
        .arg("--")
        .arg(format!("-{pid}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
}

/// Streams the command's output to `output` as text until it ends.
async fn pump(stdout: Option<ChildStdout>, output: &dyn Output) {
    let Some(mut stdout) = stdout else {
        return;
    };
    let mut buffer = vec![0u8; 16 * 1024];
    let mut text = Utf8::default();
    while let Ok(read) = stdout.read(&mut buffer).await {
        let Some(chunk) = buffer.get(..read).filter(|chunk| !chunk.is_empty()) else {
            break;
        };
        let decoded = text.push(chunk);
        if !decoded.is_empty() {
            output.write(&decoded).await;
        }
    }
    let rest = text.finish();
    if !rest.is_empty() {
        output.write(&rest).await;
    }
}

/// Turns bytes into text without splitting a character across chunks.
#[derive(Debug, Default)]
pub(crate) struct Utf8 {
    pending: Vec<u8>,
}

impl Utf8 {
    pub(crate) fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let cut = match std::str::from_utf8(&self.pending) {
            Err(error) if error.error_len().is_none() => error.valid_up_to(),
            _ => self.pending.len(),
        };
        let rest = self.pending.split_off(cut);
        let text = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending = rest;
        text
    }

    pub(crate) fn finish(&mut self) -> String {
        let text = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        text
    }
}
