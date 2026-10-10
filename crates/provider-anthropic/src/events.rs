//! The events of a streamed Messages response, as the API sends them. Anthropic adds event,
//! block and delta types over time, so unknown ones are read as `Other` and skipped, and
//! unknown fields are ignored.

use serde::Deserialize;
use serde_json::{Map, Value};

/// One event's data.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum StreamEvent {
    MessageStart {
        message: StartedMessage,
    },
    ContentBlockStart {
        index: u32,
        content_block: StartedBlock,
    },
    ContentBlockDelta {
        index: u32,
        delta: BlockDelta,
    },
    ContentBlockStop,
    MessageDelta {
        delta: MessageChange,
        #[serde(default)]
        usage: Option<Map<String, Value>>,
    },
    MessageStop,
    Ping,
    Error {
        error: WireError,
    },
    #[serde(other)]
    Other,
}

/// The message as `message_start` announces it, before any content.
#[derive(Debug, Deserialize)]
pub(crate) struct StartedMessage {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// Kept as sent, so the provider's exact numbers survive.
    #[serde(default)]
    pub usage: Option<Map<String, Value>>,
}

/// A content block as it starts.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum StartedBlock {
    Text {
        #[serde(default)]
        text: String,
    },
    Thinking {
        #[serde(default)]
        thinking: String,
        #[serde(default)]
        signature: String,
    },
    RedactedThinking {
        data: String,
    },
    ToolUse {
        id: String,
        name: String,
        #[serde(default)]
        input: Option<Value>,
    },
    #[serde(other)]
    Other,
}

/// A piece of a content block.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum BlockDelta {
    TextDelta {
        text: String,
    },
    ThinkingDelta {
        thinking: String,
    },
    SignatureDelta {
        signature: String,
    },
    InputJsonDelta {
        partial_json: String,
    },
    #[serde(other)]
    Other,
}

/// What `message_delta` changes: why the model stopped.
#[derive(Debug, Deserialize)]
pub(crate) struct MessageChange {
    #[serde(default)]
    pub stop_reason: Option<String>,
}

/// An error the API reports, in a body or mid-stream.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct WireError {
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub message: String,
}
