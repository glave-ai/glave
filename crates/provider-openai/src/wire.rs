//! The chat completions wire format: glave's neutral request in, OpenAI's JSON out, and the
//! shape of each streamed chunk coming back.

use glave_event::{Arguments, Part};
use glave_ports::{Message, ModelRequest, ToolChoice, ToolSpec};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

#[derive(Debug, Serialize)]
pub(crate) struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<WireMessage<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<WireTool<'a>>,
    stream: bool,
    stream_options: StreamOptions,
    /// `none` when the model must answer in text; the tools stay listed.
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    /// OpenRouter's automatic prompt caching: it moves the cache point forward as the
    /// conversation grows.
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<CacheControl>,
    /// For OpenRouter: context compression off. On by default for models with a window of 8k
    /// tokens or less, it silently cuts the middle of a prompt that is too long; refused
    /// instead, glave compacts in the open, recorded in the log.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    plugins: Vec<Plugin>,
}

#[derive(Debug, Serialize)]
struct Plugin {
    id: &'static str,
    enabled: bool,
}

#[derive(Debug, Serialize)]
struct StreamOptions {
    include_usage: bool,
}

#[derive(Debug, Serialize)]
struct CacheControl {
    #[serde(rename = "type")]
    kind: &'static str,
}

/// Whether to ask for automatic prompt caching. Anthropic caches only what a request marks,
/// unlike OpenAI's automatic caching, so without it every call repays the whole prefix.
/// OpenRouter's top-level marker covers Anthropic models; other servers get the plain request.
pub(crate) fn caches_automatically(provider: &str, model: &str) -> bool {
    provider == "openrouter" && model.starts_with("anthropic/")
}

#[derive(Debug, Serialize)]
#[serde(tag = "role", rename_all = "snake_case")]
enum WireMessage<'a> {
    System {
        content: &'a str,
    },
    User {
        content: String,
    },
    Assistant {
        content: Option<String>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<WireToolCall<'a>>,
    },
    Tool {
        tool_call_id: &'a str,
        content: String,
    },
}

#[derive(Debug, Serialize)]
struct WireToolCall<'a> {
    id: &'a str,
    #[serde(rename = "type")]
    kind: &'static str,
    function: WireFunctionCall<'a>,
}

#[derive(Debug, Serialize)]
struct WireFunctionCall<'a> {
    name: &'a str,
    arguments: String,
}

#[derive(Debug, Serialize)]
struct WireTool<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    function: &'a ToolSpec,
}

/// Builds the request body for a streaming chat completion to `provider`.
pub(crate) fn chat_request<'a>(request: &'a ModelRequest, provider: &str) -> ChatRequest<'a> {
    let cache = caches_automatically(provider, &request.model);
    let mut messages = Vec::with_capacity(request.messages.len().saturating_add(1));
    if !request.system.is_empty() {
        messages.push(WireMessage::System {
            content: &request.system,
        });
    }
    for message in request.messages.iter().map(wire_message) {
        // Consecutive user messages become one. Strict chat templates (Gemma's among them)
        // refuse two in a row, and a conversation has them after a cancelled turn or a
        // compaction.
        match (messages.last_mut(), message) {
            (Some(WireMessage::User { content }), WireMessage::User { content: next }) => {
                content.push_str("\n\n");
                content.push_str(&next);
            }
            (_, message) => messages.push(message),
        }
    }
    ChatRequest {
        model: &request.model,
        messages,
        tools: request
            .tools
            .iter()
            .map(|function| WireTool {
                kind: "function",
                function,
            })
            .collect(),
        stream: true,
        stream_options: StreamOptions {
            include_usage: true,
        },
        tool_choice: (request.tool_choice == ToolChoice::None && !request.tools.is_empty())
            .then_some("none"),
        max_tokens: request.max_output_tokens,
        cache_control: cache.then_some(CacheControl { kind: "ephemeral" }),
        plugins: if provider == "openrouter" {
            vec![Plugin {
                id: "context-compression",
                enabled: false,
            }]
        } else {
            Vec::new()
        },
    }
}

fn wire_message(message: &Message) -> WireMessage<'_> {
    match message {
        Message::User { parts } => WireMessage::User {
            content: text_of(parts),
        },
        Message::Assistant { parts, tool_calls } => {
            let text = text_of(parts);
            WireMessage::Assistant {
                content: (!text.is_empty()).then_some(text),
                tool_calls: tool_calls
                    .iter()
                    .map(|call| WireToolCall {
                        id: call.call_id.as_str(),
                        kind: "function",
                        function: WireFunctionCall {
                            name: &call.tool,
                            arguments: arguments_text(&call.arguments),
                        },
                    })
                    .collect(),
            }
        }
        Message::Tool {
            call_id, output, ..
        } => WireMessage::Tool {
            tool_call_id: call_id.as_str(),
            content: text_of(output),
        },
    }
}

/// The text parts of a message, joined. Reasoning is not sent back through this API, and
/// files are named rather than inlined until attachments are supported.
fn text_of(parts: &[Part]) -> String {
    let mut text = String::new();
    for part in parts {
        let piece = match part {
            Part::Text { text } => text.as_str(),
            Part::File { name, .. } => name.as_deref().unwrap_or("[file]"),
            Part::Reasoning { .. } => continue,
        };
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(piece);
    }
    text
}

fn arguments_text(arguments: &Arguments) -> String {
    match arguments {
        Arguments::Json(map) => serde_json::to_string(map).unwrap_or_else(|_| "{}".to_owned()),
        Arguments::Malformed(raw) => raw.clone(),
    }
}

/// One streamed chunk. Providers add fields freely, so unknown fields are ignored.
#[derive(Debug, Deserialize)]
pub(crate) struct Chunk {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub choices: Vec<Choice>,
    /// Kept raw so the provider's exact numbers (and a reported cost) survive.
    #[serde(default)]
    pub usage: Option<Box<RawValue>>,
    #[serde(default)]
    pub error: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Choice {
    #[serde(default)]
    pub delta: Option<ChoiceDelta>,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ChoiceDelta {
    #[serde(default)]
    pub content: Option<String>,
    /// Reasoning text, as llama.cpp and vLLM name it.
    #[serde(default)]
    pub reasoning_content: Option<String>,
    /// Reasoning text, as OpenRouter and Ollama name it.
    #[serde(default)]
    pub reasoning: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCallDelta>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ToolCallDelta {
    #[serde(default)]
    pub index: Option<u32>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub function: Option<FunctionDelta>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FunctionDelta {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

/// The usage object: token counts, plus the cost OpenRouter reports, kept as its exact
/// decimal text.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct WireUsage {
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
    #[serde(default)]
    pub prompt_tokens_details: Option<PromptDetails>,
    #[serde(default)]
    pub completion_tokens_details: Option<CompletionDetails>,
    #[serde(default)]
    pub cost: Option<Box<RawValue>>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct PromptDetails {
    #[serde(default)]
    pub cached_tokens: Option<u64>,
    #[serde(default)]
    pub cache_write_tokens: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct CompletionDetails {
    #[serde(default)]
    pub reasoning_tokens: Option<u64>,
}

#[cfg(test)]
mod tests;
