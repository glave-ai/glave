//! A program to run, set up as `std::process::Command` sets one up, and the ways glave starts
//! one: to run on its own ([`crate::Processes::spawn`]), waited for by its caller within a
//! limit ([`crate::Processes::output`]), or as a glave of its own ([`Program::detach`]).

use std::ffi::OsStr;
use std::path::Path;
use std::process::{Output, Stdio};
use std::time::Duration;

use crate::error::ProcessError;

/// Where a program is looked for when it starts with no `PATH`, as the system looks (musl's,
/// the widest of the C libraries glave is built with).
const DEFAULT_PATH: &str = "/usr/local/bin:/bin:/usr/bin";

/// What to run: the program, its arguments, folder, environment and standard streams.
#[derive(Debug)]
pub struct Program {
    /// The program, its arguments, folder and environment; its streams are set as it starts.
    pub(crate) command: tokio::process::Command,
    /// Whether glave gives it a process group of its own; not when it starts a session itself.
    pub(crate) grouped: bool,
    /// What it reads.
    pub(crate) input: Input,
    /// Where its output and its errors go, until it starts; as this process's when unset.
    stdout: Option<Stdio>,
    stderr: Option<Stdio>,
    /// Whether it starts with none of this process's environment.
    cleared: bool,
}

/// What a program reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Input {
    /// This process's own input: what a program glave becomes reads ([`Program::exec`]), or
    /// one started without a lifeline. One started with a lifeline reads nothing instead: in a
    /// group of its own it could not read a terminal.
    Inherited,
    /// Nothing: its input is at its end.
    Nothing,
    /// A pipe glave writes to.
    Piped,
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
            input: Input::Inherited,
            stdout: None,
            stderr: None,
            cleared: false,
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
        self.cleared = true;
        self
    }

    /// It reads nothing: its input is at its end from the start.
    pub fn no_input(&mut self) -> &mut Self {
        self.input = Input::Nothing;
        self
    }

    /// It reads a pipe glave writes to: its [`crate::Child`]'s `stdin`.
    pub fn piped_input(&mut self) -> &mut Self {
        self.input = Input::Piped;
        self
    }

    pub fn stdout(&mut self, stdout: impl Into<Stdio>) -> &mut Self {
        self.stdout = Some(stdout.into());
        self
    }

    pub fn stderr(&mut self, stderr: impl Into<Stdio>) -> &mut Self {
        self.stderr = Some(stderr.into());
        self
    }

    /// The command that runs it as it is, its streams set: started at once.
    pub(crate) fn direct(&mut self) -> &mut tokio::process::Command {
        match self.input {
            Input::Inherited => {}
            Input::Nothing => {
                self.command.stdin(Stdio::null());
            }
            Input::Piped => {
                self.command.stdin(Stdio::piped());
            }
        }
        if let Some(stdout) = self.stdout.take() {
            self.command.stdout(stdout);
        }
        if let Some(stderr) = self.stderr.take() {
            self.command.stderr(stderr);
        }
        &mut self.command
    }

    /// The command that runs it once glave says so: `starter` (glave itself, as
    /// [`crate::lifeline_helper`] runs it) given what the program reads, then the program and
    /// its arguments, in its folder, with its environment and output, its input a pipe glave
    /// writes to first that it may go (`lifeline/gate.rs`). It becomes the program then, in the
    /// same process, reading that pipe on or nothing.
    pub(crate) fn gated(&mut self, starter: &Path) -> tokio::process::Command {
        use crate::lifeline::gate::{NONE, PIPE, START_ARG};
        let real = self.command.as_std();
        let mut gate = tokio::process::Command::new(starter);
        let input = if self.input == Input::Piped {
            PIPE
        } else {
            NONE
        };
        // Named as the program is, while it waits to become it.
        gate.arg0(real.get_program())
            .arg(START_ARG)
            .arg(input)
            .arg(real.get_program())
            .args(real.get_args());
        if let Some(dir) = real.get_current_dir() {
            gate.current_dir(dir);
        }
        if self.cleared {
            gate.env_clear();
        }
        for (name, value) in real.get_envs() {
            match value {
                Some(value) => gate.env(name, value),
                None => gate.env_remove(name),
            };
        }
        gate.stdin(Stdio::piped()).kill_on_drop(true);
        if let Some(stdout) = self.stdout.take() {
            gate.stdout(stdout);
        }
        if let Some(stderr) = self.stderr.take() {
            gate.stderr(stderr);
        }
        gate
    }

    /// The program starts a session of its own first thing (the Linux sandbox's helper), so
    /// glave gives it no process group: a group's leader cannot start a session. The session
    /// is its group all the same.
    pub fn starts_own_session(&mut self) -> &mut Self {
        self.grouped = false;
        self
    }

    /// Whether it can be started, as starting it directly would find it: at its path, from
    /// its folder, or on the `PATH` it starts with (a relative folder there from its folder),
    /// a file this user may run; refused as that start would be.
    pub(crate) fn found(&self) -> Result<(), ProcessError> {
        let real = self.command.as_std();
        let program = Path::new(real.get_program());
        let from = |path: &Path| match real.get_current_dir() {
            Some(dir) if path.is_relative() => dir.join(path),
            _ => path.to_owned(),
        };
        let runnable = |path: &Path| {
            std::fs::metadata(path).is_ok_and(|found| found.is_file())
                && rustix::fs::access(path, rustix::fs::Access::EXEC_OK).is_ok()
        };
        let found = if program.components().count() > 1 {
            runnable(&from(program))
        } else {
            let set = real.get_envs().find(|(name, _)| *name == "PATH");
            let path = match set {
                Some((_, value)) => value.map(OsStr::to_owned),
                None if self.cleared => None,
                None => std::env::var_os("PATH"),
            };
            let path = path.unwrap_or_else(|| DEFAULT_PATH.into());
            std::env::split_paths(&path).any(|folder| runnable(&from(&folder).join(program)))
        };
        if found {
            Ok(())
        } else {
            Err(self.failed(std::io::Error::from(rustix::io::Errno::NOENT)))
        }
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
        match crate::within::output(self.direct().as_std_mut(), limit) {
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
        let error = self.direct().as_std_mut().exec();
        self.failed(error)
    }

    /// Starts it apart from this process, in a process group of its own (or the session it
    /// starts, [`Program::starts_own_session`]), and lets it go: only for a glave of its own
    /// (the person's server), which owns what it starts. The system reaps it
    /// when it exits.
    pub fn detach(&mut self) -> Result<(), ProcessError> {
        use std::os::unix::process::CommandExt as _;
        let grouped = self.grouped;
        let command = self.direct().as_std_mut();
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
