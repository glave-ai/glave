//! The sandbox around commands on this machine (`specs/sandbox.md`): the operating system
//! refuses writes outside the workspace, reads of secret locations and, unless allowed, the
//! network, whatever a command tries. macOS uses Seatbelt; Linux uses bubblewrap with a
//! seccomp filter, or Landlock with the same filter at the `read-only` level where bubblewrap
//! cannot start. Elsewhere a sandboxed command is refused, never run unconfined.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "macos", test))]
mod profile;
#[cfg(any(target_os = "macos", target_os = "linux", test))]
mod secrets;
#[cfg(test)]
mod tests;
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod unsupported;

#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
use unsupported as platform;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use glave_ports::computer::{ComputerError, Sandbox};

/// The first argument that makes glave the Linux sandbox's helper (see [`sandbox_helper`]).
pub(crate) const SANDBOX_HELPER_ARG: &str = "__glave-sandbox";

/// Whether this machine can enforce every sandbox level: Seatbelt on macOS, bubblewrap on
/// Linux. (Landlock, Linux's fallback, enforces only `read-only`, and only for a program that
/// names its helper.)
pub fn sandbox_available() -> bool {
    platform::available()
}

/// Runs this process as the Linux sandbox's helper when `args` (the whole command line) are
/// the helper's: `glave __glave-sandbox <plan> -- <command>…` confines itself with Landlock
/// and seccomp, then becomes the command. Returns `None` when they are not, and otherwise only
/// when confining or starting the command failed, saying what went wrong. A program that
/// passes its own path to [`crate::LocalComputers::with_sandbox_helper`] calls this first
/// thing in `main`, before it starts any threads.
pub fn sandbox_helper(args: &[OsString]) -> Option<String> {
    if args.get(1).is_none_or(|first| first != SANDBOX_HELPER_ARG) {
        return None;
    }
    Some(platform::helper(args.get(2..).unwrap_or_default()))
}

/// A sandbox ready to wrap commands in one workspace. Dropping it removes its temp folder.
#[derive(Debug)]
pub(crate) struct Prepared {
    /// What it was prepared from, so a command asking for another sandbox gets its own.
    pub(crate) sandbox: Sandbox,
    temp: PathBuf,
    caches: PathBuf,
    enforcer: platform::Enforcer,
}

/// How to start one command in the sandbox: a program and the arguments before the command.
#[derive(Debug)]
pub(crate) struct Launch {
    pub(crate) program: PathBuf,
    pub(crate) args: Vec<OsString>,
    /// Bytes for the wrapper's standard input, which is then closed: Linux's seccomp filter.
    pub(crate) input: Option<Vec<u8>>,
    /// The wrapper starts a session of its own, so the command must not get a process group
    /// first (a group leader cannot start a session).
    pub(crate) own_session: bool,
    /// What launching left in the workspace, removed once the command has ended.
    pub(crate) _held: platform::Held,
}

impl Prepared {
    /// Prepares `sandbox` for every command run in `root`. `git_common`, for a worktree, is
    /// its repository's shared git folder, writable too. `helper` is a program that runs
    /// [`sandbox_helper`], which Linux needs where bubblewrap cannot start.
    pub(crate) async fn new(
        sandbox: &Sandbox,
        root: &Path,
        git_common: Option<&Path>,
        helper: Option<&Path>,
    ) -> Result<Self, ComputerError> {
        let cache = cache_home(sandbox.glave_home.as_deref()).await;
        let (enforcer, temp) =
            platform::Enforcer::prepare(sandbox, root, git_common, helper, cache.as_deref())
                .await?;
        let caches = cache.unwrap_or_else(|| temp.join("caches"));
        Ok(Self {
            sandbox: sandbox.clone(),
            temp,
            caches,
            enforcer,
        })
    }

    /// The sandbox's own temp folder, which commands see as `TMPDIR`.
    pub(crate) fn temp(&self) -> &Path {
        &self.temp
    }

    /// Where package managers keep their caches in the sandbox: glave's sandbox cache folder,
    /// kept between runs, or without glave's folder one in the sandbox's temp folder.
    pub(crate) fn caches(&self) -> &Path {
        &self.caches
    }

    /// How to start the next command, from the workspace as it is now.
    pub(crate) async fn launch(&self) -> Result<Launch, ComputerError> {
        self.enforcer.launch().await
    }
}

/// The folder in glave's own where sandboxed commands keep package caches between runs.
pub(crate) const CACHE_FOLDER: &str = "sandbox-cache";

/// glave's sandbox cache folder, made private if it is not there yet, resolved; none without
/// glave's own folder.
async fn cache_home(glave_home: Option<&Path>) -> Option<PathBuf> {
    let glave_home = tokio::fs::canonicalize(glave_home?).await.ok()?;
    let cache = glave_home.join(CACHE_FOLDER);
    let mut builder = tokio::fs::DirBuilder::new();
    #[cfg(unix)]
    builder.mode(0o700);
    match builder.create(&cache).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return None,
    }
    // A link planted in its place would lead the cache, and its writes, elsewhere.
    let meta = tokio::fs::symlink_metadata(&cache).await.ok()?;
    meta.is_dir().then_some(cache)
}

/// A path resolved if it exists; one that does not exist needs no protecting yet.
#[cfg(any(target_os = "macos", target_os = "linux"))]
async fn resolved(path: Option<&Path>) -> Option<PathBuf> {
    match path {
        Some(path) => tokio::fs::canonicalize(path).await.ok(),
        None => None,
    }
}

/// A worktree's own folder in its repository's shared git folder, from the `gitdir:` line
/// of its `.git` file, if it is where a worktree's folder belongs. Blocking.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn own_git_folder(root: &Path, common: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(root.join(".git")).ok()?;
    let line = text.lines().find_map(|line| line.strip_prefix("gitdir:"))?;
    let own = std::fs::canonicalize(root.join(line.trim())).ok()?;
    (own.parent() == Some(&common.join("worktrees"))).then_some(own)
}

/// What preparing the sandbox failed on.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn failed(error: &std::io::Error) -> ComputerError {
    ComputerError::Failed(format!("could not prepare the sandbox: {error}"))
}
