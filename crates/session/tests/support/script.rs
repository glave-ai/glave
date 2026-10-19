//! The answers tests script the model with, and what it was sent.

use glave_event::{
    Arguments, Cost, Currency, ModelRef, Money, Part, StopReason, TokenCounts, Usage,
};
use glave_ports::{Completion, RequestedCall};

pub use glave_ports::testing::{ScriptedModel, Seen};

/// Every request a scripted model received, as JSON.
pub fn json(seen: &Seen) -> Vec<String> {
    let seen = seen.lock().unwrap();
    seen.iter()
        .map(|request| serde_json::to_string(request).unwrap())
        .collect()
}

/// An answer saying `text`, or calling a tool with arguments.
pub fn answer(text: Option<&str>, call: Option<(&str, serde_json::Value)>) -> Completion {
    Completion {
        model: ModelRef {
            provider: "script".into(),
            name: "test-model".into(),
        },
        response_id: None,
        parts: text
            .map(|t| Part::Text { text: t.into() })
            .into_iter()
            .collect(),
        tool_calls: call
            .map(|(tool, args)| RequestedCall {
                provider_call_id: None,
                tool: tool.into(),
                arguments: Arguments::Json(args.as_object().unwrap().clone()),
            })
            .into_iter()
            .collect(),
        stop: if text.is_some() {
            StopReason::EndTurn
        } else {
            StopReason::ToolUse
        },
        provider_stop: None,
        usage: Usage::new(TokenCounts::default(), false, None).unwrap(),
        cost: None,
        served: None,
    }
}

/// `completion`, having used `input` and `output` tokens and cost `micros`.
pub fn costing(mut completion: Completion, input: u64, output: u64, micros: i64) -> Completion {
    completion.usage = Usage::new(
        TokenCounts {
            input,
            output,
            ..TokenCounts::default()
        },
        false,
        None,
    )
    .unwrap();
    completion.cost = Some(Cost {
        amount: Money {
            micros,
            currency: Currency::USD,
        },
        price_list: "test".into(),
    });
    completion
}
