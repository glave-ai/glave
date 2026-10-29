//! A model's reasoning kept so it can be sent back: the field its text came in, and
//! OpenRouter's `reasoning_details` merged from their deltas, signed and encrypted entries
//! exactly as they came. It goes back within the current turn's tool calls, as Codex sent
//! it, on every turn only where the API demands it, and only to the provider and model that
//! produced it.

use glave_event::{Opaque, Part};
use glave_ports::Message;
use serde_json::value::RawValue;
use serde_json::{Map, Value};

/// The field OpenRouter's structured reasoning comes and goes in.
const DETAILS: &str = "reasoning_details";

/// The scalar fields reasoning text comes in: llama.cpp's, vLLM's and DeepSeek's, then
/// OpenRouter's and Ollama's.
pub(crate) const CONTENT: &str = "reasoning_content";
pub(crate) const REASONING: &str = "reasoning";

/// What one streamed response's reasoning needs to be sent back.
#[derive(Debug, Default)]
pub(crate) struct Kept {
    field: Option<&'static str>,
    details: Vec<Map<String, Value>>,
}

impl Kept {
    /// Notes the field reasoning text came in; the first one seen is the one sent back.
    pub(crate) fn came_in(&mut self, field: &'static str) {
        self.field.get_or_insert(field);
    }

    /// Adds one chunk's `reasoning_details`. Consecutive text or summary deltas of one entry
    /// become one, as the stream is their pieces in order; anything else is kept as it came.
    pub(crate) fn add_details(&mut self, deltas: Vec<Value>) {
        for delta in deltas {
            let Value::Object(delta) = delta else {
                continue;
            };
            match self.details.last_mut() {
                Some(previous) if continues(previous, &delta) => merge(previous, delta),
                _ => self.details.push(delta),
            }
        }
    }

    /// The reasoning to carry with the message, for `provider` and the `model` asked for:
    /// the details when any came, else the field the text came in, whose text is the
    /// message's own.
    pub(crate) fn into_opaque(self, provider: &str, model: &str) -> Option<Opaque> {
        let (kind, data) = if self.details.is_empty() {
            (self.field?, String::new())
        } else {
            (DETAILS, serde_json::to_string(&self.details).ok()?)
        };
        Some(Opaque {
            provider: provider.to_owned(),
            model: model.to_owned(),
            kind: kind.to_owned(),
            data,
        })
    }
}

/// The text field each mergeable entry type streams in pieces.
fn streamed_field(kind: &str) -> Option<&'static str> {
    match kind {
        "reasoning.text" => Some("text"),
        "reasoning.summary" => Some("summary"),
        _ => None,
    }
}

/// Whether `delta` is the next piece of `previous`: the same mergeable type, and no field
/// that names an entry (`id`, `index`, `format`, `signature`) set differently on both.
fn continues(previous: &Map<String, Value>, delta: &Map<String, Value>) -> bool {
    let kind = previous.get("type").and_then(Value::as_str);
    kind.is_some_and(|kind| streamed_field(kind).is_some())
        && delta.get("type").and_then(Value::as_str) == kind
        && ["id", "index", "format", "signature"].iter().all(|key| {
            match (present(previous, key), present(delta, key)) {
                (Some(a), Some(b)) => a == b,
                _ => true,
            }
        })
}

/// A field's value, unless missing, null or empty.
fn present<'a>(entry: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    entry
        .get(key)
        .filter(|value| !value.is_null() && value.as_str() != Some(""))
}

/// Appends `delta`'s text to `previous`'s and fills in the fields `previous` lacks, such as
/// a signature that arrives after the text it signs.
fn merge(previous: &mut Map<String, Value>, delta: Map<String, Value>) {
    let field = previous
        .get("type")
        .and_then(Value::as_str)
        .and_then(streamed_field);
    for (key, value) in delta {
        if Some(key.as_str()) == field
            && let (Value::String(more), Some(Value::String(text))) =
                (&value, previous.get_mut(&key))
        {
            text.push_str(more);
        } else if present(previous, &key).is_none() {
            previous.insert(key, value);
        }
    }
}

/// The providers that refuse a request with tools unless every earlier assistant message
/// carries its reasoning, earlier turns' too: DeepSeek's own API ("Thinking Mode", tool
/// calls). Through OpenRouter its models need only the current turn's.
const EVERY_TURN: &[&str] = &["api.deepseek.com"];

/// The index in `messages` from which assistant messages carry their reasoning back to
/// `provider`: those after the person's last message, the current turn's tool calls, where
/// the model continues its own chain; earlier turns' are finished and only cost tokens. From
/// the first for a provider in [`EVERY_TURN`].
pub(crate) fn sent_from(messages: &[Message], provider: &str) -> usize {
    if EVERY_TURN.contains(&provider) {
        return 0;
    }
    messages
        .iter()
        .rposition(|message| matches!(message, Message::User { .. }))
        .map_or(0, |last| last.saturating_add(1))
}

/// How an assistant message's reasoning goes back on the wire.
#[derive(Debug, Default)]
pub(crate) struct SentBack<'a> {
    pub reasoning_content: Option<String>,
    pub reasoning: Option<String>,
    pub reasoning_details: Option<&'a RawValue>,
}

/// The reasoning in `parts` to send back to `provider` for `model`: nothing unless they
/// produced it, else the details exactly as kept, or the text in the field it came in.
pub(crate) fn sent_back<'a>(parts: &'a [Part], provider: &str, model: &str) -> SentBack<'a> {
    let origin = parts.iter().find_map(|part| match part {
        Part::Reasoning {
            opaque: Some(opaque),
            ..
        } => Some(opaque),
        _ => None,
    });
    let Some(origin) = origin.filter(|o| o.provider == provider && o.model == model) else {
        return SentBack::default();
    };
    let text = || {
        parts
            .iter()
            .filter_map(|part| match part {
                Part::Reasoning { text, .. } => text.as_deref(),
                _ => None,
            })
            .collect::<String>()
    };
    match origin.kind.as_str() {
        DETAILS => SentBack {
            reasoning_details: serde_json::from_str(&origin.data).ok(),
            ..SentBack::default()
        },
        CONTENT => SentBack {
            reasoning_content: Some(text()),
            ..SentBack::default()
        },
        REASONING => SentBack {
            reasoning: Some(text()),
            ..SentBack::default()
        },
        _ => SentBack::default(),
    }
}

#[cfg(test)]
mod tests;
