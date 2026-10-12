//! The model provider port: send a conversation, stream the answer live, get the finished
//! answer back. Providers translate their own wire formats at this edge.

use std::future::Future;

use glave_event::{Arguments, CallId, Cost, ModelRef, Part, StopReason, ThreadId, Usage};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::failure::Failure;

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
    /// Whether the model may call the tools on offer. They stay listed either way, since some
    /// providers refuse a conversation that has tool calls but no tools.
    #[serde(default, skip_serializing_if = "ToolChoice::is_auto")]
    pub tool_choice: ToolChoice,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    /// What the call is for, so a gateway can choose the model for it. Providers ignore it.
    #[serde(default, skip_serializing_if = "Purpose::is_main")]
    pub purpose: Purpose,
    /// The thread the call is made for, then the threads that started it, innermost first:
    /// whose budgets it counts against. Providers ignore it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub threads: Vec<ThreadId>,
}

/// What a model call is for. A stable public vocabulary: gateway routes are named by it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    /// The agent's own turn.
    #[default]
    Main,
    /// Deciding whether a flagged action the person asked for may run (auto mode).
    Judge,
    /// A compaction's handoff summary.
    Summary,
    /// A subagent's turn.
    Subagent,
    /// A thread's short title, after its first exchange.
    Title,
}

impl Purpose {
    /// The purpose's stable id: `main`, `judge`, `summary`, `subagent` or `title`.
    pub fn id(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Judge => "judge",
            Self::Summary => "summary",
            Self::Subagent => "subagent",
            Self::Title => "title",
        }
    }

    pub fn is_main(&self) -> bool {
        *self == Self::Main
    }
}

/// Whether a response may call tools.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolChoice {
    /// The model decides.
    #[default]
    Auto,
    /// Text only: no tool calls.
    None,
}

impl ToolChoice {
    pub fn is_auto(&self) -> bool {
        *self == Self::Auto
    }
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
    /// The cost the provider itself reported, when it reports one (OpenRouter does), or a
    /// gateway computed from a configured price.
    pub cost: Option<Cost>,
    /// How a gateway served the call; `None` when no gateway was in between.
    pub served: Option<Served>,
}

/// How a gateway served a call: the route it was sent by and the target that answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Served {
    /// The route's id: the call's purpose, such as `main` or `judge`.
    pub route: String,
    /// The name of the target that answered.
    pub target: String,
    /// True when opaque reasoning was left out of the request because the target that
    /// answered could not accept it.
    pub reasoning_stripped: bool,
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
    /// Credentials for the provider could not be had: a token file missing, or an identity
    /// provider that refused. Never holds a secret.
    #[error("could not get credentials for the model provider: {0}")]
    Credentials(String),
    /// The provider could not be reached, or the connection broke.
    #[error("could not reach the model provider: {0}")]
    Transport(String),
    /// The provider's response did not follow its own protocol.
    #[error("the model provider sent a malformed response: {0}")]
    Protocol(String),
    /// The response ended in an error before any text, reasoning or tool call: nothing was
    /// shown or run, so asking again is safe.
    #[error("the model provider ended the response in an error before any output: {0}")]
    NoOutput(String),
    /// The provider sent nothing for this long, before or during its answer.
    #[error("the model provider sent nothing for {seconds} s")]
    Timeout { seconds: u64 },
    /// A gateway sent nothing: no target could serve the request. `failure` says why, in the
    /// terms a provider's own refusal would have used.
    #[error("no model could serve the request: {reason}")]
    Unserved { failure: Failure, reason: String },
    /// A gateway sent nothing: the call could exceed this budget.
    #[error("the {budget} budget would be exceeded: {detail}")]
    OverBudget { budget: String, detail: String },
}

/// An event stream that cannot be read is a response that does not follow its protocol.
impl From<crate::SseError> for ProviderError {
    fn from(error: crate::SseError) -> Self {
        Self::Protocol(error.to_string())
    }
}
