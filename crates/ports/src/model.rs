//! The model provider port: send a conversation, stream the answer live, get the finished
//! answer back. Providers translate their own wire formats at this edge.

use std::future::Future;

use glave_event::{Arguments, CallId, ModelRef, Part, StopReason, Usage};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// A model that can answer a conversation.
pub trait ModelProvider: Send + Sync {
    /// Sends one request and streams the answer. Live fragments go to `on_delta` as they
    /// arrive and are never stored; the finished answer is returned.
    fn complete(
        &self,
        request: &ModelRequest,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> impl Future<Output = Result<Completion, ProviderError>> + Send;
}

/// One request: the system prompt, the conversation so far, and the tools on offer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelRequest {
    pub model: String,
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
}

/// One message of the conversation, in provider-neutral form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum Message {
    User {
        parts: Vec<Part>,
    },
    Assistant {
        parts: Vec<Part>,
        tool_calls: Vec<ToolCall>,
    },
    Tool {
        call_id: CallId,
        is_error: bool,
        output: Vec<Part>,
    },
}

/// A tool call already in the conversation, under its glave call id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub call_id: CallId,
    pub tool: String,
    pub arguments: Arguments,
}

/// A tool offered to the model: its name, what it does, and a JSON Schema for its arguments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// A live fragment of an answer. Shown as it arrives, never stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delta {
    Text(String),
    Reasoning(String),
}

/// A finished answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Completion {
    pub model: ModelRef,
    pub response_id: Option<String>,
    /// Text and reasoning, in the order the model produced them.
    pub parts: Vec<Part>,
    pub tool_calls: Vec<RequestedCall>,
    pub stop: StopReason,
    pub provider_stop: Option<String>,
    pub usage: Usage,
}

/// A tool call the model just asked for. The harness gives it a glave call id when it
/// records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestedCall {
    pub provider_call_id: Option<String>,
    pub tool: String,
    pub arguments: Arguments,
}

/// Why a model request failed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProviderError {
    /// The provider answered with an error status.
    #[error("the model provider returned HTTP {status}: {body}")]
    Http { status: u16, body: String },
    /// The provider could not be reached, or the connection broke.
    #[error("could not reach the model provider: {0}")]
    Transport(String),
    /// The provider's response did not follow its own protocol.
    #[error("the model provider sent a malformed response: {0}")]
    Protocol(String),
}
