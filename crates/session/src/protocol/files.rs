//! Files a person adds to a message (`specs/session.md`, "Attachments"): uploaded first, kept
//! a day, then named by `turn/start`, which makes the thread refer to them.

use glave_event::Sha256;
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

/// A file `turn/start` adds to its message: one `file/upload` kept in the thread's project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AttachedFile {
    #[schemars(with = "String")]
    pub sha256: Sha256,
    /// Its name, shown with the message and told to the model.
    pub name: String,
}
