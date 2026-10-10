//! Turns streamed events into live deltas and, at the end, one finished [`Completion`]:
//! content blocks assembled in order, thinking kept with what it takes to send it back,
//! usage normalized to inclusive totals, stop reasons mapped.

use std::collections::BTreeMap;

use glave_event::{Arguments, ModelRef, Opaque, Part, StopReason, TokenCounts, Usage};
use glave_ports::{Completion, Delta, ProviderError, RequestedCall};
use serde_json::{Map, Value};

use crate::error::mid_stream;
use crate::events::{BlockDelta, StartedBlock, StreamEvent};
use crate::wire::{REDACTED_THINKING, SIGNATURE};

/// One content block as it grows.
#[derive(Debug)]
enum Block {
    Text(String),
    Thinking {
        text: String,
        signature: String,
    },
    RedactedThinking(String),
    ToolUse {
        id: String,
        name: String,
        start: Option<Value>,
        json: String,
    },
    /// A block glave does not use, such as a server tool's; its deltas are skipped.
    Other,
}

/// Accumulates one streamed response.
#[derive(Debug)]
pub(crate) struct Assembler {
    provider: String,
    /// The model as requested, which thinking is tied to: the next request names it the same
    /// way, even when the API answers with a dated name.
    requested: String,
    model: String,
    response_id: Option<String>,
    blocks: BTreeMap<u32, Block>,
    stop: Option<String>,
    usage: Option<Map<String, Value>>,
    stopped: bool,
}

impl Assembler {
    pub(crate) fn new(provider: &str, requested_model: &str) -> Self {
        Self {
            provider: provider.to_owned(),
            requested: requested_model.to_owned(),
            model: requested_model.to_owned(),
            response_id: None,
            blocks: BTreeMap::new(),
            stop: None,
            usage: None,
            stopped: false,
        }
    }

    /// Whether `message_stop` has arrived: nothing more belongs to this response.
    pub(crate) fn stopped(&self) -> bool {
        self.stopped
    }

