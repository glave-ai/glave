//! Files a thread's turns made for the person (`specs/session.md`, "Files it made"), as
//! `thread/files` gives them.

use glave_event::{AssetKind, Seq, Sha256};
use serde::{Deserialize, Serialize};

use super::Told;

/// A file a turn made for the person: kept with the thread, so it downloads after the computer
/// that made it is gone, or left there, said why.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MadeFile {
    pub kind: AssetKind,
    /// What it is called: its file's name without its end.
    pub name: String,
    /// The name its file is saved as.
    pub file_name: String,
    /// Its media type, such as `application/pdf`.
    pub media_type: String,
    /// Its size.
    pub bytes: u64,
    /// Its content's SHA-256, which `file/read` reads it by; absent when it was not kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<String>")]
    pub sha256: Option<Sha256>,
    /// Where it is kept, or why it was not, in a sentence: "Kept with this chat".
    pub kept: Told,
    /// What was made, in one line: "Made a spreadsheet: Trip budget".
    pub sentence: Told,
}

/// The files one turn made, in path order, by the `seq` of its `turn.started`.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TurnFiles {
    pub turn: Seq,
    pub files: Vec<MadeFile>,
    /// What `files` leave out, in a sentence, when they leave something out: more than a turn
    /// keeps, or a folder too large to look over.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<Told>,
}
