//! A program a test started, stopped when the test ends, however it ends.

use std::io::{self, Read};
use std::ops::{Deref, DerefMut};
use std::process::{Child, Output};

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
    /// one takes the child, which only this may stop.
    pub fn wait_with_output(mut self) -> io::Result<Output> {
        drop(self.0.stdin.take());
        let (stdout, stderr) = (self.0.stdout.take(), self.0.stderr.take());
        // Both at once, so a program that fills one pipe never waits on the other.
        let (stdout, stderr) = std::thread::scope(|scope| {
            let stdout = scope.spawn(|| everything(stdout));
            (stdout.join(), everything(stderr))
        });
        let stdout = stdout.map_err(|_| io::Error::other("reading its output failed"))??;
        Ok(Output {
            status: self.0.wait()?,
            stdout,
            stderr: stderr?,
        })
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

/// What `pipe` gives until it closes, or nothing when it was not kept.
fn everything(pipe: Option<impl Read>) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    if let Some(mut pipe) = pipe {
        pipe.read_to_end(&mut bytes)?;
    }
    Ok(bytes)
}
