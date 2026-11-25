//! Running one command at a workspace root: bash in a fresh process group, in the sandbox the
//! command carries, output streamed as it arrives; at a cancel stopped with everything it
//! started, and at a timeout stopped too, or kept running in the background when it asks.

mod started;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use glave_ports::computer::{
    Command, CommandId, ComputerError, Ending, Finished, GitCredentials, OnTimeout, Output,
    Refusal, WorkspaceId,
};
use glave_ports::{Cancellation, OsEntropy};
use glave_process::{Child, Program};
use tokio::io::AsyncWriteExt as _;
use tokio::process::ChildStderr;
use tokio::time::Instant;

use super::background::Backgrounds;
use super::credentials::{Offered, offer};
use super::process::Stream;
use super::registry::{Opened, new_id};
use super::sandbox::{Launch, Prepared};
use started::{Line, Start, Tried};

const DEFAULT_TIMEOUT_SECS: u64 = 120;
const MAX_TIMEOUT_SECS: u64 = 600;
/// How long to keep reading after the command exits, in case a background child it started
/// still holds the output open.
pub(crate) const DRAIN: Duration = Duration::from_millis(500);
/// Package managers' caches, kept in the home folder by default, where sandboxed commands
/// cannot write. In the sandbox each goes in a folder of its own in glave's sandbox cache
/// folder (`~/.glave/sandbox-cache`), kept between runs so installs stay fast, or, for
/// someone other than the person whose computer it is, or without glave's folder, in the
/// command's own temp folder. Only sandboxed commands use it: nothing a command leaves there
/// reaches a command outside the sandbox, though it does reach the person's later sandboxed
/// ones, which is why nobody else's commands share it (#1469). Cargo's home is left alone: it holds the person's settings and installed
/// tools beside the registry, so it cannot move, and its registry cannot be shared writable,
/// since the person's own builds outside the sandbox compile and run what is there.
const CACHES: &[(&str, &str)] = &[
    ("XDG_CACHE_HOME", "xdg"),
    ("npm_config_cache", "npm"),
    ("npm_config_store_dir", "pnpm"),
    ("YARN_CACHE_FOLDER", "yarn"),
    ("BUN_INSTALL_CACHE_DIR", "bun"),
    ("PIP_CACHE_DIR", "pip"),
    ("UV_CACHE_DIR", "uv"),
    ("UV_PYTHON_INSTALL_DIR", "uv-python"),
    ("POETRY_CACHE_DIR", "poetry"),
    ("PRE_COMMIT_HOME", "pre-commit"),
    ("HF_HOME", "huggingface"),
    ("DENO_DIR", "deno"),
    ("GOPATH", "go"),
    ("GOMODCACHE", "go/pkg/mod"),
    ("GOCACHE", "go-build"),
    ("GRADLE_USER_HOME", "gradle"),
];

/// Rust's toolchains and crates, in their own homes outside the sandbox. A computer whose
/// environment sets [`TOOLCHAINS_IN_SANDBOX`] (glave's computer image does) has its sandboxed
/// commands keep them in the sandbox cache like the rest, so they build without asking: nothing
/// planted there reaches a command outside the sandbox. That computer gives up what Cargo's home
/// outside it holds, which a laptop can't (`CACHES`): its settings and crates, though its tools
/// still run, after the sandbox's own (`cargo_bin_on_path`).
const TOOLCHAINS: &[(&str, &str)] = &[("CARGO_HOME", "cargo"), ("RUSTUP_HOME", "rustup")];

/// The variable of the computer's own environment that says so, as `1`.
const TOOLCHAINS_IN_SANDBOX: &str = "GLAVE_SANDBOX_TOOLCHAINS";

/// `PATH` with the sandbox's Cargo home's `bin` just before Cargo's own, where `PATH` names
/// that (`~/.cargo/bin`): a tool `cargo install` adds in the sandbox runs by its name there, and
/// one added outside it still does. Nothing outside the sandbox looks in the sandbox's.
fn cargo_bin_on_path(environment: &[(OsString, OsString)], caches: &Path) -> Option<OsString> {
    // The last of each, as the command's environment takes it.
    let named = |wanted: &str| {
        environment
            .iter()
            .rev()
            .find(|(name, _)| name == wanted)
            .map(|(_, value)| value)
    };
    let own = Path::new(named("HOME")?).join(".cargo/bin");
    let folders: Vec<PathBuf> = std::env::split_paths(named("PATH")?).collect();
    if !folders.contains(&own) {
        return None;
    }
    let sandbox = caches.join("cargo/bin");
    let folders = folders.into_iter().flat_map(|folder| {
        let before = (folder == own).then(|| sandbox.clone());
        before.into_iter().chain([folder])
    });
    std::env::join_paths(folders).ok()
}

