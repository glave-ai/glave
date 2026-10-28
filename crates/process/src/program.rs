//! A program to run, set up as `std::process::Command` sets one up, and the ways glave starts
//! one: to run on its own ([`crate::Processes::spawn`]), waited for by its caller within a
//! limit ([`crate::Processes::output`]), or as a glave of its own ([`Program::detach`]).

use std::ffi::OsStr;
use std::path::Path;
use std::process::{Output, Stdio};
use std::time::Duration;

use crate::error::ProcessError;

/// What to run: the program, its arguments, folder, environment and standard streams.
#[derive(Debug)]
pub struct Program {
    pub(crate) command: tokio::process::Command,
    /// Whether glave gives it a process group of its own; not when it starts a session itself.
    pub(crate) grouped: bool,
}

impl Program {
    /// `program`, found on `PATH` unless it is a path.
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        let mut command = tokio::process::Command::new(program);
        // A process glave starts never outlives the handle that started it.
        command.kill_on_drop(true);
        Self {
            command,
            grouped: true,
        }
    }

    pub fn arg(&mut self, arg: impl AsRef<OsStr>) -> &mut Self {
        self.command.arg(arg);
        self
    }

    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.command.args(args);
        self
    }

    pub fn current_dir(&mut self, dir: impl AsRef<Path>) -> &mut Self {
        self.command.current_dir(dir);
        self
    }

    pub fn env(&mut self, name: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> &mut Self {
        self.command.env(name, value);
        self
    }

    pub fn envs<I, K, V>(&mut self, variables: I) -> &mut Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.command.envs(variables);
        self
    }

    pub fn env_clear(&mut self) -> &mut Self {
        self.command.env_clear();
        self
    }

    pub fn stdin(&mut self, stdin: impl Into<Stdio>) -> &mut Self {
        self.command.stdin(stdin);
        self
    }

    pub fn stdout(&mut self, stdout: impl Into<Stdio>) -> &mut Self {
        self.command.stdout(stdout);
        self
    }

    pub fn stderr(&mut self, stderr: impl Into<Stdio>) -> &mut Self {
        self.command.stderr(stderr);
        self
    }

    /// The program starts a session of its own first thing (the Linux sandbox's helper), so
    /// glave gives it no process group: a group's leader cannot start a session. The session
    /// is its group all the same.
    pub fn starts_own_session(&mut self) -> &mut Self {
        self.grouped = false;
        self
    }

    /// The program's name, for errors.
    pub(crate) fn name(&self) -> String {
        self.command
            .as_std()
            .get_program()
            .to_string_lossy()
            .into_owned()
    }

    pub(crate) fn failed(&self, source: std::io::Error) -> ProcessError {
        ProcessError::Start {
            program: self.name(),
            source,
        }
    }

    /// It did not finish within `limit`.
    pub(crate) fn late(&self, limit: Duration) -> ProcessError {
        ProcessError::TimedOut {
            program: self.name(),
            limit,
        }
    }

    /// Runs it to the end, with what it printed, within `limit`, blocking, for code that runs
    /// outside the async runtime and a program that ends in moments (`bwrap --version`): in a
    /// process group of its own, killed whole at the limit, and whatever it leaves running
    /// there killed as it ends. It is neither tied nor recorded; one that runs longer is
    /// waited for through [`crate::Processes::output`].
    pub fn output_blocking(&mut self, limit: Duration) -> Result<Output, ProcessError> {
        match crate::within::output(self.command.as_std_mut(), limit) {
            Ok(Some(output)) => Ok(output),
            Ok(None) => Err(self.late(limit)),
            Err(error) => Err(self.failed(error)),
        }
    }

    /// Becomes the program, replacing this process (the sandbox's helper does, once it has
    /// confined itself): it keeps this process's id, group and owner. Returns only on
    /// failure.
    pub fn exec(&mut self) -> ProcessError {
        use std::os::unix::process::CommandExt as _;
        let error = self.command.as_std_mut().exec();
        self.failed(error)
    }

    /// Starts it apart from this process, in a process group of its own (or the session it
    /// starts, [`Program::starts_own_session`]), and lets it go: only for a glave of its own
    /// (the person's server), which owns what it starts. The system reaps it
    /// when it exits.
    pub fn detach(&mut self) -> Result<(), ProcessError> {
        use std::os::unix::process::CommandExt as _;
        let grouped = self.grouped;
        let command = self.command.as_std_mut();
        if grouped {
            command.process_group(0);
        }
        let child = command.spawn().map_err(|error| self.failed(error))?;
        drop(child);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AMPLE: Duration = Duration::from_secs(30);

    #[test]
    fn a_blocking_run_that_ends_in_time_gives_what_it_printed() {
        let blocking = Program::new("printf").arg("done").output_blocking(AMPLE);
        assert_eq!(blocking.unwrap().stdout, b"done");
    }

    #[test]
    fn a_blocking_run_still_going_at_its_limit_is_killed_and_says_so() {
        // A limit of nothing has passed by the first look, with no clock to wait on.
        let error = Program::new("sleep")
            .arg("300")
            .output_blocking(Duration::ZERO)
            .unwrap_err();
        assert_eq!(error.to_string(), "sleep did not finish within 0 ms");
    }

    #[test]
    fn a_blocking_run_ends_with_what_its_program_left_holding_its_output() {
        // The shell ends at once; the sleep it leaves holds its output open.
        let output = Program::new("sh")
            .args(["-c", "sleep 300 & echo left"])
            .output_blocking(AMPLE)
            .unwrap();
        assert_eq!(output.stdout, b"left\n");
    }
}
