//! What computers and workspaces are called, what is asked of them, and what they answer.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use super::bytes::Bytes;
use super::environment::{Environment, Size};
use super::sandbox::Sandbox;

/// Identifies a computer, unique everywhere (UUID v7, minted by the adapter that made it).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(transparent)]
pub struct ComputerId(Uuid);

/// Identifies a workspace, unique everywhere; threads reference it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkspaceId(Uuid);

macro_rules! uuid_id {
    ($id:ident) => {
        impl $id {
            pub fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            pub fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl fmt::Display for $id {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

uuid_id!(ComputerId);
uuid_id!(WorkspaceId);

/// Whether a computer can do work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Running,
    /// Its disk is kept and its compute freed; it wakes to work again.
    Sleeping,
    /// Destroyed, or never made by this adapter.
    Gone,
}

/// What a computer is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputerInfo {
    pub id: ComputerId,
    pub state: State,
    /// The operating system, as Rust names it: `macos`, `linux`.
    pub os: String,
    /// The processor, as Rust names it: `aarch64`, `x86_64`.
    pub arch: String,
    pub size: Size,
    /// The recipe it was made from; `None` once it is gone.
    pub environment: Option<Environment>,
}

/// What a workspace is checked out from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Sources {
    /// A folder already on the computer, worked on in place: the project the person is in.
    Folder { path: String },
    /// A repository cloned into a new copy, at a branch, tag or commit (its default branch
    /// when `None`).
    Git { url: String, at: Option<String> },
    /// A git worktree of the repository holding the folder at `path`, checked out on its own
    /// `branch` in a folder the computer chooses outside the repository. The branch is made
    /// from the folder's current commit when it does not exist yet. Opening the same branch
    /// again reopens the same worktree, and closing one keeps it: nothing in it is lost.
    /// `owner` is a label the computer keeps beside the worktree for whoever cleans it up
    /// later, such as the thread it belongs to; opening it again clears any hold on it.
    Worktree {
        path: String,
        branch: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        owner: Option<String>,
    },
    /// A folder of its own for a thread with no project, such as a one-off chat: made empty
    /// under `name` (letters, digits and `-`) in a place the computer chooses, and reopened,
    /// as it was left, when opened again. `owner` is kept beside it, as for a worktree.
    Scratch {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        owner: Option<String>,
    },
}

/// A workspace: where it is and what it came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    pub id: WorkspaceId,
    pub computer: ComputerId,
    /// The workspace root, an absolute path on its computer.
    pub root: String,
    pub sources: Sources,
    /// The home folder commands there see, when it is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<String>,
    /// The temp folder commands there see, when it is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temp: Option<String>,
}

/// One command, run by bash at the workspace root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Command {
    pub script: String,
    /// Seconds before it is stopped; the adapter's own limit when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    /// The sandbox it runs in; unconfined when `None`. A computer that cannot enforce it
    /// refuses the command as unsupported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<Sandbox>,
}

impl Command {
    pub fn new(script: impl Into<String>) -> Self {
        Self {
            script: script.into(),
            timeout_secs: None,
            sandbox: None,
        }
    }
}

/// How a command ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Ending {
    Exited {
        code: i32,
    },
    /// Stopped by a signal it did not ask for.
    Signalled,
    TimedOut {
        after_secs: u64,
    },
    /// The caller cancelled it.
    Cancelled,
}

/// A command that ran: how it ended and how long it took.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finished {
    pub ending: Ending,
    pub millis: u64,
}

/// One file to write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileWrite {
    /// Relative to the workspace root.
    pub path: String,
    pub content: Bytes,
    /// Refuse the write if the path the computer actually opens, links resolved, is one of the
    /// protected paths ([`PROTECTED`](super::PROTECTED), compared by
    /// [`is_protected`](super::is_protected)), with [`refuse_protected`](super::refuse_protected).
    /// The harness asks for it in a sandbox; the check and the write are one lookup, so a
    /// command swapping a folder for a link cannot slip a protected path in between.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub protect: bool,
}

impl FileWrite {
    /// `content` for `path`, with no protected-path check.
    pub fn new(path: impl Into<String>, content: Bytes) -> Self {
        Self {
            path: path.into(),
            content,
            protect: false,
        }
    }
}

/// What to list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listing {
    /// A folder, relative to the workspace root; `.` for the root.
    pub path: String,
    /// Everything under it, rather than only what it holds.
    #[serde(default)]
    pub recursive: bool,
    /// Leave out what git would ignore, even outside a repository, and the `.git` folder.
    #[serde(default)]
    pub skip_ignored: bool,
}

impl Listing {
    /// What `path` holds, everything included.
    pub fn of(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            recursive: false,
            skip_ignored: false,
        }
    }
}

/// What kind of entry a listing found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    File,
    Folder,
    Link,
}

/// One entry in a listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Relative to the workspace root, with `/` between names.
    pub path: String,
    pub kind: EntryKind,
    /// In bytes; 0 for folders.
    pub size: u64,
}

/// A listing, sorted by path. `truncated` when it stopped at the adapter's limit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listed {
    pub entries: Vec<Entry>,
    pub truncated: bool,
    /// Entries left out because git ignores them (not counting `.git`).
    #[serde(default)]
    pub ignored: u64,
}

/// What to save: everything in the workspace, committed to `branch`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Save {
    /// Such as `glave/<thread>`.
    pub branch: String,
    pub message: String,
}

/// A save: the commit made, and whether it reached where the sources came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Saved {
    pub commit: String,
    /// False for a repository with no `origin`: the commit stays in it.
    pub pushed: bool,
}

/// Why a computer could not do what was asked. Serializable, so it crosses the wire as it is.
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
#[serde(tag = "kind", content = "message", rename_all = "snake_case")]
pub enum ComputerError {
    /// No such computer, workspace or file.
    #[error("not found: {0}")]
    NotFound(String),
    /// Not allowed: a path outside the workspace, a file too large, a size beyond a cap.
    #[error("refused: {0}")]
    Refused(String),
    /// This computer cannot do it, such as run an image on the host.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// It was tried and failed.
    #[error("failed: {0}")]
    Failed(String),
    /// The computer could not be reached, or its answer made no sense.
    #[error("unreachable: {0}")]
    Unreachable(String),
}

impl ComputerError {
    /// What happened, without the kind in front.
    pub fn message(&self) -> &str {
        match self {
            Self::NotFound(message)
            | Self::Refused(message)
            | Self::Unsupported(message)
            | Self::Failed(message)
            | Self::Unreachable(message) => message,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_and_sources_have_stable_shapes() {
        let error = ComputerError::Refused("../x is outside the workspace".into());
        let json = serde_json::to_string(&error).unwrap();
        assert_eq!(
            json,
            r#"{"kind":"refused","message":"../x is outside the workspace"}"#
        );
        assert_eq!(serde_json::from_str::<ComputerError>(&json).unwrap(), error);
        let sources = Sources::Git {
            url: "https://example.com/r.git".into(),
            at: Some("main".into()),
        };
        let json = serde_json::to_string(&sources).unwrap();
        assert_eq!(
            json,
            r#"{"kind":"git","url":"https://example.com/r.git","at":"main"}"#
        );
        let ending = Ending::Exited { code: 3 };
        assert_eq!(
            serde_json::to_string(&ending).unwrap(),
            r#"{"kind":"exited","code":3}"#
        );
    }
}
