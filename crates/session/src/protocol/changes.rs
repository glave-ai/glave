//! What a thread changed in files, as `thread/changes` gives it: everything in its folder that
//! differs from where it started, and each turn's own edits.

use glave_event::Seq;
use serde::{Deserialize, Serialize};

pub use glave_ports::computer::{ChangeKind, ChangedFile, DiffLine, FileChange, LineKind};

use super::Told;

/// The files a thread changed, and each turn's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ThreadChanges {
    /// Every file in its folder that differs from where the thread started, whatever changed
    /// it, in the order of their paths; where its folder can't say, the files its own `write`
    /// and `edit` calls and its subagents' changed, in the order first changed.
    pub files: Vec<ChangedFile>,
    /// Each turn that changed files with its own `write` and `edit` calls, or its subagents',
    /// in order.
    pub turns: Vec<TurnChanges>,
    /// What they are and leave out, in a sentence: "Changes made by commands or other tools are
    /// not listed."
    pub note: Told,
}

/// The files one turn changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TurnChanges {
    /// The `seq` of the turn's `turn.started`.
    pub turn: Seq,
    pub files: Vec<ChangedFile>,
}
