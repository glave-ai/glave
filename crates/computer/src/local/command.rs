//! Running one command at a workspace root: bash in a fresh process group, in the sandbox the
//! command carries, output streamed as it arrives; at a cancel stopped with everything it
//! started, and at a timeout stopped too, or kept running in the background when it asks.

use std::ffi::OsString;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use glave_ports::Cancellation;
use glave_ports::computer::{
    Command, CommandId, ComputerError, Ending, Finished, GitCredentials, OnTimeout, Output,
    Refusal, Sandbox, WorkspaceId,
};
use glave_process::{Child, Program};
use tokio::io::AsyncWriteExt as _;
use tokio::time::Instant;

use super::background::Backgrounds;
use super::credentials::{Offered, offer};
use super::process::Stream;
use super::registry::{Opened, new_id};
use super::sandbox::{Launch, Prepared};

const DEFAULT_TIMEOUT_SECS: u64 = 120;
const MAX_TIMEOUT_SECS: u64 = 600;
/// How long to keep reading after the command exits, in case a background child it started
/// still holds the output open.
pub(crate) const DRAIN: Duration = Duration::from_millis(500);
/// Package managers' caches, kept in the home folder by default, where sandboxed commands
/// cannot write. In the sandbox each goes in a folder of its own in glave's sandbox cache
/// folder (`~/.glave/sandbox-cache`), kept between runs so installs stay fast, or, without
/// glave's folder, in the command's own temp folder. Only sandboxed commands use it: nothing a
/// command leaves there reaches a command outside the sandbox, though it does reach later
/// sandboxed ones. Cargo's home is left alone: it holds the person's settings and installed
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

/// What a running command holds: the sandbox it was prepared in, and its own temp folder and
/// what launching it left in the workspace, both removed once it ends.
#[derive(Debug)]
pub(crate) struct Held {
    _launch: Option<Launch>,
    _sandbox: Option<Arc<Prepared>>,
}

/// Starts `script` by bash in `opened`, in `sandbox` when there is one, its git asking
/// `offered`'s helper when given. `helper` runs the Linux sandbox's helper, if this machine
/// has one.
pub(crate) async fn spawn(
    opened: &Opened,
    script: &str,
    sandbox: Option<&Sandbox>,
    helper: Option<&Path>,
    offered: Option<&Offered>,
) -> Result<Spawned, ComputerError> {
    if script.trim().is_empty() {
        return Err(ComputerError::Refused(Refusal::EmptyCommand.into()));
    }
    let prepared = match sandbox {
        Some(sandbox) => Some(opened.sandbox(sandbox, helper).await?),
        None => None,
    };
    let launch = match &prepared {
        Some(prepared) => Some(prepared.launch().await?),
        None => None,
    };
    let confined = launch.as_ref().zip(prepared.as_deref());
    let mut bash = bash(opened, script, confined, offered);
    let mut child = (opened.processes.spawn(&mut bash).await)
        .map_err(|error| ComputerError::Failed(error.to_string()))?;
    let input = launch.as_ref().and_then(|launch| launch.input.as_deref());
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        // Small (a seccomp filter), so it fits the pipe; a wrapper that failed early and
        // never reads it is reported by its own output.
        let _ = stdin.write_all(input).await;
    }
    let stream = Stream::of(&mut child);
    Ok(Spawned {
        child,
        stream,
        held: Held {
            _launch: launch,
            _sandbox: prepared,
        },
        started: Instant::now(),
    })
}

/// Where a command that reaches its timeout is kept running, when it asks to be: this
/// machine's commands in the background, in the workspace it runs in.
pub(crate) struct Keep<'a> {
    pub(crate) background: &'a Backgrounds,
    pub(crate) workspace: WorkspaceId,
    /// The cgroup counting the time the command is held at its CPU limit, where Linux counts
    /// it.
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
    let sandbox = command.sandbox.as_ref();
    let before = super::pressure::throttled(keep.cgroup).await;
    let mut spawned = spawn(opened, &command.script, sandbox, helper, offered.as_ref()).await?;
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
            let at = (opened.computer, keep.workspace);
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
    let after = super::pressure::throttled(keep.cgroup).await;
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

/// `script` run by bash at the workspace root, in the sandbox if there is one, with the
/// workspace's environment, the credential helper's settings if it has one, and output that
/// reads well as text.
fn bash(
    opened: &Opened,
    script: &str,
    sandbox: Option<(&Launch, &Prepared)>,
    offered: Option<&Offered>,
) -> Program {
    let mut command = match sandbox {
        Some((launch, _)) => {
            let mut command = Program::new(&launch.program);
            command.args(&launch.args).arg("bash");
            if launch.own_session {
                command.starts_own_session();
            }
            command
        }
        None => Program::new("bash"),
    };
    let input = sandbox.is_some_and(|(launch, _)| launch.input.is_some());
    let caches = sandbox.map(|(launch, prepared)| prepared.caches(launch));
    command
        .arg("-c")
        .arg(format!("exec 2>&1\n{script}"))
        .current_dir(&opened.root)
        .env_clear()
        .envs(opened.environment.iter().map(|(name, value)| (name, value)))
        .envs(sandbox.map(|(launch, _)| ("TMPDIR", launch.temp.path().to_path_buf())))
        .envs(caches.iter().flat_map(|caches| {
            CACHES
                .iter()
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
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .env("PAGER", "cat")
        .env("GIT_PAGER", "cat")
        .stdin(if input { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    command
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
