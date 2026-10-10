//! The Messages request: glave's neutral request in, Anthropic's JSON out.

use std::borrow::Cow;

use glave_event::{Arguments, Part};
use glave_ports::{Message, ModelRequest, ToolChoice, ToolSpec};
use serde::Serialize;
use serde_json::{Map, Value};

/// The output limit sent when a request sets none. The API requires one, and this fits every
/// current model's own limit.
pub(crate) const DEFAULT_MAX_TOKENS: u32 = 32_000;

/// The kind of [`glave_event::Opaque`] that holds a thinking block's signature.
pub(crate) const SIGNATURE: &str = "signature";

/// The kind of [`glave_event::Opaque`] that holds a redacted thinking block's data.
pub(crate) const REDACTED_THINKING: &str = "redacted_thinking";

/// A streaming Messages request. Neither `thinking` nor `temperature` is sent: current models
/// choose their own thinking, and several refuse sampling settings.
#[derive(Debug, Serialize)]
pub(crate) struct MessagesRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<&'a str>,
    messages: Vec<WireMessage<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<WireTool<'a>>,
    /// `none` when the model must answer in text; the tools stay listed.
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<Kind>,
    /// Automatic prompt caching: the cache point moves forward as the conversation grows, so
    /// each call pays full price only for what is new.
    cache_control: Kind,
    stream: bool,
}

/// An object that is only its type, such as `{"type": "ephemeral"}`.
#[derive(Debug, Serialize)]
struct Kind {
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Role {
    User,
    Assistant,
}

#[derive(Debug, Serialize)]
struct WireMessage<'a> {
    role: Role,
    content: Vec<Block<'a>>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Block<'a> {
    Text {
        text: &'a str,
    },
    Thinking {
        thinking: &'a str,
        signature: &'a str,
    },
    RedactedThinking {
        data: &'a str,
    },
    ToolUse {
        id: &'a str,
        name: &'a str,
        input: Cow<'a, Map<String, Value>>,
    },
    ToolResult {
        tool_use_id: &'a str,
        #[serde(skip_serializing_if = "String::is_empty")]
        content: String,
        is_error: bool,
    },
}

#[derive(Debug, Serialize)]
struct WireTool<'a> {
    name: &'a str,
    description: &'a str,
    input_schema: &'a Value,
}

/// Builds the body of a streaming Messages request. `provider` is the name this provider
/// records with its responses: thinking it produced for the same model is sent back.
pub(crate) fn messages_request<'a>(
    request: &'a ModelRequest,
    provider: &str,
) -> MessagesRequest<'a> {
    let mut messages: Vec<WireMessage<'a>> = Vec::with_capacity(request.messages.len());
    for message in &request.messages {
        let (role, content) = blocks(message, provider, &request.model);
        if content.is_empty() {
            continue;
        }
        // Roles must alternate, so consecutive messages of one role become one: a tool
        // result joins the others of its turn, and a user message after a cancelled turn or a
        // compaction joins the one before it.
        match messages.last_mut() {
            Some(last) if last.role == role => last.content.extend(content),
            _ => messages.push(WireMessage { role, content }),
        }
    }
    MessagesRequest {
        model: &request.model,
        max_tokens: request.max_output_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
        system: (!request.system.is_empty()).then_some(request.system.as_str()),
        messages,
        tools: request.tools.iter().map(wire_tool).collect(),
        tool_choice: (request.tool_choice == ToolChoice::None && !request.tools.is_empty())
            .then_some(Kind { kind: "none" }),
        cache_control: Kind { kind: "ephemeral" },
        stream: true,
    }
}

fn wire_tool(tool: &ToolSpec) -> WireTool<'_> {
    WireTool {
        name: &tool.name,
        description: &tool.description,
        input_schema: &tool.parameters,
    }
}

/// One message as a role and its content blocks.
fn blocks<'a>(message: &'a Message, provider: &str, model: &str) -> (Role, Vec<Block<'a>>) {
    match message {
        Message::User { parts } => (Role::User, texts(parts).collect()),
        Message::Assistant { parts, tool_calls } => {
            // Thinking comes first, in its original order, then the text, then the calls.
            let mut content: Vec<Block<'a>> = parts
                .iter()
                .filter_map(|part| replayed(part, provider, model))
                .collect();
            content.extend(texts(parts));
            content.extend(tool_calls.iter().map(|call| Block::ToolUse {
                id: call.call_id.as_str(),
                name: &call.tool,
                input: match &call.arguments {
                    Arguments::Json(map) => Cow::Borrowed(map),
                    // The API takes only an object; the model sees its mistake in the result.
                    Arguments::Malformed(_) => Cow::Owned(Map::new()),
                },
            }));
            (Role::Assistant, content)
        }
        Message::Tool {
            call_id,
            is_error,
            output,
        } => (
            Role::User,
            vec![Block::ToolResult {
                tool_use_id: call_id.as_str(),
                content: text_of(output),
                is_error: *is_error,
            }],
        ),
    }
}

/// The text blocks of a message: its text, and files named rather than inlined until
/// attachments are supported. Empty text is left out, since the API refuses it.
fn texts(parts: &[Part]) -> impl Iterator<Item = Block<'_>> {
    parts.iter().filter_map(|part| {
        let text = match part {
            Part::Text { text } => text.as_str(),
            Part::File { name, .. } => name.as_deref().unwrap_or("[file]"),
            Part::Reasoning { .. } => return None,
        };
        (!text.is_empty()).then_some(Block::Text { text })
    })
}

/// A thinking block to send back unchanged, when this provider produced it for this model.
/// Thinking from anywhere else is left out: its signature would not verify.
fn replayed<'a>(part: &'a Part, provider: &str, model: &str) -> Option<Block<'a>> {
    let Part::Reasoning {
        text,
        opaque: Some(opaque),
    } = part
    else {
        return None;
    };
    if opaque.provider != provider || opaque.model != model {
        return None;
    }
    match opaque.kind.as_str() {
        SIGNATURE => Some(Block::Thinking {
            thinking: text.as_deref().unwrap_or(""),
            signature: &opaque.data,
        }),
        REDACTED_THINKING => Some(Block::RedactedThinking { data: &opaque.data }),
        _ => None,
    }
}

/// The text parts of a tool result, joined.
fn text_of(parts: &[Part]) -> String {
    let mut text = String::new();
    for block in texts(parts) {
        if let Block::Text { text: piece } = block {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(piece);
        }
    }
    text
}

#[cfg(test)]
mod tests;
