//! `thread.started`, the first event of every thread, and `thread.compacted`, where the
//! conversation restarts from a summary.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Payload, parse};
use crate::envelope::Seq;
use crate::id::{CallId, ThreadId};
use crate::payload::part::Part;
use crate::payload::principal::{Principal, open_kind};

pub(crate) const TYPE: &str = "thread.started";
pub(crate) const COMPACTED: &str = "thread.compacted";

/// A thread begins. Always at `seq` 1. Records, once, whose work the thread is and which
/// agent does it; every later event acts for them unless it says otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadStarted {
    pub producer: Producer,
    /// Set when this thread continues another thread's events up to and including `seq`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork: Option<ForkPoint>,
    /// Set when a tool call in another thread started this one, as a subagent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<ParentCall>,
    /// Whose work this is. Always written.
    #[serde(rename = "for", default, skip_serializing_if = "Option::is_none")]
    pub for_: Option<Principal>,
    /// The agent profile that runs the thread. Always written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<Principal>,
    /// What started the thread.
    pub origin: Origin,
}

/// What started a thread, and who.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Origin {
    pub kind: OriginKind,
    /// An opaque pointer to what started it, such as a schedule or a chat thread.
    #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    /// Who started it, when that differs from the thread's `for`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Principal>,
}

impl Origin {
    /// Started by a kind of trigger alone, with no pointer and nobody else.
    pub fn of(kind: OriginKind) -> Self {
        Self {
            kind,
            reference: None,
            by: None,
        }
    }
}

open_kind! {
    /// What kind of thing started a thread.
    pub enum OriginKind {
        /// Someone typed a prompt.
        Prompt => "prompt",
        /// Someone spoke.
        Voice => "voice",
        /// A schedule fired.
        Schedule => "schedule",
        /// A webhook arrived.
        Webhook => "webhook",
        /// A message in a chat platform.
        Chat => "chat",
        /// A call in another thread, as a subagent.
        Subagent => "subagent",
        /// A fork of another thread.
        Fork => "fork",
        /// An assistant acting across a person's projects.
        Assistant => "assistant",
    }
}

/// The call in another thread that started a thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParentCall {
    pub thread: ThreadId,
    pub call_id: CallId,
}

/// The software that started the thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Producer {
    pub name: String,
    pub version: String,
}

/// The point in a parent thread that a fork continues from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForkPoint {
    pub thread: ThreadId,
    pub seq: Seq,
}

impl Payload for ThreadStarted {
    const TYPE: &'static str = TYPE;
    const VERSION: u32 = 1;
}

pub(crate) fn decode(version: u32, data: &RawValue) -> Option<serde_json::Result<ThreadStarted>> {
    match version {
        1 => Some(parse(data)),
        _ => None,
    }
}

/// The conversation was summarized to fit the model's context window. From here on, the model
/// sees `parts` as the first message, then what is recorded after this event. Nothing before it
/// is removed from the log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadCompacted {
    /// The last event the summary covers.
    pub through: Seq,
    /// The message the conversation restarts from: the summary and the person's latest
    /// request.
    pub parts: Vec<Part>,
}

impl Payload for ThreadCompacted {
    const TYPE: &'static str = COMPACTED;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_compacted(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<ThreadCompacted>> {
    match version {
        1 => Some(parse(data)),
        _ => None,
    }
}
