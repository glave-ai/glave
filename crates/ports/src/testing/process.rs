//! A program a test started, stopped when the test ends, however it ends.

use std::io::{self, Read};
use std::ops::{Deref, DerefMut};
use std::process::{Child, Output};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::time::{Duration, Instant};

/// How long a program a test waits for may run before the test calls it hung: a guard against
/// a hang, never a speed limit. Generous, since a loaded machine is slow, and shorter than the
/// test runner's own limit, so the test says what the program printed.
pub const HUNG: Duration = Duration::from_secs(90);

/// Reads `program` and the test's own program to their ends, so a test's limit on what it does
/// next never counts either being read from a busy drive: a program's first start reads itself,
/// a page at each first touch (#1089). It has no limit of its own but the test runner's, since
/// how long it takes says nothing of the programs.
pub fn read_program(program: &str) -> io::Result<()> {
    for read in [std::path::PathBuf::from(program), std::env::current_exe()?] {
        io::copy(&mut std::fs::File::open(read)?, &mut io::sink())?;
    }
    Ok(())
}

/// A program a test started, killed and waited for when this drops, so a test that fails, or
/// ends without stopping it, never leaves it running. It stays in the test's process group,
/// which the test runner stops whole on a timeout or Ctrl-C, with whatever it started there.
#[derive(Debug)]
#[must_use = "dropping it stops the program"]
pub struct Running(Child);

impl Running {
    pub fn new(child: Child) -> Self {
        Self(child)
    }

    /// Waits for it to end, with what it printed, as [`Child::wait_with_output`] does; that
    /// one takes the child, which only this may stop. Its output is read to its end, which
    /// comes when the program and everything it left holding its output have ended; if that
    /// takes longer than [`HUNG`], it is stopped, and the error says what it printed.
    pub fn wait_with_output(self) -> io::Result<Output> {
        self.wait_within(HUNG)
    }

    /// [`Running::wait_with_output`], stopping it after `limit`.
    fn wait_within(mut self, limit: Duration) -> io::Result<Output> {
        drop(self.0.stdin.take());
        // Each read on a thread of its own, both at once, so a program that fills one pipe
        // never waits on the other, and a pipe something else still holds open never holds
        // the test past the guard.
        let (sender, read) = mpsc::channel();
        let out = Collected::start(self.0.stdout.take().map(Pipe::Out), &sender);
        let err = Collected::start(self.0.stderr.take().map(Pipe::Err), &sender);
        drop(sender);
        let deadline = Instant::now() + limit;
        for _ in 0..2 {
            match read.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(read) => read?,
                Err(_) => return Err(self.hung(limit, &out, &err)),
            }
        }
        Ok(Output {
            status: self.0.wait()?,
            stdout: out.taken(),
            stderr: err.taken(),
        })
    }

    /// Stops it, once it ran past `limit`: the error a test fails with, with what it printed.
    fn hung(&mut self, limit: Duration, out: &Collected, err: &Collected) -> io::Error {
        let _ = self.0.kill();
        let _ = self.0.wait();
        io::Error::other(format!(
            "still running after {limit:?}, so stopped: a hang, or something it started still \
             holds its output. It printed:\n{}{}",
            String::from_utf8_lossy(&out.taken()),
            String::from_utf8_lossy(&err.taken())
        ))
    }
}

/// What one of a program's outputs gave so far, read on a thread of its own, which says on
/// `done` when the output closes.
struct Collected(Arc<Mutex<Vec<u8>>>);

impl Collected {
    fn start(pipe: Option<Pipe>, done: &mpsc::Sender<io::Result<()>>) -> Self {
        let read = Arc::new(Mutex::new(Vec::new()));
        let (into, done) = (Arc::clone(&read), done.clone());
        std::thread::spawn(move || done.send(everything(pipe, &into)));
        Self(read)
    }

    /// Everything read so far.
    fn taken(&self) -> Vec<u8> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

/// A program's output, either one.
enum Pipe {
    Out(std::process::ChildStdout),
    Err(std::process::ChildStderr),
}

impl Read for Pipe {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Out(pipe) => pipe.read(buffer),
            Self::Err(pipe) => pipe.read(buffer),
        }
    }
}

impl Deref for Running {
    type Target = Child;

    fn deref(&self) -> &Child {
        &self.0
    }
}

impl DerefMut for Running {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.0
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        // A program already waited for is not signalled: its number may be another's by now.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Appends what `pipe` gives to `into` until it closes; nothing when it was not kept.
fn everything(pipe: Option<impl Read>, into: &Mutex<Vec<u8>>) -> io::Result<()> {
    let Some(mut pipe) = pipe else {
        return Ok(());
    };
    let mut chunk = [0_u8; 8192];
    loop {
        match pipe.read(&mut chunk) {
            Ok(0) => return Ok(()),
            Ok(read) => into
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend_from_slice(chunk.get(..read).unwrap_or_default()),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::process::{Command, Stdio};

    use super::*;

    #[test]
    fn a_program_is_waited_for_with_what_it_printed() {
        let child = Command::new("sh")
            .args(["-c", "echo out; echo err >&2"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let output = Running::new(child).wait_with_output().unwrap();
        assert!(output.status.success());
        assert_eq!(
            (&output.stdout[..], &output.stderr[..]),
            (&b"out\n"[..], &b"err\n"[..])
        );
    }

    #[test]
    fn a_hung_program_is_stopped() {
        let child = Command::new("sleep")
            .arg("600")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let error = Running::new(child)
            .wait_within(Duration::from_millis(50))
            .unwrap_err();
        assert!(
            error.to_string().contains("still running after 50ms"),
            "{error}"
        );
    }
}
