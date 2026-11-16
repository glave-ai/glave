//! A person's Library (`specs/session.md`, "Library"): what glave made for them, kept beyond
//! any chat or computer, under the Library ledger's id in the project it was made in. Each
//! `library.kept` holds its file, a blob of the project's, under a holder of its own beside
//! the chat's, so deleting the chat keeps it; a `library.removed` of it takes it out, and its
//! holder lets go of the file.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::blob::BlobRef;
use crate::codec::{Body, Payload, parse};
use crate::id::{AssetId, ThreadId};

pub(crate) const KEPT: &str = "library.kept";
pub(crate) const REMOVED: &str = "library.removed";

/// What kind of thing a Library item is, which a client filters by.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    /// An image glave made (`image_generate`, `image_edit`).
    Picture,
}

/// Something glave made for the person, kept in their Library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryKept {
    pub asset: AssetId,
    pub kind: AssetKind,
    /// What it is called: the start of what it was made from, in the person's words.
    pub name: String,
    /// The name its file is saved as: `a-fox-at-dawn.png`.
    pub file_name: String,
    /// Its file, a blob of the project's.
    pub blob: BlobRef,
    /// The id its file is held under, its own, so it outlives the chat.
    pub holder: ThreadId,
    /// The chat it was made in.
    pub thread: ThreadId,
}

/// An item taken out of the Library: its holder has let go of its file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryRemoved {
    pub asset: AssetId,
}

impl Payload for LibraryKept {
    const TYPE: &'static str = KEPT;
    const VERSION: u32 = 1;
}

impl Payload for LibraryRemoved {
    const TYPE: &'static str = REMOVED;
    const VERSION: u32 = 1;
}

/// Decodes the Library ledger's events; `None` for any other type or version.
pub(crate) fn decode(
    kind: &str,
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<Body>> {
    if version != 1 {
        return None;
    }
    Some(match kind {
        KEPT => parse(data).map(Body::LibraryKept),
        REMOVED => parse(data).map(Body::LibraryRemoved),
        _ => return None,
    })
}
