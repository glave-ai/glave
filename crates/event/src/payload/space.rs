//! The space ledger's records (`specs/storage.md`, "Spaces"). `space.folder.set`: one of a
//! project's folders, where its threads work; for each folder, the latest one holds. Where the
//! folder's files come from is kept apart from where they are checked out, so the project can
//! be checked out again, on another computer or after one is reset, from its source alone.
//! `space.removed`: the project was put away, its threads archived; a folder set after it
//! brings it back.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use uuid::Uuid;

use crate::codec::{Payload, parse};
use crate::payload::principal::Principal;

pub(crate) const FOLDER_SET: &str = "space.folder.set";
pub(crate) const REMOVED: &str = "space.removed";

/// A project's folder was set: added, or its source or checkout changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpaceFolderSet {
    /// Which of the project's folders this is: a UUID v7 chosen when it was first set, which
    /// every later record of the same folder names.
    pub folder: Uuid,
    /// Where its files come from.
    pub source: FolderSource,
    /// Where it is checked out.
    pub checkout: Checkout,
    /// Who set it. Absent: the space's person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Principal>,
}

/// Where a folder's files come from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FolderSource {
    /// A git repository, by the address it is cloned from: where the folder comes back from.
    Repository { url: String },
    /// A folder on a computer, with no repository it comes from: only its checkout has it.
    Local,
}

/// Where a folder is checked out: a computer, and the path on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Checkout {
    /// The computer, by the id it keeps across runs.
    pub computer: Uuid,
    /// The folder's absolute path there, links resolved.
    pub path: String,
}

/// A project was removed: its threads archived, and it is listed no more until a folder is set
/// for it again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpaceRemoved {
    /// Who removed it. Absent: the space's person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Principal>,
}

impl Payload for SpaceRemoved {
    const TYPE: &'static str = REMOVED;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_removed(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<SpaceRemoved>> {
    (version == 1).then(|| parse(data))
}

impl Payload for SpaceFolderSet {
    const TYPE: &'static str = FOLDER_SET;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_folder_set(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<SpaceFolderSet>> {
    (version == 1).then(|| parse(data))
}
