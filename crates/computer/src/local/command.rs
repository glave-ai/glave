//! Running one command at a workspace root: bash in a fresh process group, output streamed
//! as it arrives, stopped with everything it started on a timeout or a cancel.

use std::ffi::OsString;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use glave_ports::{Cancellation, Command, ComputerError, Credentials, Ending, Finished, Output};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::process::{Child, ChildStdout};

use super::credentials::{Offered, offer};
use super::registry::Opened;
use super::sandbox::Launch;

const DEFAULT_TIMEOUT_SECS: u64 = 120;
const MAX_TIMEOUT_SECS: u64 = 600;
/// How long to keep reading after the command exits, in case a background child it started
/// still holds the output open.
const DRAIN: Duration = Duration::from_millis(500);
/// How long a stopped command gets between SIGTERM and SIGKILL.
const GRACE: Duration = Duration::from_secs(3);
/// Package managers' caches, kept in the home folder by default, where sandboxed commands
/// cannot write. In the sandbox each goes in a folder of its own in glave's sandbox cache
/// folder (`~/.glave/sandbox-cache`), kept between runs so installs stay fast, or, without
/// glave's folder, in the sandbox's temp folder. Only sandboxed commands use it: nothing a
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
/// which is split at spaces: a cache folder with a space in its path is left out.
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

/// Runs `command` in `opened`. `helper` runs the Linux sandbox's helper, if this machine has one.
/// Git and the forge tools ask `credentials`, if given, for the hosts it names, at each use.
pub(crate) async fn run(
    opened: &Opened,
    command: &Command,
    output: &dyn Output,
    cancel: &dyn Cancellation,
    helper: Option<&Path>,
    credentials: Option<&dyn Credentials>,
) -> Result<Finished, ComputerError> {
    if command.script.trim().is_empty() {
        return Err(ComputerError::Refused("the command is empty".into()));
    }
    let limit = command
        .timeout_secs
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
        .clamp(1, MAX_TIMEOUT_SECS);
    let sandbox = match &command.sandbox {
        Some(sandbox) => Some(opened.sandbox(sandbox, helper).await?),
        None => None,
    };
    // Held until the command ends: what launching it left in the workspace is removed then.
    let launch = match &sandbox {
        Some(sandbox) => Some((sandbox.launch().await?, sandbox.temp(), sandbox.caches())),
        None => None,
    };
    // Held until the command ends: its helper's folder and listener go with it.
    let offered = match credentials {
        Some(credentials) => offer(credentials, &opened.environment).await?,
        None => None,
    };
    let started = Instant::now();
    let mut child = bash(opened, &command.script, launch.as_ref(), offered.as_ref())
        .spawn()
        .map_err(|error| ComputerError::Failed(format!("could not start bash: {error}")))?;
    let input = launch
        .as_ref()
        .and_then(|(launch, _, _)| launch.input.as_deref());
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        // Small (a seccomp filter), so it fits the pipe; a wrapper that failed early and
        // never reads it is reported by its own output.
        let _ = stdin.write_all(input).await;
    }
    let stdout = child.stdout.take();
    let pump = pump(stdout, output);
    let waiting = answering(
        wait(&mut child, limit, cancel),
        offered.as_ref().zip(credentials),
    );
    tokio::pin!(pump, waiting);
    let (ending, pumped) = tokio::select! {
        ending = &mut waiting => (ending, false),
        () = &mut pump => (waiting.await, true),
    };
    if !pumped {
        let _ = tokio::time::timeout(DRAIN, pump).await;
    }
    let millis = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok(Finished {
        ending: ending?,
        millis,
    })
}

