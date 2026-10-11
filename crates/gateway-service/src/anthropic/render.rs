//! An answer as the Messages API writes it: the streamed events (`message_start`, content
//! blocks, `message_delta`, `message_stop`), or one `message` object. Usage follows the API's
//! own split: `input_tokens` counts only what was neither read from nor written to the cache.

use bytes::Bytes;
use glave_event::{Arguments, Part, StopReason, TokenCounts};
use glave_ports::{Completion, Delta};
use serde_json::{Value, json};

use crate::errors::Refusal;
use crate::stream::{Render, event};

/// The kind of content block open in a stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Open {
    Text,
    Thinking,
}

/// Writes a streamed answer, event by event.
pub(crate) struct Events {
    id: String,
    model: String,
    /// Whether `message_start` was sent.
    begun: bool,
    /// The block being written, and its index.
    open: Option<(Open, u32)>,
    /// The index the next block takes.
    next: u32,
}

impl Events {
    pub(crate) fn new(id: String, model: String) -> Self {
        Self {
            id,
            model,
            begun: false,
            open: None,
            next: 0,
        }
    }

    fn begin(&mut self, events: &mut Vec<Bytes>) {
        if self.begun {
            return;
        }
        self.begun = true;
        let message = json!({
            "id": self.id, "type": "message", "role": "assistant", "model": self.model,
            "content": [], "stop_reason": null, "stop_sequence": null,
            "usage": {"input_tokens": 0, "output_tokens": 0},
        });
        events.push(event(
            Some("message_start"),
            &json!({"type": "message_start", "message": message}),
        ));
    }

    fn close(&mut self, events: &mut Vec<Bytes>) {
        if let Some((_, index)) = self.open.take() {
            events.push(event(
                Some("content_block_stop"),
                &json!({"type": "content_block_stop", "index": index}),
            ));
        }
    }

    /// Opens a block of `kind` unless one is open, giving its index.
    fn open(&mut self, kind: Open, events: &mut Vec<Bytes>) -> u32 {
        if let Some((open, index)) = self.open
            && open == kind
        {
            return index;
        }
        self.close(events);
        let index = self.next;
        self.next = self.next.saturating_add(1);
        self.open = Some((kind, index));
        let block = match kind {
            Open::Text => json!({"type": "text", "text": ""}),
            Open::Thinking => json!({"type": "thinking", "thinking": "", "signature": ""}),
        };
        events.push(event(
            Some("content_block_start"),
            &json!({"type": "content_block_start", "index": index, "content_block": block}),
        ));
        index
    }
}

impl Render for Events {
    fn delta(&mut self, delta: Delta) -> Vec<Bytes> {
        let mut events = Vec::new();
        self.begin(&mut events);
        let (index, piece) = match delta {
            Delta::Text(text) => (
                self.open(Open::Text, &mut events),
                json!({"type": "text_delta", "text": text}),
            ),
            Delta::Reasoning(text) => (
                self.open(Open::Thinking, &mut events),
                json!({"type": "thinking_delta", "thinking": text}),
            ),
        };
        events.push(event(
            Some("content_block_delta"),
            &json!({"type": "content_block_delta", "index": index, "delta": piece}),
        ));
        events
    }

    fn finish(&mut self, completion: &Completion) -> Vec<Bytes> {
        let mut events = Vec::new();
        self.begin(&mut events);
        self.close(&mut events);
        for (n, call) in completion.tool_calls.iter().enumerate() {
            let index = self.next;
            self.next = self.next.saturating_add(1);
            let id = call
                .provider_call_id
                .clone()
                .unwrap_or_else(|| format!("toolu_{n}"));
            let block = json!({"type": "tool_use", "id": id, "name": call.tool, "input": {}});
            events.push(event(
                Some("content_block_start"),
                &json!({"type": "content_block_start", "index": index, "content_block": block}),
            ));
            let delta =
                json!({"type": "input_json_delta", "partial_json": arguments(&call.arguments)});
            events.push(event(
                Some("content_block_delta"),
                &json!({"type": "content_block_delta", "index": index, "delta": delta}),
            ));
            events.push(event(
                Some("content_block_stop"),
                &json!({"type": "content_block_stop", "index": index}),
            ));
        }
        events.push(event(
            Some("message_delta"),
            &json!({
                "type": "message_delta",
                "delta": {"stop_reason": stop_reason(completion.stop), "stop_sequence": null},
                "usage": usage(completion),
            }),
        ));
        events.push(event(
            Some("message_stop"),
            &json!({"type": "message_stop"}),
        ));
        events
    }

    fn fail(&mut self, refusal: &Refusal) -> Vec<Bytes> {
        vec![event(Some("error"), &refusal.anthropic())]
    }
}

/// A whole answer, for a client that did not ask for a stream.
pub(crate) fn whole(id: &str, completion: &Completion) -> Value {
    let mut content = Vec::new();
    for part in &completion.parts {
        match part {
            Part::Text { text } => content.push(json!({"type": "text", "text": text})),
            Part::Reasoning {
                text: Some(text), ..
            } => content.push(json!({"type": "thinking", "thinking": text, "signature": ""})),
            Part::Reasoning { .. } | Part::File { .. } => {}
        }
    }
    for (n, call) in completion.tool_calls.iter().enumerate() {
        let input: Value =
            serde_json::from_str(&arguments(&call.arguments)).unwrap_or_else(|_| json!({}));
        let id = call
            .provider_call_id
            .clone()
            .unwrap_or_else(|| format!("toolu_{n}"));
        content.push(json!({"type": "tool_use", "id": id, "name": call.tool, "input": input}));
    }
    json!({
        "id": id, "type": "message", "role": "assistant", "model": completion.model.name,
        "content": content, "stop_reason": stop_reason(completion.stop), "stop_sequence": null,
        "usage": usage(completion),
    })
}

fn arguments(arguments: &Arguments) -> String {
    match arguments {
        Arguments::Json(map) => serde_json::to_string(map).unwrap_or_else(|_| "{}".into()),
        Arguments::Malformed(raw) => raw.clone(),
    }
}

fn stop_reason(stop: StopReason) -> &'static str {
    match stop {
        StopReason::ToolUse => "tool_use",
        StopReason::MaxTokens | StopReason::ContextExceeded => "max_tokens",
        StopReason::StopSequence => "stop_sequence",
        StopReason::Refusal | StopReason::ContentFilter => "refusal",
        StopReason::Pause => "pause_turn",
        StopReason::EndTurn
        | StopReason::MalformedToolCall
        | StopReason::Error
        | StopReason::Cancelled
        | StopReason::Interrupted => "end_turn",
    }
}

fn usage(completion: &Completion) -> Value {
    let TokenCounts {
        input,
        cache_read,
        cache_write,
        output,
        ..
    } = completion.usage.counts();
    json!({
        "input_tokens": input.saturating_sub(cache_read).saturating_sub(cache_write),
        "cache_creation_input_tokens": cache_write,
        "cache_read_input_tokens": cache_read,
        "output_tokens": output,
    })
}
