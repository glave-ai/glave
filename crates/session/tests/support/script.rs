//! A model that answers from a script, and the answers tests script it with.

use std::collections::VecDeque;
use std::future::Future;
use std::sync::{Arc, Mutex};

use glave_event::{
    Arguments, Cost, Currency, ModelRef, Money, Part, StopReason, TokenCounts, Usage,
};
use glave_ports::{Completion, Delta, ModelProvider, ModelRequest, ProviderError, RequestedCall};
use tokio::sync::Notify;

/// Every request a scripted model received, as JSON.
pub type Seen = Arc<Mutex<Vec<String>>>;

/// A model that answers from a script, optionally waiting at a gate before each answer.
pub struct Script {
    answers: Mutex<VecDeque<Completion>>,
    gate: Option<Arc<Notify>>,
    seen: Seen,
}

impl Script {
    /// A model answering `answers` in order, and what it will have received.
    pub fn new(answers: Vec<Completion>, gate: Option<Arc<Notify>>) -> (Self, Seen) {
        let seen = Seen::default();
        let script = Self {
            answers: Mutex::new(answers.into()),
            gate,
            seen: Arc::clone(&seen),
        };
        (script, seen)
    }
}

impl ModelProvider for Script {
    fn complete(
        &self,
        request: &ModelRequest,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> impl Future<Output = Result<Completion, ProviderError>> + Send {
        self.seen
            .lock()
            .unwrap()
            .push(serde_json::to_string(request).unwrap());
        let next = self.answers.lock().unwrap().pop_front();
        if let Some(Part::Text { text }) = next.as_ref().and_then(|c| c.parts.first()) {
            on_delta(Delta::Text(text.clone()));
        }
        let gate = self.gate.clone();
        async move {
            if let Some(gate) = gate {
                gate.notified().await;
            }
            next.ok_or_else(|| ProviderError::Protocol("script exhausted".into()))
        }
    }
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
