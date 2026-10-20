//! The OpenAI-compatible surface, `POST /v1/chat/completions`: a Chat Completions request read
//! into the gateway's neutral request. What the neutral request carries is honoured; fields
//! that only describe the caller (`user`, `metadata`, `store`) are accepted and have no effect;
//! anything else is refused by name rather than silently dropped, so a caller never gets an
//! answer to a question it did not ask.

mod render;

use glave_event::{Arguments, CallId, Part};
use glave_ports::{Message, ModelRequest, ToolCall, ToolChoice, ToolSpec};
use serde_json::{Map, Value, json};

pub(crate) use self::render::{Chunks, whole};
use crate::errors::Refusal;
use crate::fields::{refuse_the_rest, take_bool, take_count, take_string, text_of};

/// A request as the gateway serves it.
#[derive(Debug)]
pub(crate) struct Parsed {
    /// The model name asked for, which the gateway's routes resolve.
    pub(crate) model: String,
    pub(crate) request: ModelRequest,
    pub(crate) stream: bool,
    /// Whether a stream ends with a usage chunk, as `stream_options.include_usage` asks.
    pub(crate) usage: bool,
}

/// Top-level fields accepted with no effect: they describe the caller, not the answer.
const IGNORED: &[&str] = &["user", "metadata", "store"];

/// Reads a Chat Completions request.
pub(crate) fn parse(body: &[u8]) -> Result<Parsed, Refusal> {
    let mut fields: Map<String, Value> = serde_json::from_slice(body)
        .map_err(|error| Refusal::invalid(format!("the request is not a JSON object: {error}")))?;
    let model =
        take_string(&mut fields, "model")?.ok_or_else(|| Refusal::invalid("model is required"))?;
    let Some(Value::Array(messages)) = fields.remove("messages") else {
        return Err(Refusal::invalid("messages must be an array"));
    };
    let (system, messages) = conversation(messages)?;
    let tools = match fields.remove("tools") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(tools)) => tools.iter().map(tool).collect::<Result<_, _>>()?,
        Some(_) => return Err(Refusal::invalid("tools must be an array")),
    };
    let tool_choice = match fields.remove("tool_choice") {
        None | Some(Value::Null) => ToolChoice::Auto,
        Some(Value::String(choice)) if choice == "auto" => ToolChoice::Auto,
        Some(Value::String(choice)) if choice == "none" => ToolChoice::None,
        Some(_) => {
            return Err(Refusal::unsupported(
                "tool_choice",
                "only \"auto\" and \"none\" are passed on",
            ));
        }
    };
    let max_output_tokens = match take_count(&mut fields, "max_completion_tokens")? {
        Some(tokens) => Some(tokens),
        None => take_count(&mut fields, "max_tokens")?,
    };
    let stream = take_bool(&mut fields, "stream")?.unwrap_or(false);
    let usage = match fields.remove("stream_options") {
        None | Some(Value::Null) => false,
        Some(Value::Object(options)) => {
            options.get("include_usage").and_then(Value::as_bool) == Some(true)
        }
        Some(_) => return Err(Refusal::invalid("stream_options must be an object")),
    };
    if let Some(n) = fields.remove("n")
        && n != json!(1)
    {
        return Err(Refusal::unsupported("n", "one answer per request"));
    }
    if let Some(parallel) = fields.remove("parallel_tool_calls")
        && parallel != Value::Bool(true)
    {
        return Err(Refusal::unsupported(
            "parallel_tool_calls = false",
            "models may always call several tools at once",
        ));
    }
    refuse_the_rest(&fields, IGNORED)?;
    Ok(Parsed {
        request: ModelRequest {
            model: model.clone(),
            system,
            messages,
            tools,
            tool_choice,
            max_output_tokens,
            purpose: glave_ports::Purpose::Main,
            threads: Vec::new(),
            files: Vec::new(),
            effort: None,
        },
        model,
        stream,
        usage,
    })
}