/// Maven has no variable for its repository, only a property, passed through `MAVEN_OPTS`,
/// which is split at projects: a cache folder with a project in its path is left out.
fn maven_opts(environment: &[(OsString, OsString)], caches: &Path) -> Option<OsString> {
    let repository = caches.join("maven");
    let repository = repository
        .to_str()
        .filter(|path| !path.contains(char::is_whitespace))?;
    let mut opts = environment
        .iter()
        .find(|(name, _)| name == "MAVEN_OPTS")
        .map(|(_, value)| {
            let mut value = value.clone();
            value.push(" ");
            value
        })
        .unwrap_or_default();
    opts.push(format!("-Dmaven.repo.local={repository}"));
    Some(opts)
}

/// A command that has started, and what must last as long as it runs.
#[derive(Debug)]
pub(crate) struct Spawned {
    pub(crate) child: Child,
    pub(crate) stream: Stream,
    pub(crate) held: Held,
    pub(crate) started: Instant,
}

impl Spawned {
    /// `child`, just started, holding `held` while it runs.
    fn of(mut child: Child, held: Held) -> Self {
        let stream = Stream::of(&mut child);
        Self {
            child,
            stream,
            held,
            started: Instant::now(),
        }
    }
}

/// What a running command holds: the sandbox it was prepared in, its own temp folder and what
/// launching it left in the workspace, both removed once it ends, and the sandbox's own error
/// output.
#[derive(Debug, Default)]
pub(crate) struct Held {
    _launch: Option<Launch>,
    _sandbox: Option<Arc<Prepared>>,
    _said: Option<ChildStderr>,
}

/// Starts `command` by bash in `opened`, in its sandbox when it has one, its git asking
/// `offered`'s helper when given. `helper` runs the Linux sandbox's helper, if this machine
/// has one. A sandboxed command is handed back once its shell says it runs inside, or may
/// (`started.rs`); one its sandbox says it could not start fails, saying what the sandbox said,
/// unless bubblewrap's, made again from a fresh look, starts it. A cancel meanwhile hands it
/// back as it is, for whoever waits on it to stop.
pub(crate) async fn spawn(
    opened: &Opened,
    command: &Command,
    helper: Option<&Path>,
    offered: Option<&Offered>,
    cancel: &dyn Cancellation,
) -> Result<Spawned, ComputerError> {
    if command.script.trim().is_empty() {
        return Err(ComputerError::Refused(Refusal::EmptyCommand.into()));
    }
    let Some(sandbox) = &command.sandbox else {
        let child = start(opened, &mut bash(opened, command, None, offered), None).await?;
        return Ok(Spawned::of(child, Held::default()));
    };
    let prepared = &opened.sandbox(sandbox, helper).await?;
    started::until_started(move || launched(opened, command, prepared, offered, cancel)).await
}

/// One try at starting `command` in `prepared`, made from a fresh look: a place gone since the
/// last try is no longer hidden.
async fn launched(
    opened: &Opened,
    command: &Command,
    prepared: &Arc<Prepared>,
    offered: Option<&Offered>,
    cancel: &dyn Cancellation,
) -> Result<Tried<Spawned>, ComputerError> {
    let launch = prepared.launch().await?;
    let line = Line::new(&OsEntropy).map_err(|error| ComputerError::Failed(error.to_string()))?;
    let mut bash = bash(opened, command, Some((&launch, prepared, &line)), offered);
    let mut child = start(opened, &mut bash, launch.input.as_deref()).await?;
    let mut said = child.stderr.take();
    // None when cancelled meanwhile, or when there is no output to hear it on.
    let heard = match said.as_mut() {
        Some(said) => {
            let waited = started::wait(said, &line, launch.refuses, started::WITHIN);
            glave_ports::unless_cancelled(cancel, waited).await
        }
        None => None,
    };
    if let Some(Start::NeverRan { said, again }) = heard {
        child.stop().await;
        return Ok(Tried::NeverRan { said, again });
    }
    let held = Held {
        _launch: Some(launch),
        _sandbox: Some(Arc::clone(prepared)),
        _said: said,
    };
    Ok(Tried::Runs(Spawned::of(child, held)))
}

