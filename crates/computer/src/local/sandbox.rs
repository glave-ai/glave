//! The sandbox around commands on this machine (`specs/sandbox.md`): the operating system
//! refuses writes outside the workspace, reads of secret locations and, unless allowed, the
//! network, whatever a command tries. macOS only for now (Seatbelt); elsewhere a sandboxed
//! command is refused, never run unconfined.

mod profile;
#[cfg(test)]
mod tests;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use glave_ports::{ComputerError, Sandbox};

use profile::{Places, profile};

/// The only `sandbox-exec` trusted: an earlier one in `PATH` could be planted.
const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// Whether this machine can enforce the sandbox.
pub fn sandbox_available() -> bool {
    cfg!(target_os = "macos") && Path::new(SANDBOX_EXEC).is_file()
}

/// A sandbox ready to wrap commands in one workspace. Dropping it removes its temp folder.
#[derive(Debug)]
pub(crate) struct Prepared {
    /// What it was prepared from, so a command asking for another sandbox gets its own.
    pub(crate) sandbox: Sandbox,
    args: Vec<OsString>,
    temp: PathBuf,
}

impl Prepared {
    /// Resolves every path the profile names (macOS matches real paths) and writes the
    /// profile once, for every command run in `root` under `sandbox`.
    /// `git_common`, for a worktree, is its repository's shared git folder, writable too.
    pub(crate) async fn new(
        sandbox: &Sandbox,
        root: &Path,
        git_common: Option<&Path>,
    ) -> Result<Self, ComputerError> {
        if !sandbox_available() {
            return Err(ComputerError::Unsupported(
                "the sandbox is not available on this computer (macOS only for now)".into(),
            ));
        }
        let failed = |error: std::io::Error| {
            ComputerError::Failed(format!("could not prepare the sandbox: {error}"))
        };
        let system_temp = tokio::fs::canonicalize(std::env::temp_dir())
            .await
            .map_err(failed)?;
        let temp = run_temp(&system_temp).await.map_err(failed)?;
        let places = Places {
            root: root.to_owned(),
            temp: temp.clone(),
            home: resolved(sandbox.home.as_deref()).await,
            glave_home: resolved(sandbox.glave_home.as_deref()).await,
            git_common: resolved(git_common).await,
        };
        let profile = profile(&places, sandbox.level, sandbox.network);
        let mut args: Vec<OsString> = vec!["-p".into(), profile.text.into()];
        for (name, path) in profile.params {
            let mut arg = OsString::from(format!("-D{name}="));
            arg.push(path);
            args.push(arg);
        }
        args.push("--".into());
        Ok(Self {
            sandbox: sandbox.clone(),
            args,
            temp,
        })
    }

    /// The sandbox's own temp folder, which commands see as `TMPDIR`.
    pub(crate) fn temp(&self) -> &Path {
        &self.temp
    }

    /// The program to run instead of the command, and the arguments that go before it.
    pub(crate) fn wrapper(&self) -> (&str, &[OsString]) {
        (SANDBOX_EXEC, &self.args)
    }
}

impl Drop for Prepared {
    fn drop(&mut self) {
        // Best effort: the folder is in the system's temp folder, which is cleaned anyway.
        let _ = std::fs::remove_dir_all(&self.temp);
    }
}

/// A temp folder of the sandbox's own inside the system's, so commands cannot write other
/// programs' temp files.
async fn run_temp(system_temp: &Path) -> std::io::Result<PathBuf> {
    let pid = std::process::id();
    for attempt in 0_u32..1_000 {
        let dir = system_temp.join(format!("glave-sandbox-{pid}-{attempt}"));
        match tokio::fs::create_dir(&dir).await {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not make a temp folder for the sandbox",
    ))
}

/// A path resolved if it exists; one that does not exist needs no protecting yet.
async fn resolved(path: Option<&Path>) -> Option<PathBuf> {
    match path {
        Some(path) => tokio::fs::canonicalize(path).await.ok(),
        None => None,
    }
}
