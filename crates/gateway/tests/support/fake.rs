//! Shared helpers for the gateway's tests: a fake provider that answers from a script and
//! records what it was sent, optionally waiting at a gate first.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use glave_event::Principal;
use glave_event::{Currency, ModelRef, Money, Nanos, Opaque, Part, StopReason, TokenCounts, Usage};
use glave_gateway::{Credits, Model, Price, Target};
pub(crate) use glave_ports::block_on;
use glave_ports::testing::{CountingEntropy, FaultyLog, ManualClock};
use glave_ports::{
    BoxFuture, Completion, Delta, Hit, Made, MadeFile, MediaAsk, Message, ModelRequest,
    ProviderError, Purpose, SearchAsk, SearchResults, ToolChoice, ToolSpec,
};
use tokio::sync::Semaphore;

/// A provider that answers each call with the next scripted result, or a plain answer when the
/// script is empty, saying `says` as it begins when set, then `calls_with` as a tool call's
/// arguments.
pub(crate) struct Fake {
    pub provider: String,
    pub says: Option<String>,
    pub calls_with: Option<String>,
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
            says: None,
            calls_with: None,
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
            says: None,
            calls_with: None,
            script: Mutex::new(VecDeque::new()),
            seen: Mutex::new(Vec::new()),
            gate: Some(gate),
            calls: AtomicUsize::new(0),
        })
    }

    /// A gated fake whose first `answers` answers each used 5 tokens in and 50 out.
    pub(crate) fn gated_answering(
        provider: &str,
        gate: Arc<Semaphore>,
        answers: usize,
    ) -> Arc<Self> {
        let fake = Self::gated(provider, gate);
        for _ in 0..answers {
            fake.script
                .lock()
                .unwrap()
                .push_back(Ok(answer(provider, "m", 5, 50)));
        }
        fake
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
        on_delta: &'a mut (dyn FnMut(Delta) + Send),
    ) -> BoxFuture<'a, Result<Completion, ProviderError>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.seen.lock().unwrap().push(request.clone());
            if let Some(says) = &self.says {
                on_delta(Delta::Text(says.clone()));
            }
            if let Some(arguments) = &self.calls_with {
                on_delta(Delta::Arguments(arguments.clone()));
            }
            if let Some(gate) = &self.gate {
                gate.acquire().await.unwrap().forget();
            }
            let next = self.script.lock().unwrap().pop_front();
            next.unwrap_or_else(|| Ok(answer(&self.provider, &request.model, 100, 10)))
        })
    }

    /// One PNG, its answer's usage and cost the script's next, as a call's would be.
    fn make<'a>(
        &'a self,
        request: &'a ModelRequest,
        _: &'a MediaAsk,
    ) -> BoxFuture<'a, Result<Made, ProviderError>> {
        Box::pin(async move {
            let completion = self.send(request, &mut |_| {}).await?;
            let png = MadeFile {
                media_type: Some("image/png".into()),
                data: "iVBORw0KGgo=".into(),
            };
            Ok(Made {
                files: vec![png],
                completion,
            })
        })
    }

    /// One page found, or the script's next error, as a call's would be.
    fn search<'a>(
        &'a self,
        request: &'a ModelRequest,
        ask: &'a SearchAsk,
    ) -> BoxFuture<'a, Result<SearchResults, ProviderError>> {
        Box::pin(async move {
            self.send(request, &mut |_| {}).await?;
            let hit = Hit {
                title: ask.query.clone(),
                url: "https://example.com/".into(),
                snippet: "Found.".into(),
            };
            Ok(SearchResults {
                hits: vec![hit],
                cost: None,
            })
        })
    }
}

/// `per_mtok` billionths of a dollar a million tokens, in and out: at 1,000,000, a billionth
/// of a dollar a token.
pub(crate) fn price(per_mtok: i64) -> Price {
    Price::per_mtok(
        "test",
        Nanos::new(per_mtok),
        Nanos::ZERO,
        Nanos::new(per_mtok),
    )
}

/// `nanos` billionths of a dollar.
pub(crate) fn dollars(nanos: i64) -> Money {
    Money {
        nanos: Nanos::new(nanos),
        currency: Currency::USD,
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
        payer: None,
        route: None,
        end_user: None,
        session: None,
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

/// The person whose work the tests' paid calls are.
pub(crate) fn person() -> glave_event::PrincipalId {
    glave_event::PrincipalId::new(jiff::Timestamp::from_second(1).unwrap(), [1; 10]).unwrap()
}

/// The organization the tests' paid calls charge.
pub(crate) fn org() -> glave_ports::OrgId {
    let id = glave_event::PrincipalId::new(jiff::Timestamp::from_second(2).unwrap(), [2; 10]);
    glave_ports::OrgId::from_uuid(id.unwrap().as_uuid())
}

/// Credits on a log of their own, with `nanos` added for the test's organization.
pub(crate) fn funded(nanos: i64) -> Arc<Credits<FaultyLog>> {
    let credits = Arc::new(Credits::new(
        FaultyLog::default(),
        Box::new(ManualClock::ticking(1_793_350_800_000)),
        Box::new(CountingEntropy::starting_at(1)),
        None,
    ));
    let by = Principal::person(person());
    block_on!(credits.add(org(), dollars(nanos), "order-1", by)).unwrap();
    credits
}

/// What is left of the test organization's balance.
pub(crate) fn balance(credits: &Credits<FaultyLog>) -> i64 {
    block_on!(credits.account(org()))
        .unwrap()
        .balance
        .nanos
        .get()
}
