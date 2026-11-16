//! A person's Library (`specs/session.md`, "Library"): what glave made for them, as
//! `library/list` answers it, each item with the facts a client draws and filters by.

use glave_event::{AssetId, AssetKind, At, ThreadId};
use glave_ports::ProjectId;
use serde::{Deserialize, Serialize};

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
