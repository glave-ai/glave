//! An answer as Chat Completions writes it: streamed chunks ending in `[DONE]`, or one
//! `chat.completion` object. Usage counts are the gateway's inclusive totals, with cached and
//! reasoning tokens as their details, and the cost when it is in US dollars.

use bytes::Bytes;
use glave_event::{Arguments, Currency, Part, StopReason, TokenCounts};
use glave_ports::{Completion, Delta};
use serde::Serialize;
use serde_json::value::RawValue;
use serde_json::{Value, json};

use crate::errors::Refusal;
use crate::stream::{Render, event};

/// Writes a streamed answer, chunk by chunk.
pub(crate) struct Chunks {
    id: String,
    created: i64,
    /// The model asked for, named in every chunk until the last says which answered.
    model: String,
    /// Whether to end with a usage chunk.
    usage: bool,
    /// Whether the first chunk, which names the role, was sent.
    begun: bool,
}

impl Chunks {
    pub(crate) fn new(id: String, created: i64, model: String, usage: bool) -> Self {
        Self {
            id,
            created,
            model,
            usage,
            begun: false,
        }
    }

    fn chunk(&self, model: &str, choices: &Value) -> Value {
        json!({
            "id": self.id, "object": "chat.completion.chunk", "created": self.created,
            "model": model, "choices": choices,
        })
    }
}

impl Render for Chunks {
    fn delta(&mut self, delta: Delta) -> Vec<Bytes> {
        let mut fields = serde_json::Map::new();
        if !self.begun {
            self.begun = true;
            fields.insert("role".into(), "assistant".into());
        }
        match delta {
            Delta::Text(text) => fields.insert("content".into(), text.into()),
            Delta::Reasoning(text) => fields.insert("reasoning_content".into(), text.into()),
        };
        let choices = json!([{"index": 0, "delta": fields, "finish_reason": null}]);
        vec![event(None, &self.chunk(&self.model, &choices))]
    }

    fn finish(&mut self, completion: &Completion) -> Vec<Bytes> {
        let model = completion.model.name.clone();
        let mut events = Vec::new();
        let calls = tool_calls(completion);
        if !calls.is_empty() || !self.begun {
            let mut delta = serde_json::Map::new();
            delta.insert("role".into(), "assistant".into());
            if !calls.is_empty() {
                delta.insert("tool_calls".into(), Value::Array(calls));
            }
            let choices = json!([{"index": 0, "delta": delta, "finish_reason": null}]);
            events.push(event(None, &self.chunk(&model, &choices)));
        }
        let finished = json!([{
            "index": 0, "delta": {}, "finish_reason": finish_reason(completion.stop),
        }]);
        events.push(event(None, &self.chunk(&model, &finished)));
        if self.usage {
            let last = Whole {
                id: &self.id,
                object: "chat.completion.chunk",
                created: self.created,
                model: &model,
                choices: json!([]),
                usage: usage(completion),
            };
            events.push(event(None, &last));
        }
        events.push(Bytes::from_static(b"data: [DONE]\n\n"));
        events
    }

    fn fail(&mut self, refusal: &Refusal) -> Vec<Bytes> {
        vec![
            event(None, &refusal.openai()),
            Bytes::from_static(b"data: [DONE]\n\n"),
        ]
    }
}

/// An answer, or a stream's last chunk, with its usage.
#[derive(Serialize)]
struct Whole<'a> {
    id: &'a str,
    object: &'static str,
    created: i64,
    model: &'a str,
    choices: Value,
    usage: Usage,
}

