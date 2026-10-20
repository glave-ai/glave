//! Turns streamed chunks into live deltas and, at the end, one finished [`Completion`]:
//! tool-call fragments merged, usage normalized to inclusive totals, stop reasons mapped.

use std::collections::BTreeMap;

use glave_event::{Arguments, Cost, ModelRef, Money, Part, StopReason, TokenCounts, Usage};
use glave_ports::{Completion, Delta, ProviderError, RequestedCall};
use serde_json::value::RawValue;
use serde_json::{Map, Value};

use crate::wire::{Chunk, WireUsage};

/// The longest provider name a chunk may give, in bytes.
const LONGEST_UPSTREAM: usize = 100;

#[derive(Debug, Default)]
struct PartialCall {
    id: Option<String>,
    name: String,
    arguments: String,
}

/// Accumulates one streamed response.
#[derive(Debug)]
pub(crate) struct Assembler {
    provider: String,
    model: String,
    response_id: Option<String>,
    parts: Vec<Part>,
    calls: BTreeMap<u32, PartialCall>,
    finish: Option<String>,
    usage: Option<Box<RawValue>>,
    /// The provider the service passed the call on to, as its chunks name it.
    upstream: Option<String>,
}

impl Assembler {
    pub(crate) fn new(provider: &str, requested_model: &str) -> Self {
        Self {
            provider: provider.to_owned(),
            model: requested_model.to_owned(),
            response_id: None,
            parts: Vec::new(),
            calls: BTreeMap::new(),
            finish: None,
            usage: None,
            upstream: None,
        }
    }

