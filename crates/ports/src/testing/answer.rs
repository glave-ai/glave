//! A scripted model's answer, as tests write one: what it says and the tools it calls.

use glave_event::{Arguments, InvalidValue, ModelRef, Part, StopReason, TokenCounts, Usage};
use serde_json::Value;

use crate::model::{Completion, RequestedCall};

/// An answer saying `text`, when given, and calling each tool with its arguments, a JSON object
/// (anything else is no arguments): it ends the turn when it calls none, and asks for the
/// tools' results when it calls any. Each call's id from the provider is `p_<tool>`. It read
/// 100 tokens and wrote 10, and costs nothing.
pub fn answer(text: Option<&str>, calls: Vec<(&str, Value)>) -> Result<Completion, InvalidValue> {
    let counts = TokenCounts {
        input: 100,
        cache_read: 0,
        cache_write: 0,
        output: 10,
        reasoning: 0,
    };
    let stop = if calls.is_empty() {
        StopReason::EndTurn
    } else {
        StopReason::ToolUse
    };
    Ok(Completion {
        model: ModelRef {
            provider: "script".into(),
            name: "test-model".into(),
        },
        response_id: None,
        parts: text
            .map(|text| Part::Text { text: text.into() })
            .into_iter()
            .collect(),
        tool_calls: calls
            .into_iter()
            .map(|(tool, arguments)| RequestedCall {
                provider_call_id: Some(format!("p_{tool}")),
                tool: tool.into(),
                arguments: Arguments::Json(arguments.as_object().cloned().unwrap_or_default()),
            })
            .collect(),
        stop,
        provider_stop: None,
        usage: Usage::new(counts, false, None)?,
        cost: None,
        served: None,
        upstream: None,
    })
}
