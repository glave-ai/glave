//! `tool.call.requested` and `tool.call.completed`, linked by their `call_id`.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use serde_json::{Map, Value};

use super::part::Part;
use crate::codec::{Payload, parse};
use crate::id::CallId;

pub(crate) const REQUESTED: &str = "tool.call.requested";
pub(crate) const COMPLETED: &str = "tool.call.completed";

/// The model asks for a tool to run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCallRequested {
    pub call_id: CallId,
    /// The provider's own id for the call, when it gave one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_call_id: Option<String>,
    pub tool: String,
    pub arguments: Arguments,
}

/// A tool call's arguments: a JSON object, or the provider's raw text when it was not valid
/// JSON (some providers send arguments as text that may not parse).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Arguments {
    Json(Map<String, Value>),
    Malformed(String),
}

impl Payload for ToolCallRequested {
    const TYPE: &'static str = REQUESTED;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_requested(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<ToolCallRequested>> {
    match version {
        1 => Some(parse(data)),
        _ => None,
    }
}

/// A tool call finished. `is_error` marks a failure the model should see; `structured` holds
/// the machine-readable result kept apart from the parts the model saw.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCallCompleted {
    pub call_id: CallId,
    pub is_error: bool,
    pub output: Vec<Part>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured: Option<Value>,
}

impl Payload for ToolCallCompleted {
    const TYPE: &'static str = COMPLETED;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_completed(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<ToolCallCompleted>> {
    match version {
        1 => Some(parse(data)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_say_whether_they_parsed() {
        let json: Arguments = serde_json::from_str(r#"{"json":{"command":"ls"}}"#).unwrap();
        assert!(matches!(json, Arguments::Json(ref map) if map.contains_key("command")));
        let bad: Arguments = serde_json::from_str(r#"{"malformed":"{\"command\": \"ls"}"#).unwrap();
        assert_eq!(bad, Arguments::Malformed(r#"{"command": "ls"#.into()));
        assert!(serde_json::from_str::<Arguments>(r#"{"json":"not an object"}"#).is_err());
    }
}
