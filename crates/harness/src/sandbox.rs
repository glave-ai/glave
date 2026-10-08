//! The opt-in sandbox around commands: the operating system refuses writes outside the project,
//! reads of secret locations and, unless allowed, the network, whatever a command tries. See
//! `specs/sandbox.md`.

mod profile;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::Serialize;

use profile::{Places, profile};

/// The only `sandbox-exec` trusted: an earlier one in `PATH` could be planted.
const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// Where sandboxed commands may write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SandboxLevel {
    /// The temp folder only.
    ReadOnly,
    /// The project and the temp folder, except paths that make code run later.
    Project,
}

impl SandboxLevel {
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "read-only" => Some(Self::ReadOnly),
            "project" => Some(Self::Project),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::Project => "project",
        }
    }
}

/// What the sandbox allows, before it is prepared for a project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sandbox {
    pub level: SandboxLevel,
    /// Whether commands may use the network.
    pub network: bool,
    /// The person's home folder, whose credential folders commands may not read.
    pub home: Option<PathBuf>,
    /// glave's own data folder, which commands may neither read nor write.
    pub glave_home: Option<PathBuf>,
}

impl Sandbox {
    /// Whether this computer can enforce the sandbox. Without it, a requested sandbox means
    /// commands are refused, never run unconfined.
    pub fn available() -> bool {
        cfg!(target_os = "macos") && Path::new(SANDBOX_EXEC).is_file()
    }

    /// Resolves every path the profile names (macOS matches real paths) and writes the profile
    /// once, for every command run in `root`.
    pub(crate) async fn prepare(&self, root: &Path) -> std::io::Result<Prepared> {
        if !Self::available() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "the sandbox is not available on this computer (macOS only for now)",
            ));
        }
        let places = Places {
            root: root.to_owned(),
            temp: tokio::fs::canonicalize(std::env::temp_dir()).await?,
            home: resolved(self.home.as_deref()).await,
            glave_home: resolved(self.glave_home.as_deref()).await,
        };
        let profile = profile(&places, self.level, self.network);
        let mut args: Vec<OsString> = vec!["-p".into(), profile.text.into()];
        for (name, path) in profile.params {
            let mut arg = OsString::from(format!("-D{name}="));
            arg.push(path);
            args.push(arg);
        }
        args.push("--".into());
        Ok(Prepared {
            args,
            level: self.level,
            network: self.network,
        })
    }
}

/// A path resolved if it exists; one that does not exist needs no protecting yet.
async fn resolved(path: Option<&Path>) -> Option<PathBuf> {
    match path {
        Some(path) => tokio::fs::canonicalize(path).await.ok(),
        None => None,
    }
}

/// A sandbox ready to wrap commands in one project.
#[derive(Debug, Clone)]
pub(crate) struct Prepared {
    args: Vec<OsString>,
    level: SandboxLevel,
    network: bool,
}

impl Prepared {
    /// The program to run instead of the command, and the arguments that go before it.
    pub(crate) fn wrapper(&self) -> (&str, &[OsString]) {
        (SANDBOX_EXEC, &self.args)
    }

    /// What the thread records about the protections a turn ran under.
    pub(crate) fn recorded(&self) -> Recorded {
        Recorded {
            level: self.level,
            network: self.network,
            enforcement: "seatbelt",
        }
    }

    /// A note for the model when a failed command looks blocked by the sandbox, so it stops
    /// trying instead of guessing. Follows Codex's heuristic: a failure other than usage (2),
    /// not executable (126) or not found (127) that mentions a refusal.
    pub(crate) fn blocked_note(&self, exit: Option<i32>, output: &str) -> Option<&'static str> {
        if matches!(exit, Some(0 | 2 | 126 | 127)) {
            return None;
        }
        let output = output.to_ascii_lowercase();
        let refused = [
            "operation not permitted",
            "permission denied",
            "read-only file system",
            "sandbox",
            "could not resolve host",
            "nodename nor servname",
        ]
        .iter()
        .any(|sign| output.contains(sign));
        refused.then_some(if self.network {
            "[This command may have been blocked by the sandbox: commands here cannot write \
             outside the project, change git hooks or config, or read secrets. That cannot \
             be lifted from inside; do the task another way or tell the person what is needed.]"
        } else {
            "[This command may have been blocked by the sandbox: commands here cannot use the \
             network, write outside the project, change git hooks or config, or read secrets. \
             That cannot be lifted from inside; do the task another way or tell the person \
             what is needed.]"
        })
    }
}

/// The sandbox as recorded in a turn's configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) struct Recorded {
    level: SandboxLevel,
    network: bool,
    enforcement: &'static str,
}

#[cfg(test)]
mod tests;
