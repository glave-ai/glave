//! The Anthropic-compatible surface, `POST /v1/messages`: a Messages request read into the
//! gateway's neutral request, by the same rule as the OpenAI surface: what the neutral request
//! carries is honoured, `metadata` and cache markers are accepted with no effect, and anything
//! else is refused by name. Thinking blocks a client sends back are left out: the gateway
//! cannot tell which model wrote them, and only that model may read them.

mod render;

use glave_event::{Arguments, CallId, Part};
use glave_ports::{Message, ModelRequest, Purpose, ToolCall, ToolChoice, ToolSpec};
use serde_json::{Map, Value, json};

pub(crate) use self::render::{Events, whole};
use crate::errors::Refusal;
use crate::fields::{refuse_the_rest, take_bool, take_count, take_string, text_of};

/// A request as the gateway serves it.
#[derive(Debug)]
pub(crate) struct Parsed {
    pub(crate) model: String,
    pub(crate) request: ModelRequest,
    pub(crate) stream: bool,
}

/// Top-level fields accepted with no effect: who is calling, and where to cache, which the
/// gateway's own providers decide.
const IGNORED: &[&str] = &["metadata", "cache_control"];

/// Reads a Messages request.
pub(crate) fn parse(body: &[u8]) -> Result<Parsed, Refusal> {
    let mut fields: Map<String, Value> = serde_json::from_slice(body)
        .map_err(|error| Refusal::invalid(format!("the request is not a JSON object: {error}")))?;
    let model =
        take_string(&mut fields, "model")?.ok_or_else(|| Refusal::invalid("model is required"))?;
    let max_output_tokens = take_count(&mut fields, "max_tokens")?;
    let system = match fields.remove("system") {
        None => String::new(),
        Some(system) => text_of(&system, "system")?,
    };
    let Some(Value::Array(messages)) = fields.remove("messages") else {
        return Err(Refusal::invalid("messages must be an array"));
    };
    let mut conversation = Vec::with_capacity(messages.len());
    for (n, message) in messages.iter().enumerate() {
        conversation.extend(message_of(message, n)?);
    }
    let tools = match fields.remove("tools") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(tools)) => tools.iter().map(tool).collect::<Result<_, _>>()?,
        Some(_) => return Err(Refusal::invalid("tools must be an array")),
    };
    let tool_choice = match fields.remove("tool_choice") {
        None | Some(Value::Null) => ToolChoice::Auto,
        Some(choice) => tool_choice(&choice)?,
    };
    let stream = take_bool(&mut fields, "stream")?.unwrap_or(false);
    refuse_the_rest(&fields, IGNORED)?;
    Ok(Parsed {
        request: ModelRequest {
            model: model.clone(),
            system,
            messages: conversation,
            tools,
            tool_choice,
            max_output_tokens,
            purpose: Purpose::Main,
            files: Vec::new(),
            effort: None,
        },
        model,
        stream,
    })
}

fn tool_choice(choice: &Value) -> Result<ToolChoice, Refusal> {
    if choice.get("disable_parallel_tool_use") == Some(&Value::Bool(true)) {
        return Err(Refusal::unsupported(
            "tool_choice.disable_parallel_tool_use",
            "models may always call several tools at once",
        ));
    }
    match choice.get("type").and_then(Value::as_str) {
        Some("auto") => Ok(ToolChoice::Auto),
        Some("none") => Ok(ToolChoice::None),
        _ => Err(Refusal::unsupported(
            "tool_choice",
            "only {\"type\": \"auto\"} and {\"type\": \"none\"} are passed on",
        )),
    }
}

/// One message as the gateway's messages: a user message may carry tool results, which come
/// first, as the API requires.
fn message_of(message: &Value, n: usize) -> Result<Vec<Message>, Refusal> {
    let at = format!("messages[{n}]");
    let role = message.get("role").and_then(Value::as_str).unwrap_or("");
    let blocks = match message.get("content") {
        Some(Value::String(text)) => vec![json!({"type": "text", "text": text})],
        Some(Value::Array(blocks)) => blocks.clone(),
        _ => {
            return Err(Refusal::invalid(format!(
                "{at}.content must be text or blocks"
            )));
        }
    };
    match role {
        "user" => user(&blocks, &at),
        "assistant" => assistant(&blocks, &at).map(|message| vec![message]),
        other => Err(Refusal::invalid(format!(
            "{at} has role {other:?}: use user or assistant"
        ))),
    }
}

fn user(blocks: &[Value], at: &str) -> Result<Vec<Message>, Refusal> {
    let mut out = Vec::new();
    let mut texts = Vec::new();
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => texts.push(text_of(&Value::Array(vec![block.clone()]), at)?),
            Some("tool_result") => {
                let id = block
                    .get("tool_use_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let content = block.get("content").cloned().unwrap_or(Value::Null);
                out.push(Message::Tool {
                    call_id: call_id(id, at)?,
                    is_error: block.get("is_error") == Some(&Value::Bool(true)),
                    output: vec![Part::Text {
                        text: text_of(&content, &format!("{at} tool_result content"))?,
                    }],
                });
            }
            kind => return Err(unsupported_block(kind, at)),
        }
    }
    if !texts.is_empty() {
        out.push(Message::User {
            parts: vec![Part::Text {
                text: texts.join("\n"),
            }],
        });
    }
    Ok(out)
}

fn assistant(blocks: &[Value], at: &str) -> Result<Message, Refusal> {
    let mut parts = Vec::new();
    let mut tool_calls = Vec::new();
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => parts.push(Part::Text {
                text: text_of(&Value::Array(vec![block.clone()]), at)?,
            }),
            Some("thinking" | "redacted_thinking") => {}
            Some("tool_use") => {
                let id = block.get("id").and_then(Value::as_str).unwrap_or_default();
                let name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| Refusal::invalid(format!("{at}: a tool_use has no name")))?;
                let arguments = match block.get("input") {
                    Some(Value::Object(input)) => Arguments::Json(input.clone()),
                    Some(other) => Arguments::Malformed(other.to_string()),
                    None => Arguments::Json(Map::new()),
                };
                tool_calls.push(ToolCall {
                    call_id: call_id(id, at)?,
                    tool: name.to_owned(),
                    arguments,
                });
            }
            kind => return Err(unsupported_block(kind, at)),
        }
    }
    Ok(Message::Assistant { parts, tool_calls })
}

fn unsupported_block(kind: Option<&str>, at: &str) -> Refusal {
    Refusal::unsupported(
        &format!("{at} blocks of type {}", kind.unwrap_or("(none)")),
        "only text, tool_use and tool_result are passed on yet",
    )
}

fn call_id(id: &str, at: &str) -> Result<CallId, Refusal> {
    CallId::new(id).map_err(|error| Refusal::invalid(format!("{at}: {error}")))
}

fn tool(tool: &Value) -> Result<ToolSpec, Refusal> {
    if let Some(kind) = tool.get("type").and_then(Value::as_str)
        && kind != "custom"
    {
        return Err(Refusal::unsupported(
            &format!("tools of type {kind}"),
            "the gateway's own tools are served at /mcp",
        ));
    }
    let name = tool
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| Refusal::invalid("a tool has no name"))?;
    Ok(ToolSpec {
        name: name.to_owned(),
        description: tool
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        parameters: tool
            .get("input_schema")
            .cloned()
            .unwrap_or_else(|| json!({"type": "object", "properties": {}})),
    })
}

#[cfg(test)]
mod tests;
