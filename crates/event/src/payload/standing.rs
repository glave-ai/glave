//! `thread.archived`, `thread.unarchived` and `thread.deleted`: whether a thread is shown.
//! The latest of the three decides: archived threads are hidden from lists and kept in the
//! log, an archive can be undone, and a deleted thread is hidden for good and waits for
//! erasure (`specs/storage.md`).

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Payload, parse};
use crate::payload::principal::Principal;

pub(crate) const ARCHIVED: &str = "thread.archived";
pub(crate) const UNARCHIVED: &str = "thread.unarchived";
pub(crate) const DELETED: &str = "thread.deleted";

/// The thread was archived: hidden from lists, kept in the log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadArchived {
    /// Who archived it. Absent: the thread's `for`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Principal>,
}

/// The thread was brought back from the archive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadUnarchived {
    /// Who brought it back. Absent: the thread's `for`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Principal>,
}

/// The thread was deleted: hidden for good, and marked for erasure of its content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadDeleted {
    /// Who deleted it. Absent: the thread's `for`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Principal>,
}

impl Payload for ThreadArchived {
    const TYPE: &'static str = ARCHIVED;
    const VERSION: u32 = 1;
}

impl Payload for ThreadUnarchived {
    const TYPE: &'static str = UNARCHIVED;
    const VERSION: u32 = 1;
}

impl Payload for ThreadDeleted {
    const TYPE: &'static str = DELETED;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_archived(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<ThreadArchived>> {
    (version == 1).then(|| parse(data))
}

pub(crate) fn decode_unarchived(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<ThreadUnarchived>> {
    (version == 1).then(|| parse(data))
}

pub(crate) fn decode_deleted(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<ThreadDeleted>> {
    (version == 1).then(|| parse(data))
}
