//! A program to run, set up as `std::process::Command` sets one up, and the ways glave starts
//! one: to run on its own ([`crate::Processes::spawn`]), waited for by its caller within a
//! limit ([`Program::output`]), or as a glave of its own ([`Program::detach`]).

use std::ffi::OsStr;
use std::path::Path;
use std::process::{Output, Stdio};
use std::time::Duration;

use rustix::process::{Pid, Signal};

use crate::error::ProcessError;
use crate::group;

/// The process group of a program run to its end, killed whole if the run stops before it
/// ends: at its limit, or when its caller stops waiting.
struct Unfinished(Option<Pid>);

impl Drop for Unfinished {
    fn drop(&mut self) {
        // While anything is left in the group its number is not given to another, so this
        // reaches only what the program started.
        if let Some(group) = self.0
            && group::exists(group)
        {
            group::signal(group, Signal::KILL);
        }
    }
}

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
    /// killed with everything it started, and the call fails, saying how long it had. It runs
    /// in a process group of its own, killed whole too if the caller stops waiting, so what it
    /// started (a clone's `ssh`, say) ends with the call. What it leaves running once it has
    /// ended itself, such as the browser an opener starts, is left be. It is not recorded in
    /// the ledger.
    pub async fn output(&mut self, limit: Duration) -> Result<Output, ProcessError> {
        self.command.stdout(Stdio::piped());
        self.command.stderr(Stdio::piped());
        self.run(None, limit).await
    }

    /// [`Program::output`], with `input` written to its standard input, which is then closed.
    pub async fn output_with(
        &mut self,
        input: &[u8],
        limit: Duration,
    ) -> Result<Output, ProcessError> {
        self.command.stdin(Stdio::piped());
        self.run(Some(input), limit).await
    }

    /// Runs it to the end within `limit`, in a process group of its own, `input` written to it
    /// first when there is some: the group is killed whole unless it ends in time.
    async fn run(&mut self, input: Option<&[u8]>, limit: Duration) -> Result<Output, ProcessError> {
        use tokio::io::AsyncWriteExt as _;
        if self.grouped {
            self.command.process_group(0);
        }
        let mut child = self.command.spawn().map_err(|error| self.failed(error))?;
        let leader = child.id().and_then(|pid| i32::try_from(pid).ok());
        let mut unfinished = Unfinished(leader.and_then(group::of));
        let stdin = child.stdin.take();
        let ran = async {
            if let (Some(input), Some(mut stdin)) = (input, stdin) {
                stdin.write_all(input).await?;
            }
            child.wait_with_output().await
        };
        let output = match tokio::time::timeout(limit, ran).await {
            Ok(output) => output.map_err(|error| self.failed(error)),
            Err(_) => return Err(self.late(limit)),
        };
        unfinished.0 = None;
        output
    }

    /// [`Program::output`], blocking, for code that runs outside the async runtime: in a
    /// process group of its own, killed whole at the limit, and whatever it leaves running
    /// there killed as it ends.
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
    fn at_its_limit_what_it_started_is_stopped_with_it() {
        use rustix::process::{kill_process, test_kill_process};
        let folder = glave_ports::testing::TempDir::new("program-limit").unwrap();
        let said = folder.join("pid");
        // The shell starts a sleep of its own, says which, and waits for it.
        let script = format!("sleep 300 & echo $! > '{}'; wait", said.display());
        let ran = glave_ports::block_on!(async {
            tokio::time::pause();
            let mut program = Program::new("sh");
            program.args(["-c", &script]);
            // Busy until the shell has said which, so the clock jumps to the limit only then.
            let file = said.clone();
            let saying = tokio::spawn(async move {
                while !std::fs::read_to_string(&file).is_ok_and(|pid| pid.ends_with('\n')) {
                    tokio::task::yield_now().await;
                }
            });
            let ran = program.output(Duration::from_secs(10)).await;
            saying.await.unwrap();
            ran
        });
        assert!(matches!(ran, Err(ProcessError::TimedOut { .. })), "{ran:?}");
        let pid: i32 = std::fs::read_to_string(&said)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let sleep = Pid::from_raw(pid).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while test_kill_process(sleep).is_ok() {
            if std::time::Instant::now() >= deadline {
                let _ = kill_process(sleep, Signal::KILL);
                panic!("the sleep it started outlived it");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn what_it_leaves_running_once_it_has_ended_is_left_be() {
        use rustix::process::{kill_process, test_kill_process};
        // As an opener leaves the browser it starts, its output elsewhere.
        let output = glave_ports::block_on!(
            Program::new("sh")
                .args(["-c", "sleep 300 >/dev/null 2>&1 & echo $!"])
                .output(AMPLE)
        )
        .unwrap();
        let pid: i32 = String::from_utf8(output.stdout)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let sleep = Pid::from_raw(pid).unwrap();
        let running = test_kill_process(sleep).is_ok();
        let _ = kill_process(sleep, Signal::KILL);
        assert!(running, "the sleep was stopped with the shell");
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
