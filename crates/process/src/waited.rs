//! A program glave waits for, started as one that runs on its own is ([`Processes::spawn`]):
//! in a process group of its own, tied to glave by the lifeline and recorded for its owner
//! while it runs, so a `git clone` or an eval's grader stops at once when glave is killed
//! outright, rather than running on for minutes.

use std::process::{ExitStatus, Output, Stdio};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWriteExt as _};

use crate::child::Child;
use crate::error::ProcessError;
use crate::processes::Processes;
use crate::program::Program;

impl Processes {
    /// Runs `program` to its end, with what it printed, within `limit`: still running then,
    /// it is stopped with everything it started, and the call fails, saying how long it had.
    /// Its group is stopped whole too if the caller stops waiting, so what it started (a
    /// clone's `ssh`, say) ends with the call. What it leaves running once it has ended
    /// itself, such as the browser an opener starts, is let go: no longer tied or recorded.
    /// In a group of its own it gets no Ctrl-C from the terminal; glave stops it instead.
    pub async fn output(
        &self,
        program: &mut Program,
        limit: Duration,
    ) -> Result<Output, ProcessError> {
        program.stdout(Stdio::piped()).stderr(Stdio::piped());
        self.waited(program, None, limit).await
    }

    /// [`Processes::output`] for a program whose output no one reads, such as the opener
    /// that hands a page to the browser: its streams stay as `program` set them, and it is
    /// done when it ends, however long what it started (that browser) holds them open.
    pub async fn status(
        &self,
        program: &mut Program,
        limit: Duration,
    ) -> Result<ExitStatus, ProcessError> {
        Ok(self.waited(program, None, limit).await?.status)
    }

    /// [`Processes::output`], with `input` written to its standard input, which is then
    /// closed.
    pub async fn output_with(
        &self,
        program: &mut Program,
        input: &[u8],
        limit: Duration,
    ) -> Result<Output, ProcessError> {
        program.piped_input();
        self.waited(program, Some(input), limit).await
    }

    /// Starts `program`, `input` written to it first when there is some, and waits for it
    /// within `limit`; unless it ends in time, its group is stopped as its [`Child`] drops.
    async fn waited(
        &self,
        program: &mut Program,
        input: Option<&[u8]>,
        limit: Duration,
    ) -> Result<Output, ProcessError> {
        let mut child = self.spawn(program).await?;
        match tokio::time::timeout(limit, finished(&mut child, input)).await {
            Ok(Ok(output)) => {
                child.let_go();
                Ok(output)
            }
            Ok(Err(error)) => Err(program.failed(error)),
            Err(_) => Err(program.late(limit)),
        }
    }
}

/// How `child` ended, with all it printed: `input` written to it and its standard input
/// closed, beside reading both its outputs to their ends, so neither side waits on a full
/// pipe.
async fn finished(child: &mut Child, input: Option<&[u8]>) -> std::io::Result<Output> {
    let stdin = child.stdin.take();
    let (stdout, stderr) = (child.stdout.take(), child.stderr.take());
    let written = async {
        if let (Some(input), Some(mut stdin)) = (input, stdin) {
            stdin.write_all(input).await?;
        }
        Ok(())
    };
    let (status, (), stdout, stderr) =
        tokio::try_join!(child.wait(), written, drained(stdout), drained(stderr))?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

/// Everything `pipe` gives until it closes; nothing when there is no pipe.
async fn drained(pipe: Option<impl AsyncRead + Unpin>) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    if let Some(mut pipe) = pipe {
        pipe.read_to_end(&mut bytes).await?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests;