/// `bash` started in `opened`, given `input` on its standard input, which then closes.
async fn start(
    opened: &Opened,
    bash: &mut Program,
    input: Option<&[u8]>,
) -> Result<Child, ComputerError> {
    let mut child = (opened.processes.spawn(bash).await)
        .map_err(|error| ComputerError::Failed(error.to_string()))?;
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        // Small (a seccomp filter), so it fits the pipe; a wrapper that failed early and
        // never reads it says why on its own output.
        let _ = stdin.write_all(input).await;
    }
    Ok(child)
}

/// Where a command that reaches its timeout is kept running, when it asks to be: this
/// machine's commands in the background, in the workspace it runs in.
pub(crate) struct Keep<'a> {
    pub(crate) background: &'a Backgrounds,
    pub(crate) workspace: WorkspaceId,
    /// The cgroup counting the CPU the command uses against its limit, where Linux counts it.
    pub(crate) cgroup: Option<&'a Path>,
}

/// Runs `command` in `opened`. `helper` runs the Linux sandbox's helper, if this machine has
/// one. Git and the forge tools ask `credentials`, if given, for the hosts it names, at each
/// use; a command moved to the background asks no more.
pub(crate) async fn run(
    opened: &Opened,
    command: &Command,
    (output, cancel): (&dyn Output, &dyn Cancellation),
    helper: Option<&Path>,
    credentials: Option<&dyn GitCredentials>,
    keep: Keep<'_>,
) -> Result<Finished, ComputerError> {
    let limit = command
        .timeout_secs
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
        .clamp(1, MAX_TIMEOUT_SECS);
    // Held until the command ends, or moves to the background: its helper's folder and
    // listener go with it.
    let offered = match credentials {
        Some(credentials) => {
            offer(
                credentials,
                &opened.environment,
                opened.glave_on_path.as_deref(),
            )
            .await?
        }
        None => None,
    };
    // Its id in the background, made before it starts: without an id it never runs.
    let keeps = (command.on_timeout == OnTimeout::Background)
        .then(|| new_id().map(CommandId::from_uuid))
        .transpose()?;
    let before = super::pressure::counted(keep.cgroup).await;
    let mut spawned = spawn(opened, command, helper, offered.as_ref(), cancel).await?;
    let waited = {
        let pump = spawned.stream.pump(output);
        let waiting = answering(
            wait(&mut spawned.child, limit, cancel, keeps),
            offered.as_ref().zip(credentials),
        );
        tokio::pin!(pump, waiting);
        let (waited, pumped) = tokio::select! {
            waited = &mut waiting => (waited, false),
            () = &mut pump => (waiting.await, true),
        };
        if !pumped && !matches!(waited, Ok(Waited::Reached(_))) {
            let _ = tokio::time::timeout(DRAIN, pump).await;
        }
        waited
    };
    let millis = u64::try_from(spawned.started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let ending = match waited? {
        Waited::Ended(ending) => ending,
        Waited::Reached(id) => {
            let named = (id, command.script.clone());
            let at = (
                opened.computer,
                keep.workspace,
                super::background::runs_for(command),
            );
            let adopted = keep.background.adopt(named, at, spawned, None);
            match adopted {
                Ok(command) => Ending::Backgrounded {
                    command,
                    after_secs: limit,
                },
                // The workspace keeps as many as it may: stopped, as without the move.
                Err(mut spawned) => {
                    spawned.child.stop().await;
                    Ending::TimedOut { after_secs: limit }
                }
            }
        }
    };
    let after = super::pressure::counted(keep.cgroup).await;
    Ok(Finished {
        ending,
        millis,
        cpu_held: super::pressure::held(before, after, millis),
    })
}

/// `work`, with the command's credential helper answered meanwhile, when it has one.
async fn answering<T>(
    work: impl Future<Output = T>,
    helper: Option<(&Offered, &dyn GitCredentials)>,
) -> T {
    let Some((offered, credentials)) = helper else {
        return work.await;
    };
    let serving = offered.serve(credentials);
    tokio::pin!(work, serving);
    let mut listening = true;
    loop {
        tokio::select! {
            done = &mut work => return done,
            () = &mut serving, if listening => listening = false,
        }
    }
}

/// `command` run by bash at the workspace root, in the sandbox if there is one, with its own
/// variables and the workspace's environment, whose win, the credential helper's settings if it
/// has one, and output that reads well as text.
fn bash(
    opened: &Opened,
    command: &Command,
    sandbox: Option<(&Launch, &Prepared, &Line)>,
    offered: Option<&Offered>,
) -> Program {
    let (script, own) = (&command.script, &command.env);
    let mut command = match sandbox {
        Some((launch, _, _)) => {
            let mut command = Program::new(&launch.program);
            command.args(&launch.args).arg("bash");
            if launch.own_session {
                command.starts_own_session();
            }
            command
        }
        None => Program::new("bash"),
    };
    let input = sandbox.is_some_and(|(launch, _, _)| launch.input.is_some());
    let caches = sandbox.map(|(launch, prepared, _)| prepared.caches(launch));
    let git_settings = sandbox.and_then(|(launch, prepared, _)| prepared.git_settings(launch));
    let toolchains = opened
        .environment
        .iter()
        .any(|(name, value)| name == TOOLCHAINS_IN_SANDBOX && value == "1");
    command
        .arg("-c")
        .arg(format!(
            "{}exec 2>&1\n{script}",
            said_first(sandbox.map(|(_, _, line)| line))
        ))
        .current_dir(&opened.root)
        .env_clear()
        .envs(own.iter().map(|(name, value)| (name, value.expose())))
        .envs(opened.environment.iter().map(|(name, value)| (name, value)))
        .envs(sandbox.map(|(launch, _, _)| ("TMPDIR", launch.temp.path().to_path_buf())))
        .envs(caches.iter().flat_map(|caches| {
            CACHES
                .iter()
                .chain(toolchains.then_some(TOOLCHAINS).into_iter().flatten())
                .map(move |(name, folder)| (*name, caches.join(folder)))
        }))
        .envs(
            caches
                .as_deref()
                .and_then(|caches| maven_opts(&opened.environment, caches))
                .map(|opts| ("MAVEN_OPTS", opts)),
        )
        .envs(offered.into_iter().flat_map(|offered| {
            offered
                .environment
                .iter()
                .map(|(name, value)| (name, value))
        }))
        .envs(
            caches
                .as_deref()
                .filter(|_| toolchains)
                .and_then(|caches| {
                    // The credential helper's `PATH`, when it has one, is the one to add to.
                    let environment: Vec<(OsString, OsString)> = opened
                        .environment
                        .iter()
                        .chain(
                            offered
                                .iter()
                                .flat_map(|offered| offered.environment.iter()),
                        )
                        .cloned()
                        .collect();
                    cargo_bin_on_path(&environment, caches)
                })
                .map(|path| ("PATH", path)),
        )
        // Someone else's git reads glave's settings, never the person's (`sandbox/shut.rs`).
        .envs(git_settings.map(|settings| ("GIT_CONFIG_GLOBAL", settings)))
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .env("PAGER", "cat")
        .env("GIT_PAGER", "cat")
        .stdout(Stdio::piped())
        .stderr(sandbox_says(sandbox.is_some()));
    if input {
        command.piped_input();
    } else {
        command.no_input();
    }
    command
}

/// What a sandboxed command's shell says first, `line`: that it runs inside (`started.rs`).
fn said_first(line: Option<&Line>) -> String {
    line.map(Line::said_first).unwrap_or_default()
}

/// Where the shell's own error output goes before it sends it to standard output: to glave,
/// which hears there whether a sandbox started the command; else nowhere.
fn sandbox_says(sandboxed: bool) -> Stdio {
    if sandboxed {
        Stdio::piped()
    } else {
        Stdio::null()
    }
}

/// How waiting for a command ended.
enum Waited {
    Ended(Ending),
    /// It reached its timeout still running, and is to be kept running as this command.
    Reached(CommandId),
}

/// Waits for the command to end, stopping it on a cancel, and on a timeout unless it `keeps`
/// running then, as the command it names.
async fn wait(
    child: &mut Child,
    limit: u64,
    cancel: &dyn Cancellation,
    keeps: Option<CommandId>,
) -> Result<Waited, ComputerError> {
    let waited = glave_ports::unless_cancelled(
        cancel,
        tokio::time::timeout(Duration::from_secs(limit), child.wait()),
    )
    .await;
    match waited {
        Some(Ok(Ok(status))) => Ok(Waited::Ended(exited(status))),
        Some(Ok(Err(error))) => Err(ComputerError::Failed(format!(
            "could not wait for the command: {error}"
        ))),
        Some(Err(_)) => {
            if let Some(id) = keeps {
                return Ok(Waited::Reached(id));
            }
            child.stop().await;
            Ok(Waited::Ended(Ending::TimedOut { after_secs: limit }))
        }
        None => {
            child.stop().await;
            Ok(Waited::Ended(Ending::Cancelled))
        }
    }
}

/// How a command that exited ended.
pub(crate) fn exited(status: std::process::ExitStatus) -> Ending {
    status
        .code()
        .map_or(Ending::Signalled, |code| Ending::Exited { code })
}
