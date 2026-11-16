//! The answers tests script the model with, and what it was sent.

use glave_event::{
    Arguments, Cost, Currency, ModelRef, Money, Nanos, Part, StopReason, TokenCounts, Usage,
};
use glave_ports::{Completion, RequestedCall};

pub(crate) use glave_ports::testing::{ScriptedModel, Seen};

/// Every request a scripted model received, as JSON.
pub(crate) fn json(seen: &Seen) -> Vec<String> {
    let seen = seen.lock().unwrap();
    seen.iter()
        .map(|request| serde_json::to_string(request).unwrap())
        .collect()
}

/// Lets the server's work run until the model has been asked `calls` times.
pub(crate) async fn asked(seen: &Seen, calls: usize) {
    for _ in 0..100_000 {
        if seen.lock().unwrap().len() >= calls {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("the model was never asked {calls} times");
}

/// An answer saying `text`, or calling a tool with arguments.
pub(crate) fn answer(text: Option<&str>, call: Option<(&str, serde_json::Value)>) -> Completion {
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
        upstream: None,
    }
}

/// `completion`, having used `input` and `output` tokens and cost `nanos`.
pub(crate) fn costing(
    mut completion: Completion,
    input: u64,
    output: u64,
    nanos: i64,
) -> Completion {
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
            nanos: Nanos::new(nanos),
            currency: Currency::USD,
        },
        price_list: "test".into(),
    });
    completion
}

/// A one-pixel PNG, as an image tool's provider would send it, in base64.
pub(crate) const PIXEL: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

/// A model answering `answers` in turn that draws [`PIXEL`] for every image tool's call.
pub(crate) fn drawing(answers: Vec<Completion>) -> ScriptedModel {
    let made = glave_ports::Made {
        files: vec![glave_ports::MadeFile {
            media_type: Some("image/png".into()),
            data: PIXEL.into(),
        }],
        completion: answer(None, None),
    };
    ScriptedModel::new(answers).making(Ok(made))
}