    /// Applies one event's data, forwarding live text and reasoning to `on_delta`.
    pub(crate) fn apply(
        &mut self,
        data: &str,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<(), ProviderError> {
        let chunk: Chunk = serde_json::from_str(data)
            .map_err(|e| ProviderError::Protocol(format!("unreadable chunk: {e}")))?;
        if let Some(error) = chunk.error {
            let reason = format!("error mid-stream: {error}");
            return Err(if self.has_output() {
                ProviderError::Protocol(reason)
            } else {
                ProviderError::NoOutput(reason)
            });
        }
        if let Some(id) = chunk.id.filter(|id| !id.is_empty()) {
            self.response_id = Some(id);
        }
        if let Some(model) = chunk.model.filter(|model| !model.is_empty()) {
            self.model = model;
        }
        let named = chunk.provider.map(|provider| provider.trim().to_owned());
        if let Some(provider) = named.filter(|p| !p.is_empty() && p.len() <= LONGEST_UPSTREAM) {
            self.upstream = Some(provider);
        }
        if chunk.usage.is_some() {
            self.usage = chunk.usage;
        }
        for choice in chunk.choices {
            if let Some(reason) = choice.finish_reason {
                self.finish = Some(reason);
            }
            let Some(delta) = choice.delta else { continue };
            if let Some(reasoning) = delta.reasoning_content.or(delta.reasoning) {
                self.push_reasoning(&reasoning, on_delta);
            }
            if let Some(text) = delta.content {
                self.push_text(&text, on_delta);
            }
            for (position, call) in delta.tool_calls.into_iter().flatten().enumerate() {
                let index = call
                    .index
                    .unwrap_or_else(|| u32::try_from(position).unwrap_or(u32::MAX));
                let partial = self.calls.entry(index).or_default();
                if let Some(id) = call.id.filter(|id| !id.is_empty()) {
                    partial.id = Some(id);
                }
                if let Some(function) = call.function {
                    partial
                        .name
                        .push_str(function.name.as_deref().unwrap_or(""));
                    partial
                        .arguments
                        .push_str(function.arguments.as_deref().unwrap_or(""));
                }
            }
        }
        Ok(())
    }

    /// Whether anything has been shown or asked for yet: text, reasoning or a tool call.
    fn has_output(&self) -> bool {
        !self.parts.is_empty() || !self.calls.is_empty()
    }

    fn push_text(&mut self, text: &str, on_delta: &mut (dyn FnMut(Delta) + Send)) {
        if text.is_empty() {
            return;
        }
        on_delta(Delta::Text(text.to_owned()));
        if let Some(Part::Text { text: existing }) = self.parts.last_mut() {
            existing.push_str(text);
        } else {
            self.parts.push(Part::Text {
                text: text.to_owned(),
            });
        }
    }

    fn push_reasoning(&mut self, text: &str, on_delta: &mut (dyn FnMut(Delta) + Send)) {
        if text.is_empty() {
            return;
        }
        on_delta(Delta::Reasoning(text.to_owned()));
        if let Some(Part::Reasoning {
            text: Some(existing),
            ..
        }) = self.parts.last_mut()
        {
            existing.push_str(text);
        } else {
            self.parts.push(Part::Reasoning {
                text: Some(text.to_owned()),
                opaque: None,
            });
        }
    }

    /// Finishes the response. A stream that ended without a finish reason is `interrupted`,
    /// and its usage is marked estimated.
    pub(crate) fn finish(self) -> Result<Completion, ProviderError> {
        let tool_calls: Vec<RequestedCall> = self
            .calls
            .into_values()
            .map(|call| RequestedCall {
                provider_call_id: call.id,
                tool: call.name,
                arguments: parse_arguments(call.arguments),
            })
            .collect();
        let stop = stop_reason(self.finish.as_deref(), !tool_calls.is_empty());
        if stop == StopReason::Error && self.parts.is_empty() && tool_calls.is_empty() {
            return Err(ProviderError::NoOutput(format!(
                "finish reason {}",
                self.finish.as_deref().unwrap_or("error")
            )));
        }
        let interrupted = self.finish.is_none();
        let (usage, cost) = usage_and_cost(self.usage.as_deref(), interrupted, &self.provider)?;
        Ok(Completion {
            model: ModelRef {
                provider: self.provider,
                name: self.model,
            },
            response_id: self.response_id,
            parts: self.parts,
            tool_calls,
            stop,
            provider_stop: self.finish,
            usage,
            cost,
            served: None,
            upstream: self.upstream,
        })
    }
}

fn parse_arguments(raw: String) -> Arguments {
    if raw.trim().is_empty() {
        return Arguments::Json(Map::new());
    }
    match serde_json::from_str::<Map<String, Value>>(&raw) {
        Ok(map) => Arguments::Json(map),
        Err(_) => Arguments::Malformed(raw),
    }
}

fn stop_reason(finish: Option<&str>, has_tool_calls: bool) -> StopReason {
    match finish {
        None => StopReason::Interrupted,
        Some(_) if has_tool_calls => StopReason::ToolUse,
        Some("stop") => StopReason::EndTurn,
        Some("length") => StopReason::MaxTokens,
        Some("tool_calls" | "function_call") => StopReason::ToolUse,
        Some("content_filter") => StopReason::ContentFilter,
        Some(_) => StopReason::Error,
    }
}

/// Normalizes chat-completions usage to inclusive totals: `prompt_tokens` already includes
/// cached tokens, and reasoning is part of completion tokens. Inconsistent numbers from a
/// provider are clamped and the usage marked estimated rather than rejected.
fn usage_and_cost(
    raw: Option<&RawValue>,
    interrupted: bool,
    provider: &str,
) -> Result<(Usage, Option<Cost>), ProviderError> {
    let Some(raw) = raw else {
        let usage = Usage::new(TokenCounts::default(), true, None)
            .map_err(|e| ProviderError::Protocol(e.to_string()))?;
        return Ok((usage, None));
    };
    let wire: WireUsage = serde_json::from_str(raw.get())
        .map_err(|e| ProviderError::Protocol(format!("unreadable usage: {e}")))?;
    let input = wire.prompt_tokens;
    let output = wire.completion_tokens;
    let details = wire.prompt_tokens_details.unwrap_or_default();
    let cache_read = details.cached_tokens.unwrap_or(0).min(input);
    let cache_write = details
        .cache_write_tokens
        .unwrap_or(0)
        .min(input.saturating_sub(cache_read));
    let reasoning = wire
        .completion_tokens_details
        .unwrap_or_default()
        .reasoning_tokens
        .unwrap_or(0)
        .min(output);
    let counts = TokenCounts {
        input,
        cache_read,
        cache_write,
        output,
        reasoning,
    };
    let raw_value: Option<Value> = serde_json::from_str(raw.get()).ok();
    let usage = Usage::new(counts, interrupted, raw_value)
        .map_err(|e| ProviderError::Protocol(e.to_string()))?;
    let cost = wire
        .cost
        .and_then(|cost| Money::parse_dollars(cost.get()).ok())
        .map(|amount| Cost {
            amount,
            price_list: format!("{provider}:reported"),
        });
    Ok((usage, cost))
}

#[cfg(test)]
mod tests;
