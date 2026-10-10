//! `message.added`: a finished message from the person or the assistant.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use super::part::Part;
use super::principal::Principal;
use crate::codec::{Payload, parse};

pub(crate) const TYPE: &str = "message.added";

/// A finished message. Streaming fragments are never events; this is the whole message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageAdded {
    pub role: Role,
    pub parts: Vec<Part>,
    /// Who wrote it. Absent: the thread's `for` for a user message, its `agent` for an
    /// assistant message. Since version 2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<Principal>,
}

/// Version 1, before a message could name its author.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MessageAddedV1 {
    role: Role,
    parts: Vec<Part>,
}

impl From<MessageAddedV1> for MessageAdded {
    fn from(old: MessageAddedV1) -> Self {
        Self {
            role: old.role,
            parts: old.parts,
            author: None,
        }
    }
}

/// Who a message is from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

impl Payload for MessageAdded {
    const TYPE: &'static str = TYPE;
    const VERSION: u32 = 2;
}

pub(crate) fn decode(version: u32, data: &RawValue) -> Option<serde_json::Result<MessageAdded>> {
    match version {
        1 => Some(parse::<MessageAddedV1>(data).map(MessageAdded::from)),
        2 => Some(parse(data)),
        _ => None,
    }
}
