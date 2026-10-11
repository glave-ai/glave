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

use glave_ports::{ComputerError, Sandbox};

/// The first argument that makes glave the Linux sandbox's helper (see [`sandbox_helper`]).
pub const SANDBOX_HELPER_ARG: &str = "__glave-sandbox";

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
        let (enforcer, temp) =
            platform::Enforcer::prepare(sandbox, root, git_common, helper).await?;
        Ok(Self {
            sandbox: sandbox.clone(),
            temp,
            enforcer,
        })
    }

    /// The sandbox's own temp folder, which commands see as `TMPDIR`.
    pub(crate) fn temp(&self) -> &Path {
        &self.temp
    }

    /// How to start the next command, from the workspace as it is now.
    pub(crate) async fn launch(&self) -> Result<Launch, ComputerError> {
        self.enforcer.launch().await
    }
}

/// A path resolved if it exists; one that does not exist needs no protecting yet.
#[cfg(any(target_os = "macos", target_os = "linux"))]
async fn resolved(path: Option<&Path>) -> Option<PathBuf> {
    match path {
        Some(path) => tokio::fs::canonicalize(path).await.ok(),
        None => None,
    }
}

/// What preparing the sandbox failed on.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn failed(error: &std::io::Error) -> ComputerError {
    ComputerError::Failed(format!("could not prepare the sandbox: {error}"))
}
