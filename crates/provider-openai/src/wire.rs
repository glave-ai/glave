//! The chat completions wire format: glave's neutral request in, OpenAI's JSON out, and the
//! shape of each streamed chunk coming back.

use glave_event::{Arguments, Part};
use glave_ports::{InlineFile, Message, ModelRequest, ToolChoice, ToolSpec};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_json::value::RawValue;

use crate::reasoning::sent_back;

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
    /// How much the model reasons, as OpenRouter takes it: `{"effort": "high"}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<Effort<'a>>,
    /// How much the model reasons, as OpenAI and the servers that copy it take it.
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'a str>,
    /// For OpenRouter, on every request: no provider that keeps or trains on prompts, and
    /// none that would drop a parameter the request uses, such as its tools.
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<Preferences>,
}

#[derive(Debug, Serialize)]
struct Effort<'a> {
    effort: &'a str,
}

#[derive(Debug, Serialize)]
struct Preferences {
    data_collection: &'static str,
    require_parameters: bool,
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
        content: Content,
    },
    Assistant {
        content: Option<String>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<WireToolCall<'a>>,
        /// The model's own reasoning, sent back in the field it came in.
        #[serde(skip_serializing_if = "Option::is_none")]
        reasoning_content: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reasoning: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reasoning_details: Option<&'a RawValue>,
    },
    Tool {
        tool_call_id: &'a str,
        content: String,
    },
}

/// A person's message: its text alone, or its parts in order when it holds an image or a PDF.
#[derive(Debug, Serialize)]
#[serde(untagged)]
enum Content {
    Text(String),
    Parts(Vec<ContentPart>),
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentPart {
    Text {
        text: String,
    },
    ImageUrl {
        image_url: ImageUrl,
    },
    /// A PDF: the API takes no other document inline.
    File {
        file: FileData,
    },
}

#[derive(Debug, Serialize)]
struct ImageUrl {
    /// A `data:` URL holding the image.
    url: String,
}

#[derive(Debug, Serialize)]
struct FileData {
    filename: String,
    /// A `data:` URL holding the file.
    file_data: String,
}

impl ContentPart {
    /// `file`, which `part` names, sent inline: an image as one, a PDF as a file.
    fn inline(file: &InlineFile, part: &Part) -> Self {
        let url = format!("data:{};base64,{}", file.media_type, file.data);
        if !file.is_pdf() {
            return Self::ImageUrl {
                image_url: ImageUrl { url },
            };
        }
        let name = match part {
            Part::File {
                name: Some(name), ..
            } => name.clone(),
            _ => "document.pdf".to_owned(),
        };
        Self::File {
            file: FileData {
                filename: name,
                file_data: url,
            },
        }
    }
}

impl Content {
    /// `parts` as a person's message: images and PDFs sent inline where `request` holds their
    /// content, the rest as text.
    fn of(request: &ModelRequest, parts: &[Part]) -> Self {
        if !parts.iter().any(|part| request.inline(part).is_some()) {
            return Self::Text(text_of(parts));
        }
        let mut out: Vec<ContentPart> = Vec::new();
        for part in parts {
            if let Some(file) = request.inline(part) {
                out.push(ContentPart::inline(file, part));
                continue;
            }
            let text = text_of(std::slice::from_ref(part));
            if text.is_empty() {
                continue;
            }
            match out.last_mut() {
                Some(ContentPart::Text { text: before }) => {
                    before.push('\n');
                    before.push_str(&text);
                }
                _ => out.push(ContentPart::Text { text }),
            }
        }
        Self::Parts(out)
    }

    /// `next` after this, as one message: text joined by a blank line, else parts in order.
    fn append(&mut self, next: Self) {
        match (&mut *self, next) {
            (Self::Text(text), Self::Text(next)) => {
                text.push_str("\n\n");
                text.push_str(&next);
            }
            (_, next) => {
                let mut parts = std::mem::replace(self, Self::Parts(Vec::new())).into_parts();
                parts.extend(next.into_parts());
                *self = Self::Parts(parts);
            }
        }
    }