/// `work`, with the command's credential helper answered meanwhile, when it has one.
async fn answering<T>(
    work: impl Future<Output = T>,
    helper: Option<(&Offered, &dyn Credentials)>,
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
    sandbox: Option<&(Launch, &Path, &Path)>,
    offered: Option<&Offered>,
) -> tokio::process::Command {
    let mut command = match sandbox {
        Some((launch, _, _)) => {
            let mut command = tokio::process::Command::new(&launch.program);
            command.args(&launch.args).arg("bash");
            command
        }
        None => tokio::process::Command::new("bash"),
    };
    let input = sandbox.is_some_and(|(launch, _, _)| launch.input.is_some());
    command
        .arg("-c")
        .arg(format!("exec 2>&1\n{script}"))
        .current_dir(&opened.root)
        .env_clear()
        .envs(opened.environment.iter().map(|(name, value)| (name, value)))
        .envs(sandbox.map(|(_, temp, _)| ("TMPDIR", temp.to_path_buf())))
        .envs(sandbox.into_iter().flat_map(|(_, _, caches)| {
            CACHES
                .iter()
                .map(move |(name, folder)| (*name, caches.join(folder)))
        }))
        .envs(
            sandbox
                .and_then(|(_, _, caches)| maven_opts(&opened.environment, caches))
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
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    if !sandbox.is_some_and(|(launch, _, _)| launch.own_session) {
        command.process_group(0);
    }
    command
}

/// Waits for the command to end, stopping it on a timeout or a cancel.
async fn wait(
    child: &mut Child,
    limit: u64,
    cancel: &dyn Cancellation,
) -> Result<Ending, ComputerError> {
    let waited = glave_ports::unless_cancelled(
        cancel,
        tokio::time::timeout(Duration::from_secs(limit), child.wait()),
    )
    .await;
    match waited {
        Some(Ok(Ok(status))) => Ok(status
            .code()
            .map_or(Ending::Signalled, |code| Ending::Exited { code })),
        Some(Ok(Err(error))) => Err(ComputerError::Failed(format!(
            "could not wait for the command: {error}"
        ))),
        Some(Err(_)) => {
            stop(child).await;
            Ok(Ending::TimedOut { after_secs: limit })
        }
        None => {
            stop(child).await;
            Ok(Ending::Cancelled)
        }
    }
}

/// Stops a command and everything it started: SIGTERM to its process group, then SIGKILL to
/// whatever of the group is still there after a grace period. Returns once the group is gone,
/// not just bash: a child that outlives bash, or ignores SIGTERM, is stopped too.
async fn stop(child: &mut Child) {
    #[cfg(unix)]
    if let Some(group) = child
        .id()
        .and_then(|pid| i32::try_from(pid).ok())
        .and_then(rustix::process::Pid::from_raw)
    {
        use rustix::process::{Signal, kill_process_group};
        let _ = kill_process_group(group, Signal::TERM);
        if tokio::time::timeout(GRACE, gone(child, group))
            .await
            .is_err()
        {
            let _ = kill_process_group(group, Signal::KILL);
            // SIGKILL cannot be ignored; this waits only for the kernel to finish the job.
            let _ = tokio::time::timeout(GRACE, gone(child, group)).await;
        }
    }
    let _ = child.kill().await;
}

/// How often a stopped command's process group is checked for anything left in it.
#[cfg(unix)]
const POLL: Duration = Duration::from_millis(10);

/// Resolves once bash has exited and nothing is left in its process group, not even a process
/// waiting to be reaped. Glave's workers run with an init that reaps orphans (Docker's `Init`,
/// the Pod's shared process namespace), as does any desktop.
#[cfg(unix)]
async fn gone(child: &mut Child, group: rustix::process::Pid) {
    let _ = child.wait().await;
    while rustix::process::test_kill_process_group(group).is_ok() {
        tokio::time::sleep(POLL).await;
    }
}

/// Streams the command's output to `output` as text until it ends.
async fn pump(stdout: Option<ChildStdout>, output: &dyn Output) {
    let Some(mut stdout) = stdout else {
        return;
    };
    let mut buffer = vec![0u8; 16 * 1024];
    let mut text = Utf8::default();
    while let Ok(read) = stdout.read(&mut buffer).await {
        let Some(chunk) = buffer.get(..read).filter(|chunk| !chunk.is_empty()) else {
            break;
        };
        let decoded = text.push(chunk);
        if !decoded.is_empty() {
            output.write(&decoded).await;
        }
    }
    let rest = text.finish();
    if !rest.is_empty() {
        output.write(&rest).await;
    }
}

/// Turns bytes into text without splitting a character across chunks.
#[derive(Debug, Default)]
pub(crate) struct Utf8 {
    pending: Vec<u8>,
}

impl Utf8 {
    pub(crate) fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let cut = match std::str::from_utf8(&self.pending) {
            Err(error) if error.error_len().is_none() => error.valid_up_to(),
            _ => self.pending.len(),
        };
        let rest = self.pending.split_off(cut);
        let text = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending = rest;
        text
    }

    pub(crate) fn finish(&mut self) -> String {
        let text = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        text
    }
}