/// A whole answer, for a client that did not ask for a stream, as JSON text.
pub(crate) fn whole(id: &str, created: i64, completion: &Completion) -> String {
    let mut text = String::new();
    let mut reasoning = String::new();
    for part in &completion.parts {
        match part {
            Part::Text { text: piece } => text.push_str(piece),
            Part::Reasoning {
                text: Some(piece), ..
            } => reasoning.push_str(piece),
            Part::Reasoning { .. } | Part::File { .. } => {}
        }
    }
    let mut message = serde_json::Map::new();
    message.insert("role".into(), "assistant".into());
    let content = if text.is_empty() {
        Value::Null
    } else {
        Value::String(text)
    };
    message.insert("content".into(), content);
    if !reasoning.is_empty() {
        message.insert("reasoning_content".into(), Value::String(reasoning));
    }
    let calls = tool_calls(completion);
    if !calls.is_empty() {
        message.insert("tool_calls".into(), Value::Array(calls));
    }
    let answer = Whole {
        id,
        object: "chat.completion",
        created,
        model: &completion.model.name,
        choices: json!([{
            "index": 0, "message": message, "finish_reason": finish_reason(completion.stop),
        }]),
        usage: usage(completion),
    };
    serde_json::to_string(&answer).unwrap_or_default()
}

fn tool_calls(completion: &Completion) -> Vec<Value> {
    completion
        .tool_calls
        .iter()
        .enumerate()
        .map(|(n, call)| {
            let arguments = match &call.arguments {
                Arguments::Json(map) => serde_json::to_string(map).unwrap_or_default(),
                Arguments::Malformed(raw) => raw.clone(),
            };
            let id = call
                .provider_call_id
                .clone()
                .unwrap_or_else(|| format!("call_{n}"));
            json!({
                "index": n, "id": id, "type": "function",
                "function": {"name": call.tool, "arguments": arguments},
            })
        })
        .collect()
}

fn finish_reason(stop: StopReason) -> &'static str {
    match stop {
        StopReason::ToolUse => "tool_calls",
        StopReason::MaxTokens | StopReason::ContextExceeded => "length",
        StopReason::ContentFilter | StopReason::Refusal => "content_filter",
        StopReason::EndTurn
        | StopReason::StopSequence
        | StopReason::Pause
        | StopReason::MalformedToolCall
        | StopReason::Error
        | StopReason::Cancelled
        | StopReason::Interrupted => "stop",
    }
}

#[derive(Serialize)]
struct Usage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
    prompt_tokens_details: PromptDetails,
    completion_tokens_details: CompletionDetails,
    /// US dollars as an exact decimal, as OpenRouter reports cost.
    #[serde(skip_serializing_if = "Option::is_none")]
    cost: Option<Box<RawValue>>,
}

#[derive(Serialize)]
struct PromptDetails {
    cached_tokens: u64,
    cache_write_tokens: u64,
}

#[derive(Serialize)]
struct CompletionDetails {
    reasoning_tokens: u64,
}

fn usage(completion: &Completion) -> Usage {
    let TokenCounts {
        input,
        cache_read,
        cache_write,
        output,
        reasoning,
    } = completion.usage.counts();
    let cost = completion
        .cost
        .as_ref()
        .filter(|cost| cost.amount.currency == Currency::USD)
        .and_then(|cost| decimal(cost.amount.micros));
    Usage {
        prompt_tokens: input,
        completion_tokens: output,
        total_tokens: input.saturating_add(output),
        prompt_tokens_details: PromptDetails {
            cached_tokens: cache_read,
            cache_write_tokens: cache_write,
        },
        completion_tokens_details: CompletionDetails {
            reasoning_tokens: reasoning,
        },
        cost,
    }
}

/// Millionths as an exact decimal: 4200 is `0.0042`.
fn decimal(micros: i64) -> Option<Box<RawValue>> {
    let sign = if micros < 0 { "-" } else { "" };
    let micros = micros.unsigned_abs();
    let text = format!("{sign}{}.{:06}", micros / 1_000_000, micros % 1_000_000);
    RawValue::from_string(text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn costs_are_exact_decimals() {
        assert_eq!(decimal(4_200).unwrap().get(), "0.004200");
        assert_eq!(decimal(12_345_678).unwrap().get(), "12.345678");
        assert_eq!(decimal(-1).unwrap().get(), "-0.000001");
    }
}