    fn into_parts(self) -> Vec<ContentPart> {
        match self {
            Self::Text(text) if text.is_empty() => Vec::new(),
            Self::Text(text) => vec![ContentPart::Text { text }],
            Self::Parts(parts) => parts,
        }
    }
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
    let openrouter = provider == "openrouter";
    let effort = request.effort.as_deref();
    let mut messages = Vec::with_capacity(request.messages.len().saturating_add(1));
    if !request.system.is_empty() {
        messages.push(WireMessage::System {
            content: &request.system,
        });
    }
    // The images and PDFs tool results hand the model go after them, as the person's, each
    // after a line naming its result: a tool's message takes text alone. OpenCode sends them
    // so too.
    let mut handed: Vec<ContentPart> = Vec::new();
    for message in &request.messages {
        if let Message::Tool {
            call_id, output, ..
        } = message
        {
            for part in output {
                if let Some(file) = request.inline(part) {
                    let name = match part {
                        Part::File {
                            name: Some(name), ..
                        } => name.as_str(),
                        _ => "a file",
                    };
                    let call = call_id.as_str();
                    let text = format!("{name}, from tool result {call}:");
                    handed.push(ContentPart::Text { text });
                    handed.push(ContentPart::inline(file, part));
                }
            }
        } else {
            hand(&mut messages, &mut handed);
        }
        push(&mut messages, wire_message(request, provider, message));
    }
    hand(&mut messages, &mut handed);
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
        plugins: if openrouter {
            vec![Plugin {
                id: "context-compression",
                enabled: false,
            }]
        } else {
            Vec::new()
        },
        reasoning: effort
            .filter(|_| openrouter)
            .map(|effort| Effort { effort }),
        reasoning_effort: effort.filter(|_| !openrouter),
        provider: openrouter.then_some(Preferences {
            data_collection: "deny",
            require_parameters: true,
        }),
    }
}

/// Adds `message` after `messages`. Consecutive user messages become one: strict chat
/// templates (Gemma's among them) refuse two in a row, and a conversation has them after a
/// cancelled turn or a compaction.
fn push<'a>(messages: &mut Vec<WireMessage<'a>>, message: WireMessage<'a>) {
    match (messages.last_mut(), message) {
        (Some(WireMessage::User { content }), WireMessage::User { content: next }) => {
            content.append(next);
        }
        (_, message) => messages.push(message),
    }
}

/// Sends what tool results `handed` the model, if anything, as a message of the person's.
fn hand(messages: &mut Vec<WireMessage<'_>>, handed: &mut Vec<ContentPart>) {
    if !handed.is_empty() {
        let content = Content::Parts(std::mem::take(handed));
        push(messages, WireMessage::User { content });
    }
}

fn wire_message<'a>(
    request: &ModelRequest,
    provider: &str,
    message: &'a Message,
) -> WireMessage<'a> {
    match message {
        Message::User { parts } => WireMessage::User {
            content: Content::of(request, parts),
        },
        Message::Assistant { parts, tool_calls } => {
            let text = text_of(parts);
            let reasoning = sent_back(parts, provider, &request.model);
            WireMessage::Assistant {
                reasoning_content: reasoning.reasoning_content,
                reasoning: reasoning.reasoning,
                reasoning_details: reasoning.reasoning_details,
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

/// The text parts of a message, joined. Reasoning goes back in fields of its own, and a file
/// not sent inline is named.
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
    /// The provider a service that routes calls passed this one on to (OpenRouter's).
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub choices: Vec<Choice>,
    /// Kept raw so the provider's exact numbers (and a reported cost) survive.
    #[serde(default)]
    pub usage: Option<Box<RawValue>>,
    #[serde(default)]
    pub error: Option<Value>,
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
    /// OpenRouter's structured reasoning, signed or encrypted entries among them.
    #[serde(default)]
    pub reasoning_details: Option<Vec<Value>>,
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
