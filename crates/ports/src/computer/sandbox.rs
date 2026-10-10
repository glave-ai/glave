//! The sandbox a command runs in (`specs/sandbox.md`): what it may write, whether it may use
//! the network, and the paths that make code run later, which neither commands nor the file
//! tools may change. It travels with each command, so a computer here or across the wire
//! confines it the same way.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Inside the project: paths that make code run later, outside the sandbox. Neither commands
/// nor glave's file tools may change them in a sandbox, even before they exist.
pub const PROTECTED: &[&str] = &[
    ".git/hooks",
    ".git/config",
    ".git/config.worktree",
    "HEAD",
    "objects",
    "refs",
    ".glave",
    ".mcp.json",
    ".vscode",
    ".idea",
    ".envrc",
];

/// Whether `path` (resolved) is one of the project's protected paths. Compared without case:
/// macOS folders usually ignore it, so `.GIT/hooks` is `.git/hooks`.
pub fn is_protected(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    let relative = PathBuf::from(relative.to_string_lossy().to_lowercase());
    let parts: Vec<String> = relative
        .iter()
        .map(|part| part.to_string_lossy().into_owned())
        .collect();
    let nested_git_runs_code = match parts.as_slice() {
        [git, kind, _, rest @ ..] => {
            git == ".git"
                && (kind == "worktrees" || kind == "modules")
                && rest.first().is_some_and(|part| {
                    ["hooks", "config", "config.worktree"].contains(&part.as_str())
                })
        }
        _ => false,
    };
    relative == Path::new(".git")
        || nested_git_runs_code
        || PROTECTED
            .iter()
            .any(|protected| relative.starts_with(protected.to_lowercase()))
}

/// Where sandboxed commands may write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

/// What the sandbox allows. A computer that cannot enforce it refuses the command rather
/// than running it unconfined.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Sandbox {
    pub level: SandboxLevel,
    /// Whether commands may use the network.
    pub network: bool,
    /// The person's home folder, whose credential folders commands may not read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<PathBuf>,
    /// glave's own data folder, which commands may neither read nor write.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glave_home: Option<PathBuf>,
}
