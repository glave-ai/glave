//! The gateway's overhead per request, in-process (budget: under 1 ms at p99): the same
//! request sent straight to a fake provider and through the gateway, with two routes,
//! fallback targets and budgets on, timed call by call. Fails when the gateway adds 1 ms or
//! more at p99. Run it through `cargo xtask bench`, alone on an idle machine, since these are
//! wall-clock numbers.

#![expect(clippy::print_stdout, reason = "a benchmark: its table is its output")]

use std::error::Error;
use std::future::{Future, ready};
use std::sync::Arc;
use std::time::{Duration, Instant};

use glave_event::{Arguments, CallId, ModelRef, Part, StopReason, ThreadId, TokenCounts, Usage};
use glave_gateway::{Budgets, Gateway, Limit, Model, Routes, Target};
use glave_ports::{
    Clock as _, Completion, Delta, Message, ModelProvider, ModelRequest, ProviderError, Purpose,
    SystemClock, ToolCall, ToolChoice, ToolSpec,
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
    let (direct, routed) = runtime.block_on(async {
        // Warm both paths first.
        for _ in 0..1_000 {
            fake.complete(&request, &mut |_| {}).await?;
            gateway.complete(&request, &mut |_| {}).await?;
        }
        let direct = time(&*fake, &request).await?;
        let routed = time(&gateway, &request).await?;
        Ok::<_, ProviderError>((direct, routed))
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

/// Main with a fallback target, a judge route of its own, and every budget on.
fn gateway(fake: &Arc<Fake>) -> Result<Gateway> {
    let handle: Arc<dyn Model> = Arc::clone(fake) as Arc<dyn Model>;
    let target = |model: &str| Target::new(Arc::clone(&handle), "bench", model);
    let routes = Routes::new(vec![target("bench-model"), target("fallback")])?
        .with_route(Purpose::Judge, vec![target("judge")])?;
    let plenty = Some(Limit::Tokens(u64::MAX / 4));
    Ok(Gateway::new(routes).with_budgets(Budgets {
        organization: plenty,
        person: plenty,
        thread: plenty,
    }))
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
    let thread = ThreadId::new(SystemClock.now(), [7; 10])?;
    Ok(ModelRequest {
        model: "bench-model".into(),
        system: "You are a careful coding agent. ".repeat(250),
        messages,
        tools,
        tool_choice: ToolChoice::Auto,
        max_output_tokens: Some(8_192),
        purpose: Purpose::Main,
        threads: vec![thread],
        files: Vec::new(),
        effort: None,
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
