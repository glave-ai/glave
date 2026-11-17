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

#[cfg(test)]
mod tests {
    use std::process::Stdio;

    use glave_ports::block_on;
    use glave_process::{Processes, Program};

    use super::*;

    /// `script` run by `sh` as a sandbox would run a command, with `said` as `$1`, its error
    /// output glave's.
    async fn sandbox(script: &str, said: &str) -> Child {
        let mut program = Program::new("sh");
        program
            .args(["-c", script, "sh", said])
            .no_input()
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        Processes::unrecorded().spawn(&mut program).await.unwrap()
    }

    #[test]
    fn a_command_whose_shell_says_so_runs_whatever_the_sandbox_said_first() {
        block_on!(async {
            let script = format!("echo \"$1\" >&2; {SAY_STARTED}exec sleep 30");
            let mut child = sandbox(&script, "a warning first").await;
            let started = wait(&mut child).await;
            assert!(matches!(started, Started::Yes(Some(_))));
            assert!(child.try_wait().unwrap().is_none(), "still runs");
            child.stop().await;
        });
    }

    #[test]
    fn a_sandbox_that_ends_before_its_command_started_says_why() {
        block_on!(async {
            let said = "bwrap: Can't bind mount /oldroot/e on /newroot/tmp/gone: Unable to mount \
                        source on destination: No such file or directory";
            let mut child = sandbox("echo \"$1\" >&2; exit 1", said).await;
            match wait(&mut child).await {
                Started::Ended(ended) => assert_eq!(ended, said),
                Started::Yes(_) | Started::Late => panic!("it started nothing"),
            }
            child.stop().await;
        });
    }
}
