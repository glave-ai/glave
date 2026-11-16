//! Files a person adds to a message (`specs/session.md`, "Attachments"): uploaded first, kept
//! a day, then named by `turn/start`, which makes the thread refer to them; files read back a
//! piece at a time; and what glave made for the person, kept in their Library ("Library").

use glave_event::{AssetId, AssetKind, At, Sha256, ThreadId};
use glave_ports::ProjectId;
use serde::{Deserialize, Serialize};

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

/// Something glave made for the person, kept in their Library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct LibraryItem {
    /// What `library/read` and `library/remove` name it by, with its `project`; a media tool's
    /// result calls it `asset:<id>`.
    pub asset: AssetId,
    pub kind: AssetKind,
    /// What it is called: the start of what it was made from, in the person's words.
    pub name: String,
    /// The name its file is saved as.
    pub file_name: String,
    /// Its media type, such as `image/png`.
    pub media_type: String,
    /// Its file's size.
    pub bytes: u64,
    /// The project it was made in, the person's Chats among them.
    pub project: ProjectId,
    pub made_at: At,
    /// The chat it was made in, while that chat is there: absent once it was deleted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat: Option<LibraryChat>,
}

/// The chat a Library item was made in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct LibraryChat {
    pub thread: ThreadId,
    /// Its title, once it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}
