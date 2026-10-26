//! Shared helpers for the gateway's tests: a fake provider that answers from a script and
//! records what it was sent, optionally waiting at a gate first.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use glave_event::{ModelRef, Opaque, Part, StopReason, TokenCounts, Usage};
use glave_gateway::{Model, Target};
pub(crate) use glave_ports::block_on;
use glave_ports::{
    BoxFuture, Completion, Delta, Message, ModelRequest, ProviderError, Purpose, ToolChoice,
    ToolSpec,
};
use tokio::sync::Semaphore;

/// A provider that answers each call with the next scripted result, or a plain answer when the
/// script is empty.
pub(crate) struct Fake {
    pub provider: String,
    pub script: Mutex<VecDeque<Result<Completion, ProviderError>>>,
    pub seen: Mutex<Vec<ModelRequest>>,
    /// When set, every call waits for a permit before answering.
    pub gate: Option<Arc<Semaphore>>,
    /// Calls that reached the provider.
    pub calls: AtomicUsize,
}

impl Fake {
    pub(crate) fn new(provider: &str) -> Arc<Self> {
        Arc::new(Self {
            provider: provider.into(),
            script: Mutex::new(VecDeque::new()),
            seen: Mutex::new(Vec::new()),
            gate: None,
            calls: AtomicUsize::new(0),
        })
    }

    pub(crate) fn failing(provider: &str, error: ProviderError) -> Arc<Self> {
        let fake = Self::new(provider);
        fake.script.lock().unwrap().push_back(Err(error));
        fake
    }

    pub(crate) fn gated(provider: &str, gate: Arc<Semaphore>) -> Arc<Self> {
        Arc::new(Self {
            provider: provider.into(),
            script: Mutex::new(VecDeque::new()),
            seen: Mutex::new(Vec::new()),
            gate: Some(gate),
            calls: AtomicUsize::new(0),
        })
    }

    pub(crate) fn seen(&self) -> Vec<ModelRequest> {
        self.seen.lock().unwrap().clone()
    }

    pub(crate) fn target(self: &Arc<Self>, model: &str) -> Target {
        Target::new(
            Arc::clone(self) as Arc<dyn Model>,
            self.provider.clone(),
            model,
        )
    }
}

impl Model for Fake {
    fn send<'a>(
        &'a self,
        request: &'a ModelRequest,
        _on_delta: &'a mut (dyn FnMut(Delta) + Send),
    ) -> BoxFuture<'a, Result<Completion, ProviderError>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.seen.lock().unwrap().push(request.clone());
            if let Some(gate) = &self.gate {
                gate.acquire().await.unwrap().forget();
            }
            let next = self.script.lock().unwrap().pop_front();
            next.unwrap_or_else(|| Ok(answer(&self.provider, &request.model, 100, 10)))
        })
    }
}

/// A plain text answer from `model` at `provider`, with this usage.
pub(crate) fn answer(provider: &str, model: &str, input: u64, output: u64) -> Completion {
    let counts = TokenCounts {
        input,
        output,
        ..TokenCounts::default()
    };
    Completion {
        model: ModelRef {
            provider: provider.into(),
            name: model.into(),
        },
        response_id: None,
        parts: vec![Part::Text { text: "ok".into() }],
        tool_calls: Vec::new(),
        stop: StopReason::EndTurn,
        provider_stop: None,
        usage: Usage::new(counts, false, None).unwrap(),
        cost: None,
        served: None,
        upstream: None,
    }
}

/// A short request for `purpose`, with no tools.
pub(crate) fn request(purpose: Purpose) -> ModelRequest {
    ModelRequest {
        model: "asked-for".into(),
        system: "Be brief.".into(),
        messages: vec![Message::User {
            parts: vec![Part::Text {
                text: "hello".into(),
            }],
        }],
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
        max_output_tokens: Some(100),
        purpose,
        files: Vec::new(),
        effort: None,
    }
}

/// The same request, offering one tool.
pub(crate) fn with_tool(mut request: ModelRequest) -> ModelRequest {
    request.tools.push(ToolSpec {
        name: "read".into(),
        description: "Read a file".into(),
        parameters: serde_json::json!({"type": "object"}),
    });
    request
}

/// The same request, after an answer that carried `provider`'s opaque reasoning for `model`.
pub(crate) fn with_reasoning(
    mut request: ModelRequest,
    provider: &str,
    model: &str,
) -> ModelRequest {
    request.messages.push(Message::Assistant {
        parts: vec![
            Part::Reasoning {
                text: Some("thinking".into()),
                opaque: Some(Opaque {
                    provider: provider.into(),
                    model: model.into(),
                    kind: "signature".into(),
                    data: "sig".into(),
                }),
            },
            Part::Text { text: "Hi.".into() },
        ],
        tool_calls: Vec::new(),
    });
    request.messages.push(Message::User {
        parts: vec![Part::Text {
            text: "and then?".into(),
        }],
    });
    request
}