    /// Applies one event's data, forwarding live text and thinking to `on_delta`.
    pub(crate) fn apply(
        &mut self,
        data: &str,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<(), ProviderError> {
        let event: StreamEvent = serde_json::from_str(data)
            .map_err(|e| ProviderError::Protocol(format!("unreadable event: {e}")))?;
        match event {
            StreamEvent::MessageStart { message } => {
                if let Some(id) = message.id.filter(|id| !id.is_empty()) {
                    self.response_id = Some(id);
                }
                if let Some(model) = message.model.filter(|model| !model.is_empty()) {
                    self.model = model;
                }
                self.add_usage(message.usage);
            }
            StreamEvent::ContentBlockStart {
                index,
                content_block,
            } => self.start(index, content_block, on_delta),
            StreamEvent::ContentBlockDelta { index, delta } => {
                self.grow(index, delta, on_delta)?;
            }
            StreamEvent::MessageDelta { delta, usage } => {
                if delta.stop_reason.is_some() {
                    self.stop = delta.stop_reason;
                }
                self.add_usage(usage);
            }
            StreamEvent::MessageStop => self.stopped = true,
            StreamEvent::Error { error } => return Err(mid_stream(&error)),
            StreamEvent::ContentBlockStop | StreamEvent::Ping | StreamEvent::Other => {}
        }
        Ok(())
    }

    fn start(
        &mut self,
        index: u32,
        started: StartedBlock,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) {
        let block = match started {
            StartedBlock::Text { text } => {
                if !text.is_empty() {
                    on_delta(Delta::Text(text.clone()));
                }
                Block::Text(text)
            }
            StartedBlock::Thinking {
                thinking,
                signature,
            } => {
                if !thinking.is_empty() {
                    on_delta(Delta::Reasoning(thinking.clone()));
                }
                Block::Thinking {
                    text: thinking,
                    signature,
                }
            }
            StartedBlock::RedactedThinking { data } => Block::RedactedThinking(data),
            StartedBlock::ToolUse { id, name, input } => Block::ToolUse {
                id,
                name,
                start: input,
                json: String::new(),
            },
            StartedBlock::Other => Block::Other,
        };
        self.blocks.insert(index, block);
    }

    fn grow(
        &mut self,
        index: u32,
        delta: BlockDelta,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<(), ProviderError> {
        let Some(block) = self.blocks.get_mut(&index) else {
            return Err(ProviderError::Protocol(format!(
                "a delta for content block {index}, which never started"
            )));
        };
        match (block, delta) {
            (Block::Text(text), BlockDelta::TextDelta { text: more }) => {
                if !more.is_empty() {
                    on_delta(Delta::Text(more.clone()));
                }
                text.push_str(&more);
            }
            (Block::Thinking { text, .. }, BlockDelta::ThinkingDelta { thinking }) => {
                if !thinking.is_empty() {
                    on_delta(Delta::Reasoning(thinking.clone()));
                }
                text.push_str(&thinking);
            }
            (Block::Thinking { signature, .. }, BlockDelta::SignatureDelta { signature: more }) => {
                signature.push_str(&more);
            }
            (Block::ToolUse { json, .. }, BlockDelta::InputJsonDelta { partial_json }) => {
                json.push_str(&partial_json);
            }
            (Block::Other, _) | (_, BlockDelta::Other) => {}
            (block, delta) => {
                return Err(ProviderError::Protocol(format!(
                    "content block {index} got a delta of the wrong kind: {delta:?} for {block:?}"
                )));
            }
        }
        Ok(())
    }

    /// Merges reported usage: `message_delta` repeats the counts so far, so later numbers
    /// replace earlier ones field by field.
    fn add_usage(&mut self, usage: Option<Map<String, Value>>) {
        let Some(usage) = usage else { return };
        match &mut self.usage {
            Some(existing) => existing.extend(usage.into_iter().filter(|(_, v)| !v.is_null())),
            None => self.usage = Some(usage),
        }
    }

    /// Finishes the response. A stream that ended without a stop reason is `interrupted`,
    /// and its usage is marked estimated.
    pub(crate) fn finish(self) -> Result<Completion, ProviderError> {
        let mut parts = Vec::new();
        let mut tool_calls = Vec::new();
        for block in self.blocks.into_values() {
            match block {
                Block::Text(text) if !text.is_empty() => parts.push(Part::Text { text }),
                Block::Thinking { text, signature } => {
                    let opaque = (!signature.is_empty())
                        .then(|| opaque(&self.provider, &self.requested, SIGNATURE, signature));
                    let text = (!text.is_empty()).then_some(text);
                    if text.is_some() || opaque.is_some() {
                        parts.push(Part::Reasoning { text, opaque });
                    }
                }
                Block::RedactedThinking(data) => parts.push(Part::Reasoning {
                    text: None,
                    opaque: Some(opaque(
                        &self.provider,
                        &self.requested,
                        REDACTED_THINKING,
                        data,
                    )),
                }),
                Block::ToolUse {
                    id,
                    name,
                    start,
                    json,
                } => tool_calls.push(RequestedCall {
                    provider_call_id: (!id.is_empty()).then_some(id),
                    tool: name,
                    arguments: parse_arguments(start, json),
                }),
                Block::Text(_) | Block::Other => {}
            }
        }
        let interrupted = self.stop.is_none();
        Ok(Completion {
            model: ModelRef {
                provider: self.provider,
                name: self.model,
            },
            response_id: self.response_id,
            parts,
            tool_calls,
            stop: stop_reason(self.stop.as_deref()),
            provider_stop: self.stop,
            usage: usage(self.usage, interrupted)?,
            cost: None,
        })
    }
}

fn opaque(provider: &str, model: &str, kind: &str, data: String) -> Opaque {
    Opaque {
        provider: provider.to_owned(),
        model: model.to_owned(),
        kind: kind.to_owned(),
        data,
    }
}

/// A tool call's input: the streamed JSON when any came, else the object the block started
/// with. Input that is not a JSON object is kept raw, for the model to see its mistake.
fn parse_arguments(start: Option<Value>, json: String) -> Arguments {
    if json.trim().is_empty() {
        return match start {
            Some(Value::Object(map)) => Arguments::Json(map),
            _ => Arguments::Json(Map::new()),
        };
    }
    match serde_json::from_str::<Map<String, Value>>(&json) {
        Ok(map) => Arguments::Json(map),
        Err(_) => Arguments::Malformed(json),
    }
}

fn stop_reason(stop: Option<&str>) -> StopReason {
    match stop {
        None => StopReason::Interrupted,
        Some("end_turn") => StopReason::EndTurn,
        Some("tool_use") => StopReason::ToolUse,
        Some("max_tokens") => StopReason::MaxTokens,
        Some("stop_sequence") => StopReason::StopSequence,
        Some("refusal") => StopReason::Refusal,
        Some("pause_turn") => StopReason::Pause,
        Some("model_context_window_exceeded") => StopReason::ContextExceeded,
        Some(_) => StopReason::Error,
    }
}

/// Normalizes Messages usage to inclusive totals. Anthropic counts `input_tokens` apart from
/// the tokens read from or written to the cache, so input is their sum; `output_tokens`
/// already includes thinking, which the API does not count separately.
fn usage(raw: Option<Map<String, Value>>, interrupted: bool) -> Result<Usage, ProviderError> {
    let count = |field: &str| {
        raw.as_ref()
            .and_then(|usage| usage.get(field))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    let cache_read = count("cache_read_input_tokens");
    let cache_write = count("cache_creation_input_tokens");
    let counts = TokenCounts {
        input: count("input_tokens")
            .saturating_add(cache_read)
            .saturating_add(cache_write),
        cache_read,
        cache_write,
        output: count("output_tokens"),
        reasoning: 0,
    };
    // Without any reported usage the counts are a guess, whatever the stop.
    let estimated = interrupted || raw.is_none();
    Usage::new(counts, estimated, raw.map(Value::Object))
        .map_err(|e| ProviderError::Protocol(e.to_string()))
}

#[cfg(test)]
mod tests;