/// The system prompt (every system or developer message, joined) and the conversation.
fn conversation(messages: Vec<Value>) -> Result<(String, Vec<Message>), Refusal> {
    let mut system: Vec<String> = Vec::new();
    let mut out = Vec::with_capacity(messages.len());
    for (n, message) in messages.into_iter().enumerate() {
        let Value::Object(mut fields) = message else {
            return Err(Refusal::invalid(format!("messages[{n}] must be an object")));
        };
        if fields.contains_key("function_call") || fields.contains_key("audio") {
            return Err(Refusal::unsupported(
                &format!("messages[{n}]: function_call and audio"),
                "use tool_calls",
            ));
        }
        let role = take_string(&mut fields, "role")?.unwrap_or_default();
        let content = fields.remove("content").unwrap_or(Value::Null);
        let text = text_of(&content, &format!("messages[{n}].content"))?;
        match role.as_str() {
            "system" | "developer" => system.push(text),
            "user" => out.push(Message::User {
                parts: vec![Part::Text { text }],
            }),
            "assistant" => {
                let calls = match fields.remove("tool_calls") {
                    None | Some(Value::Null) => Vec::new(),
                    Some(Value::Array(calls)) => calls
                        .iter()
                        .map(|call| tool_call(call, n))
                        .collect::<Result<_, _>>()?,
                    Some(_) => {
                        return Err(Refusal::invalid(format!(
                            "messages[{n}].tool_calls must be an array"
                        )));
                    }
                };
                let parts = if text.is_empty() {
                    Vec::new()
                } else {
                    vec![Part::Text { text }]
                };
                out.push(Message::Assistant {
                    parts,
                    tool_calls: calls,
                });
            }
            "tool" => {
                let id = take_string(&mut fields, "tool_call_id")?.unwrap_or_default();
                out.push(Message::Tool {
                    call_id: call_id(id, n)?,
                    is_error: false,
                    output: vec![Part::Text { text }],
                });
            }
            other => {
                return Err(Refusal::invalid(format!(
                    "messages[{n}] has role {other:?}: use system, developer, user, assistant or tool"
                )));
            }
        }
    }
    Ok((system.join("\n\n"), out))
}

fn call_id(id: String, n: usize) -> Result<CallId, Refusal> {
    CallId::new(id).map_err(|error| Refusal::invalid(format!("messages[{n}]: {error}")))
}

fn tool_call(call: &Value, n: usize) -> Result<ToolCall, Refusal> {
    let id = call.get("id").and_then(Value::as_str).unwrap_or_default();
    let function = call.get("function");
    let name = function
        .and_then(|function| function.get("name"))
        .and_then(Value::as_str)
        .ok_or_else(|| Refusal::invalid(format!("messages[{n}]: a tool call names no function")))?;
    let raw = function
        .and_then(|function| function.get("arguments"))
        .and_then(Value::as_str)
        .unwrap_or("{}");
    let arguments = match serde_json::from_str::<Map<String, Value>>(raw) {
        Ok(map) => Arguments::Json(map),
        Err(_) => Arguments::Malformed(raw.to_owned()),
    };
    Ok(ToolCall {
        call_id: call_id(id.to_owned(), n)?,
        tool: name.to_owned(),
        arguments,
    })
}

fn tool(tool: &Value) -> Result<ToolSpec, Refusal> {
    if tool.get("type").and_then(Value::as_str) != Some("function") {
        return Err(Refusal::unsupported(
            "tools that are not functions",
            "only function tools are passed on",
        ));
    }
    let function = tool
        .get("function")
        .ok_or_else(|| Refusal::invalid("a tool has no function"))?;
    let name = function
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| Refusal::invalid("a tool has no name"))?;
    Ok(ToolSpec {
        name: name.to_owned(),
        description: function
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        parameters: function
            .get("parameters")
            .cloned()
            .unwrap_or_else(|| json!({"type": "object", "properties": {}})),
    })
}

#[cfg(test)]
mod tests;
