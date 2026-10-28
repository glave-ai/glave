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
    /// assistant message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<Principal>,
    /// What the model's reasoning in it came to, for the line that tells it: absent when it has
    /// no readable reasoning, or was recorded before glave kept this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thought: Option<Thought>,
}

/// The model's reasoning in a line: its headline, and how long the model reasoned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Thought {
    /// Its last bold title when it has any, else its last whole sentence, as the model wrote it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headline: Option<String>,
    /// How long the model reasoned, in milliseconds, when its reasoning streamed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ms: Option<u64>,
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
    const VERSION: u32 = 1;
}

pub(crate) fn decode(version: u32, data: &RawValue) -> Option<serde_json::Result<MessageAdded>> {
    match version {
        1 => Some(parse(data)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_recorded_before_its_thought_was_kept_reads_without_one() {
        let old = r#"{"role":"assistant","parts":[{"type":"reasoning","text":"Done."}]}"#;
        let read: MessageAdded = serde_json::from_str(old).unwrap();
        assert_eq!(read.thought, None);
        assert_eq!(serde_json::to_string(&read).unwrap(), old);
    }

    #[test]
    fn a_thought_round_trips() {
        let json = r#"{"role":"assistant","parts":[],"thought":{"headline":"Done.","ms":4250}}"#;
        let read: MessageAdded = serde_json::from_str(json).unwrap();
        let thought = Thought {
            headline: Some("Done.".to_owned()),
            ms: Some(4_250),
        };
        assert_eq!(read.thought, Some(thought));
        assert_eq!(serde_json::to_string(&read).unwrap(), json);
    }
}
