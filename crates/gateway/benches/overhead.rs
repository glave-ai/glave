//! The gateway's overhead per request, in-process (budget: under 1 ms at p99): the same
//! request sent straight to a fake provider and through the gateway, with two routes,
//! fallback targets and a default reasoning level to send, timed call by call. Fails when the gateway adds 1 ms or
//! more at p99. Then the same through a gateway that charges an organization's credits, on the
//! in-memory log: what holding and charging cost a call beyond the gateway's own work (a read
//! of what is new and one small append before the call, one append after), with no store's
//! round trips in it. Run it through `cargo xtask bench`, alone on an idle machine, since these
//! are wall-clock numbers.

#![expect(clippy::print_stdout, reason = "a benchmark: its table is its output")]

use std::error::Error;
use std::future::{Future, ready};
use std::sync::Arc;
use std::time::{Duration, Instant};

use glave_event::{
    Arguments, CallId, Currency, ModelRef, Money, Nanos, Part, Principal, PrincipalId, Reasoning,
    StopReason, TokenCounts, Usage,
};
use glave_gateway::{Credits, Gateway, Model, Price, Routes, Target};
use glave_ports::{
    Completion, Delta, InMemoryLog, Message, ModelProvider, ModelRequest, OrgId, OsEntropy, Payer,
    ProviderError, Purpose, SystemClock, ToolCall, ToolChoice, ToolSpec,
};

/// Calls timed each way.
const CALLS: usize = 20_000;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn main() -> Result<()> {
    let answer = answer()?;
    let fake = Arc::new(Fake(answer));
    let gateway = gateway(&fake)?;
    let request = request()?;
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    let (paying, paid) = paying(&fake, &request, &runtime)?;
    let (direct, routed, charged) = runtime.block_on(async {
        // Warm every path first.
        for _ in 0..1_000 {
            fake.complete(&request, &mut |_| {}).await?;
            gateway.complete(&request, &mut |_| {}).await?;
            paying.complete(&paid, &mut |_| {}).await?;
        }
        let direct = time(&*fake, &request).await?;
        let routed = time(&gateway, &request).await?;
        let charged = time(&paying, &paid).await?;
        Ok::<_, ProviderError>((direct, routed, charged))
    })?;
    let overhead_p50 = percentile(&routed, 50).saturating_sub(percentile(&direct, 50));
    let overhead_p99 = percentile(&routed, 99).saturating_sub(percentile(&direct, 99));
    println!(
        "\ngateway overhead, {CALLS} calls each way, {} KB request",
        request_kb(&request)
    );
    println!("{:<22} {:>10} {:>10}", "", "p50", "p99");
    for (name, times) in [
        ("direct to the provider", &direct),
        ("through the gateway", &routed),
        ("paying credits", &charged),
    ] {
        println!(
            "{name:<22} {:>10} {:>10}",
            show(percentile(times, 50)),
            show(percentile(times, 99))
        );
    }
    println!(
        "{:<22} {:>10} {:>10}  (budget < 1 ms at p99)",
        "overhead",
        show(overhead_p50),
        show(overhead_p99)
    );
    println!(
        "{:<22} {:>10} {:>10}  (a hold and a charge, in memory)",
        "credits",
        show(percentile(&charged, 50).saturating_sub(percentile(&routed, 50))),
        show(percentile(&charged, 99).saturating_sub(percentile(&routed, 99)))
    );
    if overhead_p99 >= Duration::from_millis(1) {
        return Err("the gateway's overhead is over its budget".into());
    }
    Ok(())
}

/// Every call's time, sorted.
async fn time<P: ModelProvider>(
    provider: &P,
    request: &ModelRequest,
) -> std::result::Result<Vec<Duration>, ProviderError> {
    let mut times = Vec::with_capacity(CALLS);
    for _ in 0..CALLS {
        let started = Instant::now();
        provider.complete(request, &mut |_| {}).await?;
        times.push(started.elapsed());
    }
    times.sort();
    Ok(times)
}

fn percentile(sorted: &[Duration], percent: usize) -> Duration {
    let index = sorted.len().saturating_mul(percent) / 100;
    sorted
        .get(index.min(sorted.len().saturating_sub(1)))
        .copied()
        .unwrap_or_default()
}

fn show(duration: Duration) -> String {
    format!("{:.1} µs", duration.as_secs_f64() * 1_000_000.0)
}

fn request_kb(request: &ModelRequest) -> usize {
    serde_json::to_vec(request).map_or(0, |json| json.len() / 1_024)
}

/// A provider that answers at once, so only the gateway's own work is measured.
struct Fake(Completion);

