//! Whether a sandbox started the command it was made for (#1533). Inside it, the command's
//! shell first writes a line on the sandbox's own error output, which only the sandbox
//! (bubblewrap, Seatbelt, glave's Landlock helper) and that line write to: the shell then
//! sends its own errors and the command's to standard output. A sandbox that ends before the
//! line ran nothing, and what it wrote says why. bubblewrap does when a place it was asked to
//! hide goes away while it makes the sandbox (an ssh session's agent ending, an editor or
//! tmux closing): it hides each as it was when the command's mounts were planned.

use std::time::Duration;

use glave_process::Child;
use tokio::io::AsyncReadExt as _;
use tokio::process::ChildStderr;

/// What the command's shell runs first: the line, on the sandbox's error output.
pub(super) const SAY_STARTED: &str = "printf '%s\\n' glave-sandbox-started >&2; ";

/// The line, as the sandbox's error output carries it.
const STARTED: &[u8] = b"glave-sandbox-started\n";

/// How long a sandbox has to start a command, which takes it milliseconds: a guard against a
/// hang, not a speed limit.
pub(super) const WITHIN: Duration = Duration::from_secs(10);

/// The most of what a sandbox writes that is kept, to say why it started nothing.
const MOST_SAID: usize = 4096;

/// How the sandbox's start ended.
pub(super) enum Started {
    /// The command runs: the sandbox's error output, held open while it does, so the sandbox
    /// never writes to a closed pipe.
    Yes(Option<ChildStderr>),
    /// The sandbox ended first, having said this.
    Ended(String),
    /// Neither, within [`WITHIN`].
    Late,
}

/// Waits until the sandbox `child` runs in has started its command, or has ended.
pub(super) async fn wait(child: &mut Child) -> Started {
    let Some(mut output) = child.stderr.take() else {
        return Started::Yes(None);
    };
    let mut said = Vec::new();
    let heard = tokio::time::timeout(WITHIN, heard(&mut output, &mut said)).await;
    match heard {
        Ok(true) => Started::Yes(Some(output)),
        Ok(false) => Started::Ended(String::from_utf8_lossy(&said).trim().to_owned()),
        Err(_) => Started::Late,
    }
}

/// Reads `output` into `said`, its last [`MOST_SAID`] bytes, until the line comes (true) or
/// the output ends (false).
async fn heard(output: &mut ChildStderr, said: &mut Vec<u8>) -> bool {
    let mut chunk = [0_u8; 1024];
    loop {
        let read = match output.read(&mut chunk).await {
            Ok(0) | Err(_) => return false,
            Ok(read) => read,
        };
        said.extend_from_slice(chunk.get(..read).unwrap_or_default());
        if said.windows(STARTED.len()).any(|window| window == STARTED) {
            return true;
        }
        let over = said.len().saturating_sub(MOST_SAID);
        said.drain(..over);
    }
}
