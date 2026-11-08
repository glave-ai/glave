//! What changed in files, as one change at a time: what a computer says changed in a
//! workspace's folder since a point in its history (`files/changes`), and what the session API
//! says a thread changed, which is drawn from it.

use serde::{Deserialize, Serialize};

/// What changed in a workspace's folder since a point in its history (`files/changes`):
/// whatever changed it, a tool, a command or the person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Changes {
    /// Every file that differs from that point, in the order of their paths.
    pub files: Vec<ChangedFile>,
    /// Whether some of it was left out to keep the answer small: a file binary, named like
    /// one that holds secrets, or past the limit is named, with its counts, without its lines;
    /// past the most files listed, the rest are not named.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub shortened: bool,
}

/// A file, with every line its changes added and took out, and each change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ChangedFile {
    /// Where it is: relative to the workspace's root when its folder says, else as the call
    /// that changed it named it.
    pub path: String,
    pub added: u64,
    pub removed: u64,
    /// Its changes, in order: none for a file whose lines are not shown.
    pub changes: Vec<FileChange>,
}

/// One change to a file: a `write` or an `edit` that finished without an error, as its call
/// asked for it, which the tool applied exactly; or, as its folder says, the whole of a new
/// file or one stretch of an old one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FileChange {
    pub kind: ChangeKind,
    /// Its lines in order: those kept before it, those taken out, those put in, and those kept
    /// after it.
    pub lines: Vec<DiffLine>,
}

/// What a change did to its file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    /// Wrote the whole file (`write`, or a file new since).
    Wrote,
    /// Swapped one piece for another (`edit`, or a stretch of a file changed since).
    Edited,
    /// Swapped every match of a piece (`edit` with `replace_all`).
    EditedEvery,
}

/// One line of a change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DiffLine {
    pub kind: LineKind,
    pub text: String,
}

/// Whether a line was kept, taken out or put in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LineKind {
    Same,
    Removed,
    Added,
}
