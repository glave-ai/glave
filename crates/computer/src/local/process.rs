//! A command's process on this machine once it runs: its output read as text, a chunk at a
//! time, without losing a chunk when reading stops part way; and its process group stopped
//! whole, children it started included.

use std::time::Duration;

use glave_ports::Output;
use tokio::io::AsyncReadExt as _;
use tokio::process::{Child, ChildStdout};

/// How long a stopped command gets between SIGTERM and SIGKILL.
const GRACE: Duration = Duration::from_secs(3);
/// How often a stopped command's process group is checked for anything left in it.
#[cfg(unix)]
const POLL: Duration = Duration::from_millis(10);
/// How much is read from a command's output at once.
const CHUNK: usize = 16 * 1024;

/// A command's output as it is read: standard output and error together, decoded as text,
/// with what was read but not yet handed on.
#[derive(Debug)]
pub(crate) struct Stream {
    stdout: Option<ChildStdout>,
    buffer: Vec<u8>,
    text: Utf8,
    /// Decoded, and not yet taken by whoever reads the stream.
    unsent: String,
}

impl Stream {
    pub(crate) fn of(child: &mut Child) -> Self {
        Self {
            stdout: child.stdout.take(),
            buffer: vec![0u8; CHUNK],
            text: Utf8::default(),
            unsent: String::new(),
        }
    }

    /// Reads the next chunk into what is unsent; false once the output has closed. Safe to
    /// stop at any point: nothing read is lost.
    pub(crate) async fn read(&mut self) -> bool {
        let Some(stdout) = self.stdout.as_mut() else {
            return false;
        };
        match stdout.read(&mut self.buffer).await {
            Ok(read) if read > 0 => {
                let decoded = self.text.push(self.buffer.get(..read).unwrap_or_default());
                self.unsent.push_str(&decoded);
                true
            }
            _ => {
                self.stdout = None;
                let rest = self.text.finish();
                self.unsent.push_str(&rest);
                false
            }
        }
    }

    /// What was read and not yet taken.
    pub(crate) fn take(&mut self) -> String {
        std::mem::take(&mut self.unsent)
    }

    /// Hands the output to `output` as it arrives, until it closes. Stopped part way, it keeps
    /// what `output` did not take yet.
    pub(crate) async fn pump(&mut self, output: &dyn Output) {
        loop {
            if !self.unsent.is_empty() {
                output.write(&self.unsent).await;
                self.unsent.clear();
            }
            if !self.read().await && self.unsent.is_empty() {
                return;
            }
        }
    }
}

/// The process group a child leads, while it runs.
#[cfg(unix)]
pub(crate) fn group_of(child: &Child) -> Option<rustix::process::Pid> {
    child
        .id()
        .and_then(|pid| i32::try_from(pid).ok())
        .and_then(rustix::process::Pid::from_raw)
}

/// Stops a command and everything it started: SIGTERM to its process group, then SIGKILL to
/// whatever of the group is still there after a grace period. Returns once the group is gone,
/// not just bash: a child that outlives bash, or ignores SIGTERM, is stopped too.
pub(crate) async fn stop(child: &mut Child) {
    #[cfg(unix)]
    if let Some(group) = group_of(child) {
        use rustix::process::{Signal, kill_process_group};
        let _ = kill_process_group(group, Signal::TERM);
        if tokio::time::timeout(GRACE, gone(child, group))
            .await
            .is_err()
        {
            let _ = kill_process_group(group, Signal::KILL);
            // SIGKILL cannot be ignored; this waits only for the kernel to finish the job.
            let _ = tokio::time::timeout(GRACE, gone(child, group)).await;
        }
    }
    let _ = child.kill().await;
}

/// Stops what is left of a process group whose leader has exited: what a command left running
/// when it ended.
#[cfg(unix)]
pub(crate) async fn stop_left(group: rustix::process::Pid) {
    use rustix::process::{Signal, kill_process_group, test_kill_process_group};
    if test_kill_process_group(group).is_err() {
        return;
    }
    let _ = kill_process_group(group, Signal::TERM);
    if tokio::time::timeout(GRACE, emptied(group)).await.is_err() {
        let _ = kill_process_group(group, Signal::KILL);
        let _ = tokio::time::timeout(GRACE, emptied(group)).await;
    }
}

/// Resolves once bash has exited and nothing is left in its process group, not even a process
/// waiting to be reaped. Glave's workers run with an init that reaps orphans (Docker's `Init`,
/// the Pod's shared process namespace), as does any desktop.
#[cfg(unix)]
async fn gone(child: &mut Child, group: rustix::process::Pid) {
    let _ = child.wait().await;
    emptied(group).await;
}

/// Resolves once nothing is left in a process group.
#[cfg(unix)]
async fn emptied(group: rustix::process::Pid) {
    while rustix::process::test_kill_process_group(group).is_ok() {
        tokio::time::sleep(POLL).await;
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
