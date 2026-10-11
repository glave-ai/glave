//! `memory.saved`, `memory.edited`, `memory.forgotten` and `memory.restored`: what glave
//! remembers for later threads (`specs/memory.md`). They are kept in the space's memory ledger,
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

/// Where a memory belongs: to the person, or to the space it was saved in.
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
    /// About the person: read in their threads, in spaces that allow it.
    Personal,
    /// About the space's work: read by every member and agent working in it.
    Space,
}

impl MemoryScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Personal => "personal",
            Self::Space => "space",
        }
    }

    /// The scope named `name`: `personal` or `space`.
    pub fn named(name: &str) -> Option<Self> {
        match name.trim() {
            "personal" => Some(Self::Personal),
            "space" => Some(Self::Space),
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
    /// Who forgot it.
    pub by: Principal,
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

impl Payload for MemorySaved {
    const TYPE: &'static str = SAVED;
    const VERSION: u32 = 1;
}

impl Payload for MemoryEdited {
    const TYPE: &'static str = EDITED;
    const VERSION: u32 = 1;
}

impl Payload for MemoryForgotten {
    const TYPE: &'static str = FORGOTTEN;
    const VERSION: u32 = 1;
}

impl Payload for MemoryRestored {
    const TYPE: &'static str = RESTORED;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_saved(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<MemorySaved>> {
    (version == 1).then(|| parse(data))
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
    (version == 1).then(|| parse(data))
}

pub(crate) fn decode_restored(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<MemoryRestored>> {
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
