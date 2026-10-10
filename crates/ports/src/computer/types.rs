//! What computers and workspaces are called, what is asked of them, and what they answer.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use super::bytes::Bytes;
use super::environment::{Environment, Size};

/// Identifies a computer, unique everywhere (UUID v7, minted by the adapter that made it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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
}

/// A workspace: where it is and what it came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    pub id: WorkspaceId,
    pub computer: ComputerId,
    /// The workspace root, an absolute path on its computer.
    pub root: String,
    pub sources: Sources,
}

/// One command, run by bash at the workspace root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Command {
    pub script: String,
    /// Seconds before it is stopped; the adapter's own limit when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
}

impl Command {
    pub fn new(script: impl Into<String>) -> Self {
        Self {
            script: script.into(),
            timeout_secs: None,
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
}

/// What to list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listing {
    /// A folder, relative to the workspace root; `.` for the root.
    pub path: String,
    /// Everything under it, rather than only what it holds.
    #[serde(default)]
    pub recursive: bool,
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
    /// False for a folder with nowhere to push to: the commit is kept in its own history.
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
