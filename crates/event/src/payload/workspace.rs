//! `workspace.bound`: a thread is bound to a workspace on a computer, where its tools act.
//! Recorded once when the binding is made; resuming the thread reads the latest one to find
//! its workspace again.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use uuid::Uuid;

use crate::codec::{Payload, parse};

pub(crate) const BOUND: &str = "workspace.bound";
pub(crate) const REMOVED: &str = "workspace.removed";

/// The thread's tools act in this workspace from now on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceBound {
    /// The workspace's id, as its computer named it.
    pub workspace: Uuid,
    pub computer: ComputerRef,
    pub opened: Opened,
    /// The workspace root, an absolute path as the computer names it. Tool output and glave's
    /// own fields use paths relative to it.
    pub root: String,
}

/// The computer a workspace is on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComputerRef {
    pub id: Uuid,
    pub kind: ComputerKind,
}

/// What kind of computer it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerKind {
    /// The machine glave runs on.
    Local,
    /// A computer glave made elsewhere, such as a cloud worker.
    Worker,
}

/// How the workspace was opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Opened {
    /// A folder worked on in place.
    Folder { path: String },
    /// A git worktree of the repository at `repository`, on its own `branch`.
    Worktree { repository: String, branch: String },
    /// A repository cloned into a new copy, at a branch, tag or commit (its default branch
    /// when absent).
    Clone {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<String>,
    },
    /// A folder of the thread's own, for a thread with no project.
    Scratch { path: String },
}

/// The thread's worktree was removed, or moved to the Trash, by a cleanup. Its branch is kept,
/// so resuming the thread checks it out again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceRemoved {
    /// The worktree's folder, as its computer named it.
    pub root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub reason: RemovedReason,
    /// Where it went when it was moved to the Trash rather than removed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trash: Option<String>,
    /// For a released worker whose work could not be checked or saved before it was
    /// destroyed: why. Anything it had not pushed is gone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lost: Option<String>,
}

/// Why a worktree was removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemovedReason {
    /// Its thread was archived or deleted.
    Archived,
    /// Its thread had no events for 30 days.
    Idle,
    /// Its repository no longer exists; it went to the Trash.
    RepositoryGone,
    /// The person pruned it.
    Pruned,
    /// Its worker was released when its session ended: unsaved work was pushed to `branch`
    /// first, and the worker destroyed.
    Released,
}

impl Payload for WorkspaceBound {
    const TYPE: &'static str = BOUND;
    const VERSION: u32 = 1;
}

impl Payload for WorkspaceRemoved {
    const TYPE: &'static str = REMOVED;
    /// 2 added `lost`.
    const VERSION: u32 = 1;
}

pub(crate) fn decode(version: u32, data: &RawValue) -> Option<serde_json::Result<WorkspaceBound>> {
    match version {
        1 => Some(parse(data)),
        _ => None,
    }
}

pub(crate) fn decode_removed(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<WorkspaceRemoved>> {
    match version {
        1 => Some(parse(data)),
        _ => None,
    }
}
