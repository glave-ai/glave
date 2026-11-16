//! Files a person adds to a message (`specs/session.md`, "Attachments"): uploaded first, kept
//! a day, then named by `turn/start`, which makes the thread refer to them. And files a call
//! made for the person ("Files it made"), as `thread/files` gives them.

use glave_event::{FileKind, Seq, Sha256};
use serde::{Deserialize, Serialize};

use super::Told;

/// A file a call made for the person, as `thread/files` gives it: kept with the thread, so it
/// opens and downloads after the computer that made it is gone.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MadeFile {
    /// The `seq` of the `tool.call.completed` that made it, as last made in its turn.
    pub seq: Seq,
    /// Its content's SHA-256, which `file/read` and `file/preview` read it by.
    #[schemars(with = "String")]
    pub sha256: Sha256,
    /// Its name as it was made, which it is saved under.
    pub name: String,
    /// What a card calls it: its name without the end that says its kind.
    pub title: String,
    pub kind: FileKind,
    /// Its media type, such as `application/pdf`.
    pub media_type: String,
    /// Its size.
    pub bytes: u64,
    /// Where it is kept, in a sentence: "Kept with this chat".
    pub kept: Told,
}

/// The files one turn made, by the `seq` of its `turn.started`.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TurnFiles {
    pub turn: Seq,
    /// Each file once, by where it was made, in the order first made, as last made.
    pub files: Vec<MadeFile>,
}

/// A file the server keeps, as `file/upload` answers it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FileInfo {
    /// Its content's SHA-256, which `turn/start` names it by.
    #[schemars(with = "String")]
    pub sha256: Sha256,
    /// Its name, as the person's computer called it.
    pub name: String,
    /// Its media type, such as `image/png`, as what it holds says.
    pub media_type: String,
    /// Its size.
    pub bytes: u64,
}

/// A piece of a file a thread holds, as `file/read` answers it: read a piece at a time, so a
/// file of any size comes within the 16 MiB a message may be on every transport.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FilePiece {
    /// Its media type, such as `image/png`, as what it holds says.
    pub media_type: String,
    /// The whole file's size.
    pub bytes: u64,
    /// Where in the file this piece starts.
    pub offset: u64,
    /// This piece, in base64.
    pub data: String,
    /// Where the next piece starts; absent at the file's end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<u64>,
}

/// A file `turn/start` adds to its message: one `file/upload` kept in the thread's project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AttachedFile {
    #[schemars(with = "String")]
    pub sha256: Sha256,
    /// Its name, shown with the message and told to the model.
    pub name: String,
}
