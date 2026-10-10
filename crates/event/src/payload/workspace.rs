//! `workspace.bound`: a thread is bound to a workspace on a computer, where its tools act.
//! Recorded once when the binding is made; resuming the thread reads the latest one to find
//! its workspace again.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use uuid::Uuid;

use crate::codec::{Payload, parse};

pub(crate) const BOUND: &str = "workspace.bound";

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
}

impl Payload for WorkspaceBound {
    const TYPE: &'static str = BOUND;
    const VERSION: u32 = 1;
}

pub(crate) fn decode(version: u32, data: &RawValue) -> Option<serde_json::Result<WorkspaceBound>> {
    match version {
        1 => Some(parse(data)),
        _ => None,
    }
}
