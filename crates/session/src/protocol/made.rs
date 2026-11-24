//! Files a thread's turns made for the person (`specs/session.md`, "Files it made"), as
//! `thread/files` gives them, and one previewed, as `file/preview` gives it.

use glave_event::{AssetKind, Seq, Sha256};
use serde::{Deserialize, Serialize};

use super::Told;

/// A file a turn made for the person: kept with the thread, so it opens and downloads after the
/// computer that made it is gone, or left there, said why.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MadeFile {
    pub kind: AssetKind,
    /// What it is called: the title it gives itself, else its file's name without its end.
    pub name: String,
    /// The name its file is saved as.
    pub file_name: String,
    /// Its media type, such as `application/pdf`.
    pub media_type: String,
    /// Its size.
    pub bytes: u64,
    /// Its content's SHA-256, which `file/read` and `file/preview` read it by; absent when it
    /// was not kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<String>")]
    pub sha256: Option<Sha256>,
    /// How many pages, sheets or slides it has, in a phrase ("2 sheets"), when that is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parts: Option<Told>,
    /// Whether `file/preview` previews it (a spreadsheet, a CSV file, a document or a deck
    /// glave read), or a client draws it (a PDF glave read); false for any other, which only
    /// downloads.
    pub previews: bool,
    /// Where it is kept, or why it was not, in a sentence: "Kept with this chat".
    pub kept: Told,
    /// Why it can't be shown, only downloaded, when glave kept it but couldn't read it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unread: Option<Told>,
    /// What was made, in one line: "Made a spreadsheet: Trip budget (2 sheets)".
    pub sentence: Told,
    /// Who it is shared with, in a sentence, where it is shared: "Shared with 2 people", which
    /// a card shows in place of where it is kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared: Option<Told>,
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

/// A file a thread made, as a person previews it.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FilePreview {
    pub markdown: String,
    /// What was left out, in the server's sentence, when the preview is not the whole file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cut: Option<Told>,
}