impl ModelProvider for Fake {
    fn complete(
        &self,
        _request: &ModelRequest,
        _on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> impl Future<Output = std::result::Result<Completion, ProviderError>> + Send {
        ready(Ok(self.0.clone()))
    }
}

/// Main with a fallback target and a judge route of its own; no credits are charged.
/// Main's catalog names its reasoning levels and default, as most of OpenRouter's do, so a call
/// that asks for none is sent the default.
fn gateway(fake: &Arc<Fake>) -> Result<Gateway> {
    let handle: Arc<dyn Model> = Arc::clone(fake) as Arc<dyn Model>;
    let target = |model: &str| Target::new(Arc::clone(&handle), "bench", model);
    let reasoning = Reasoning {
        efforts: ["high", "medium", "low", "none"]
            .map(str::to_owned)
            .to_vec(),
        default: Some("medium".into()),
        mandatory: false,
    };
    let main = target("bench-model").with_reasoning(&reasoning);
    let routes = Routes::new(vec![main, target("fallback")])?
        .with_route(Purpose::Judge, vec![target("judge")])?;
    Ok(Gateway::new(routes))
}

/// A gateway like [`gateway`]'s whose main target has a price, charging an organization's
/// credits on the in-memory log, with plenty added; and the request, as that organization's.
fn paying(
    fake: &Arc<Fake>,
    request: &ModelRequest,
    runtime: &tokio::runtime::Runtime,
) -> Result<(Gateway, ModelRequest)> {
    let handle: Arc<dyn Model> = Arc::clone(fake) as Arc<dyn Model>;
    let price = Price {
        list: "bench".into(),
        currency: Currency::USD,
        input_per_mtok: Nanos::new(1_000),
        cache_read_per_mtok: Nanos::new(100),
        output_per_mtok: Nanos::new(1_000),
    };
    let target = Target::new(handle, "bench", "bench-model").with_price(price);
    let credits = Arc::new(Credits::new(
        InMemoryLog::new(),
        Box::new(SystemClock),
        Box::new(OsEntropy),
        None,
    ));
    let person = PrincipalId::new(jiff::Timestamp::UNIX_EPOCH, [1; 10])?;
    let org = OrgId::from_uuid(PrincipalId::new(jiff::Timestamp::UNIX_EPOCH, [2; 10])?.as_uuid());
    let plenty = Money {
        nanos: Nanos::new(i64::MAX / 2),
        currency: Currency::USD,
    };
    let by = Principal::person(person);
    runtime.block_on(credits.add(org, plenty, "bench", by))?;
    let gateway = Gateway::new(Routes::new(vec![target])?).with_credits(credits);
    let paid = ModelRequest {
        payer: Some(Payer {
            org,
            person: Some(person),
        }),
        ..request.clone()
    };
    Ok((gateway, paid))
}

/// A mid-sized coding turn: a long system prompt, ten tools and sixty messages.
fn request() -> Result<ModelRequest> {
    let tools = (0..10)
        .map(|n| ToolSpec {
            name: format!("tool_{n}"),
            description: "Does one thing to the workspace and reports what it did.".repeat(4),
            parameters: serde_json::json!({"type": "object", "properties": {
                "path": {"type": "string"}, "content": {"type": "string"}}}),
        })
        .collect();
    let mut messages = Vec::new();
    for n in 0..20 {
        let mut arguments = serde_json::Map::new();
        arguments.insert("path".into(), format!("src/file_{n}.rs").into());
        let call_id = CallId::new(format!("call_{n}"))?;
        messages.push(Message::User {
            parts: vec![Part::Text {
                text: "Please look at the next file and fix what is wrong. ".repeat(10),
            }],
        });
        messages.push(Message::Assistant {
            parts: vec![Part::Text {
                text: "Reading it.".into(),
            }],
            tool_calls: vec![ToolCall {
                call_id: call_id.clone(),
                tool: "tool_0".into(),
                arguments: Arguments::Json(arguments),
            }],
        });
        messages.push(Message::Tool {
            call_id,
            is_error: false,
            output: vec![Part::Text {
                text: "fn main() { println!(\"hello\"); }\n".repeat(60),
            }],
        });
    }
    Ok(ModelRequest {
        model: "bench-model".into(),
        system: "You are a careful coding agent. ".repeat(250),
        messages,
        tools,
        tool_choice: ToolChoice::Auto,
        max_output_tokens: Some(8_192),
        purpose: Purpose::Main,
        files: Vec::new(),
        effort: None,
        payer: None,
        route: None,
    })
}

fn answer() -> Result<Completion> {
    let counts = TokenCounts {
        input: 30_000,
        cache_read: 28_000,
        output: 200,
        ..TokenCounts::default()
    };
    Ok(Completion {
        model: ModelRef {
            provider: "bench".into(),
            name: "bench-model".into(),
        },
        response_id: None,
        parts: vec![Part::Text {
            text: "Fixed it.".into(),
        }],
        tool_calls: Vec::new(),
        stop: StopReason::EndTurn,
        provider_stop: None,
        usage: Usage::new(counts, false, None)?,
        cost: None,
        served: None,
        upstream: None,
    })
}
