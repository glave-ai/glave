//! `memory.saved`, `memory.edited`, `memory.forgotten`, `memory.restored` and `memory.tidied`: what glave
//! remembers for later threads (`specs/memory.md`). They are kept in the project's memory ledger,
//! glave's own stream under a reserved id, never in a conversation. A memory's text is sealed
//! under a data key of its own, so forgetting it destroys every version at once.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Payload, parse};
use crate::envelope::{At, RawEvent, Seq};
use crate::id::{EventId, MemoryId, ThreadId};
use crate::payload::principal::Principal;

pub(crate) const SAVED: &str = "memory.saved";
pub(crate) const EDITED: &str = "memory.edited";
pub(crate) const FORGOTTEN: &str = "memory.forgotten";
pub(crate) const RESTORED: &str = "memory.restored";
pub(crate) const TIDIED: &str = "memory.tidied";

/// Where a memory belongs: to the person, or to the project it was saved in.
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
pub enum MemoryScope {
    /// About the person: read in their threads, in projects that allow it.
    Personal,
    /// About the project's work: read by every member and agent working in it.
    Project,
}

impl MemoryScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Personal => "personal",
            Self::Project => "project",
        }
    }

    /// The scope named `name`: `personal` or `project`.
    pub fn named(name: &str) -> Option<Self> {
        match name.trim() {
            "personal" => Some(Self::Personal),
            "project" => Some(Self::Project),
            _ => None,
        }
    }
}

/// Where in a thread a memory was saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemorySource {
    pub thread: ThreadId,
    pub seq: Seq,
}

/// A memory was saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemorySaved {
    pub memory: MemoryId,
    pub scope: MemoryScope,
    /// The statement, at most 500 characters.
    pub text: String,
    /// The thread and `seq` it was saved in, when it was saved in a thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<MemorySource>,
    /// Whether it was saved in a thread that had read untrusted content or a secrets file.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub untrusted_read: bool,
    /// Who saved it.
    pub by: Principal,
}

/// Version 1, before spaces were projects: a memory of the project was saved as `space`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemorySavedV1 {
    memory: MemoryId,
    scope: ScopeV1,
    text: String,
    #[serde(default)]
    source: Option<MemorySource>,
    #[serde(default)]
    untrusted_read: bool,
    by: Principal,
}

/// A scope as version 1 named it.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ScopeV1 {
    Personal,
    Space,
}

impl From<MemorySavedV1> for MemorySaved {
    fn from(old: MemorySavedV1) -> Self {
        Self {
            memory: old.memory,
            scope: match old.scope {
                ScopeV1::Personal => MemoryScope::Personal,
                ScopeV1::Space => MemoryScope::Project,
            },
            text: old.text,
            source: old.source,
            untrusted_read: old.untrusted_read,
            by: old.by,
        }
    }
}

/// A memory's text changed. Tidy-up merges are edits too.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryEdited {
    pub memory: MemoryId,
    pub text: String,
    /// Who changed it.
    pub by: Principal,
}

/// A memory was forgotten. Content-free.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryForgotten {
    pub memory: MemoryId,
    /// When its key is destroyed: at once when the person forgets, 30 days on when glave does.
    pub erase_at: At,
    /// Where it was saved, kept here because the saved event's text (and its source with it)
    /// is erased: `recall` never returns that turn of the conversation again. Since version 2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<MemorySource>,
    /// Who forgot it.
    pub by: Principal,
}

/// Version 1, before a forget named where the memory was saved.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryForgottenV1 {
    memory: MemoryId,
    erase_at: At,
    by: Principal,
}

impl From<MemoryForgottenV1> for MemoryForgotten {
    fn from(old: MemoryForgottenV1) -> Self {
        Self {
            memory: old.memory,
            erase_at: old.erase_at,
            source: None,
            by: old.by,
        }
    }
}

/// A change glave made to a memory was undone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryRestored {
    pub memory: MemoryId,
    /// The event undone: glave's `memory.edited` or `memory.forgotten`.
    pub undone: EventId,
    /// Who undid it.
    pub by: Principal,
}

/// Tidy-up ran over a ledger. Its changes are `memory.edited` and `memory.forgotten` events of
/// their own, `by` glave; this records that it looked, and the pairs of memories it could not
/// reconcile, which wait for the person to say which holds. Content-free.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryTidied {
    /// Pairs of memories that disagree where tidy-up could not tell which holds: both are
    /// kept, and the person is asked, until one of them changes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub questions: Vec<[MemoryId; 2]>,
    /// Who tidied: glave.
    pub by: Principal,
}

impl Payload for MemorySaved {
    const TYPE: &'static str = SAVED;
    const VERSION: u32 = 2;
}

impl Payload for MemoryEdited {
    const TYPE: &'static str = EDITED;
    const VERSION: u32 = 1;
}

impl Payload for MemoryForgotten {
    const TYPE: &'static str = FORGOTTEN;
    const VERSION: u32 = 2;
}

impl Payload for MemoryRestored {
    const TYPE: &'static str = RESTORED;
    const VERSION: u32 = 1;
}

impl Payload for MemoryTidied {
    const TYPE: &'static str = TIDIED;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_saved(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<MemorySaved>> {
    match version {
        1 => Some(parse::<MemorySavedV1>(data).map(MemorySaved::from)),
        2 => Some(parse(data)),
        _ => None,
    }
}

pub(crate) fn decode_edited(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<MemoryEdited>> {
    (version == 1).then(|| parse(data))
}

pub(crate) fn decode_forgotten(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<MemoryForgotten>> {
    match version {
        1 => Some(parse::<MemoryForgottenV1>(data).map(MemoryForgotten::from)),
        2 => Some(parse(data)),
        _ => None,
    }
}

pub(crate) fn decode_restored(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<MemoryRestored>> {
    (version == 1).then(|| parse(data))
}

pub(crate) fn decode_tidied(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<MemoryTidied>> {
    (version == 1).then(|| parse(data))
}

/// The memory whose own data key seals `event`, when it is one that holds a memory's text
/// (`memory.saved`, `memory.edited`); every other event is sealed under its month's key.
pub fn sealed_under_memory(event: &RawEvent) -> Option<MemoryId> {
    #[derive(Deserialize)]
    struct Named {
        memory: MemoryId,
    }
    if !matches!(event.kind.as_str(), SAVED | EDITED) {
        return None;
    }
    serde_json::from_str::<Named>(event.data.get())
        .ok()
        .map(|named| named.memory)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_memory_saved_to_a_space_reads_as_saved_to_its_project() {
        let data = r#"{"memory":"0192f5c3-0000-7000-8000-000000000001","scope":"space","text":"t","by":{"kind":"person","id":"0192f5c0-11aa-7e21-8c3d-5f9b2e4a6c11"}}"#;
        let raw = RawValue::from_string(data.to_owned()).unwrap();
        let saved = decode_saved(1, &raw).unwrap().unwrap();
        assert_eq!(saved.scope, MemoryScope::Project);
        assert!(
            decode_saved(2, &raw).unwrap().is_err(),
            "version 2 says project"
        );
    }
}
