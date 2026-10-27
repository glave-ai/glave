//! What a thread changed in files with glave's own `write` and `edit`, as `thread/changes`
//! gives it.

use glave_event::Seq;
use serde::{Deserialize, Serialize};

use super::Told;

/// The files a thread changed, its subagents' changes with its own, and each turn's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ThreadChanges {
    /// Every file it changed, in the order first changed.
    pub files: Vec<ChangedFile>,
    /// Each turn that changed files, in order.
    pub turns: Vec<TurnChanges>,
    /// What they leave out, in a sentence: "Changes made by commands or other tools are not
    /// listed."
    pub note: Told,
}

/// The files one turn changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TurnChanges {
    /// The `seq` of the turn's `turn.started`.
    pub turn: Seq,
    pub files: Vec<ChangedFile>,
}

/// A file, with every line its changes added and took out, and each change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ChangedFile {
    /// Where it is, as the call named it.
    pub path: String,
    pub added: u64,
    pub removed: u64,
    /// Its changes, in order.
    pub changes: Vec<FileChange>,
}

/// One change to a file: a `write` or an `edit` that finished without an error, as its call
/// asked for it, which the tool applied exactly.
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
    /// Wrote the whole file (`write`).
    Wrote,
    /// Swapped one piece for another (`edit`).
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
