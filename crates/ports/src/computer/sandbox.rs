//! The sandbox a command runs in (`specs/sandbox.md`): what it may write, whether it may use
//! the network, and the paths that make code run later, which neither commands nor the file
//! tools may change. It travels with each command, so a computer here or across the wire
//! confines it the same way.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::ComputerError;

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

/// Whether `path` (resolved) is one of the project's protected paths. Compared the way a file
/// system that ignores case compares names (macOS folders usually do), so `.GIT/hooks` is
/// `.git/hooks`, and so is `.git/hook\u{17f}` with a long s, which APFS folds to `s`.
pub fn is_protected(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    let relative = PathBuf::from(fold(&relative.to_string_lossy()));
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

/// Why a change to a protected path is refused: the sentence after the path, as the model
/// reads it whichever side refused.
const REFUSED_BECAUSE: &str = " is protected by the sandbox: changing it could make code run \
     later, outside the sandbox. Ask the person to change it themselves.";

/// The refusal of a change to the protected `path`, as the computer gives it and the model
/// reads it.
pub fn refuse_protected(path: &str) -> ComputerError {
    ComputerError::Refused(format!("{path}{REFUSED_BECAUSE}"))
}

/// Whether `error` is a refusal of a change to a protected path ([`refuse_protected`]).
pub fn is_protected_refusal(error: &ComputerError) -> bool {
    matches!(error, ComputerError::Refused(message) if message.ends_with(REFUSED_BECAUSE))
}

/// A name as a file system that ignores case sees it: lowercase, and the one letter outside
/// ASCII that Unicode case folding turns into an ASCII letter lowercasing does not (`\u{17f}`,
/// long s, folds to `s`; the Kelvin sign already lowercases to `k`).
fn fold(name: &str) -> String {
    name.to_lowercase().replace('\u{17f}', "s")
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::computer::{Bytes, FileWrite};

    #[test]
    fn the_refusal_reads_as_it_always_has_and_is_recognized() {
        let refused = refuse_protected(".git/hooks/pre-commit");
        assert_eq!(
            refused,
            ComputerError::Refused(
                ".git/hooks/pre-commit is protected by the sandbox: changing it could make code \
                 run later, outside the sandbox. Ask the person to change it themselves."
                    .into()
            )
        );
        assert!(is_protected_refusal(&refused));
        assert!(!is_protected_refusal(&ComputerError::Refused(
            "x is outside the workspace (/p).".into()
        )));
        assert!(!is_protected_refusal(&ComputerError::Failed(
            refused.message().to_owned()
        )));
    }

    #[test]
    fn protect_crosses_the_wire_only_when_asked() {
        let plain = FileWrite::new("a.txt", Bytes::new(b"x"));
        let json = serde_json::to_value(&plain).unwrap();
        assert!(json.get("protect").is_none(), "{json}");
        let guarded = FileWrite {
            protect: true,
            ..plain
        };
        let json = serde_json::to_value(&guarded).unwrap();
        assert_eq!(json["protect"], true);
        assert_eq!(serde_json::from_value::<FileWrite>(json).unwrap(), guarded);
    }
}
