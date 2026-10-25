//! A program to run, set up as `std::process::Command` sets one up, and the ways glave starts
//! one: to run on its own ([`crate::Processes::spawn`]), waited for by its caller within a
//! limit ([`Program::output`]), or as a glave of its own ([`Program::detach`]).

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
    fn late(&self, limit: Duration) -> ProcessError {
        ProcessError::TimedOut {
            program: self.name(),
            limit,
        }
    }

    /// Runs it to the end, with what it printed, within `limit`: still running then, it is
    /// killed and the call fails, saying how long it had. It is awaited where it starts and
    /// killed if the caller stops waiting, so it ends with the call; it stays in glave's
    /// process group, as a short helper (git, the browser's opener) is, and is not recorded in
    /// the ledger.
    pub async fn output(&mut self, limit: Duration) -> Result<Output, ProcessError> {
        match tokio::time::timeout(limit, self.command.output()).await {
            Ok(output) => output.map_err(|error| self.failed(error)),
            Err(_) => Err(self.late(limit)),
        }
    }

    /// [`Program::output`], with `input` written to its standard input, which is then closed.
    pub async fn output_with(
        &mut self,
        input: &[u8],
        limit: Duration,
    ) -> Result<Output, ProcessError> {
        use tokio::io::AsyncWriteExt as _;
        self.command.stdin(Stdio::piped());
        let mut child = self.command.spawn().map_err(|error| self.failed(error))?;
        let ran = async {
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(input).await?;
            }
            child.wait_with_output().await
        };
        match tokio::time::timeout(limit, ran).await {
            Ok(output) => output.map_err(|error| self.failed(error)),
            Err(_) => Err(self.late(limit)),
        }
    }

    /// [`Program::output`], blocking, for code that runs outside the async runtime.
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
    fn a_program_that_ends_in_time_gives_what_it_printed() {
        let printed = glave_ports::block_on!(Program::new("printf").arg("done").output(AMPLE));
        assert_eq!(printed.unwrap().stdout, b"done");
        let echoed = glave_ports::block_on!(
            Program::new("cat")
                .stdout(Stdio::piped())
                .output_with(b"given", AMPLE)
        );
        assert_eq!(echoed.unwrap().stdout, b"given");
        let blocking = Program::new("printf").arg("done").output_blocking(AMPLE);
        assert_eq!(blocking.unwrap().stdout, b"done");
    }

    #[test]
    fn one_still_running_at_its_limit_is_stopped_saying_how_long_it_had() {
        let limit = Duration::from_secs(10);
        let (waited, fed) = glave_ports::block_on!(async {
            // The clock jumps to the limit as soon as nothing else can run.
            tokio::time::pause();
            let waited = Program::new("sleep").arg("300").output(limit).await;
            let fed = Program::new("sleep")
                .arg("300")
                .output_with(b"unread", limit)
                .await;
            (waited, fed)
        });
        for error in [waited.unwrap_err(), fed.unwrap_err()] {
            assert!(matches!(error, ProcessError::TimedOut { .. }), "{error:?}");
            assert_eq!(error.to_string(), "sleep did not finish within 10 seconds");
        }
    }

    #[test]
    fn a_blocking_run_still_going_at_its_limit_is_killed_and_says_so() {
        let error = Program::new("sleep")
            .arg("300")
            .output_blocking(Duration::from_millis(50))
            .unwrap_err();
        assert_eq!(error.to_string(), "sleep did not finish within 50 ms");
    }
}
