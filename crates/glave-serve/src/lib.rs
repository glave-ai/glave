//! Glave serving layer — the user↔brain agent-run API.
//!
//! The brain (harness + gateway +, later, Hope) runs server-side; a client sends
//! a task + model and receives the run **streamed** as Server-Sent Events. This
//! is the boundary the product — and anyone building on us — wraps: they get an
//! agent *run*, never the harness itself. One [`RunEngine`] (Arc-backed,
//! `Send + Sync`) serves every concurrent request; each run executes on a
//! blocking thread and its trace events stream back over a channel.
//!
//! Two deliberate v1 simplifications, called out so they aren't mistaken for the
//! final shape: (1) the task is loaded from a server-side `task_dir` rather than
//! the client uploading a workspace; (2) auth is a single shared bearer token,
//! not per-tenant keys. Both are behind the auth wall and both are clean swaps.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::extract::{Json, State};
use axum::http::{header::AUTHORIZATION, HeaderMap, StatusCode};
use axum::response::sse::{Event, Sse};
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;

use glave_core::engine::{AgentSpec, RunEngine, RunOutcome, RunRequest};
use glave_core::{RunEvent, TaskSpec, TraceSink};

/// Build the server and run it to completion on a multi-threaded runtime.
/// Blocks. Fails fast if the bearer token isn't configured.
pub fn serve_blocking(addr: SocketAddr, engine: RunEngine) -> Result<()> {
    let api_key: Arc<str> = std::env::var("GLAVE_API_KEY")
        .context("GLAVE_API_KEY must be set — the bearer token clients authenticate with")?
        .into();
    let runtime = tokio::runtime::Runtime::new().context("starting tokio runtime")?;
    runtime.block_on(run(addr, engine, api_key))
}

#[derive(Clone)]
struct AppState {
    engine: RunEngine,
    api_key: Arc<str>,
}

async fn run(addr: SocketAddr, engine: RunEngine, api_key: Arc<str>) -> Result<()> {
    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/runs", post(run_handler))
        .with_state(AppState { engine, api_key });
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    eprintln!("glave serving on http://{addr}");
    axum::serve(listener, app).await.context("serving")
}

async fn health() -> &'static str {
    "ok"
}

#[derive(Deserialize)]
struct RunBody {
    /// Server-side path to the task directory to run. (v1: the workspace lives on
    /// the server; client-uploaded workspaces are a later enhancement.)
    task_dir: String,
    #[serde(default = "default_model")]
    model: String,
    #[serde(default)]
    agent: AgentKind,
}

fn default_model() -> String {
    "qwen/qwen3.6-27b".to_string()
}

/// Which agent the request runs. Defaults to the full harnessed agent; the stub
/// kinds are useful for exercising the wire without spending a model call.
#[derive(Deserialize, Default, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum AgentKind {
    #[default]
    Model,
    Solve,
    Noop,
}

impl AgentKind {
    fn into_spec(self, model: &str) -> AgentSpec {
        match self {
            AgentKind::Model => AgentSpec::Model { model: model.to_string() },
            AgentKind::Solve => AgentSpec::Solve,
            AgentKind::Noop => AgentSpec::Noop,
        }
    }
}

/// `POST /v1/runs` — run an agent task, streaming the trace as SSE. Events:
/// `trace` (each `RunEvent`), then a terminal `done` (the `RunOutcome`) or
/// `error`.
async fn run_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<RunBody>,
) -> Result<Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>>, (StatusCode, String)> {
    authorize(&headers, &state.api_key)?;

    let task = TaskSpec::load(Path::new(&body.task_dir))
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("loading task: {e}")))?;
    let req = RunRequest {
        run_id: uuid::Uuid::new_v4().to_string(),
        task,
        agent: body.agent.into_spec(&body.model),
    };

    // The engine is synchronous; run it on a blocking thread and stream its
    // trace events back through a channel. The client disconnecting just drops
    // the receiver — sends fail silently and the run finishes server-side.
    let (tx, rx) = tokio::sync::mpsc::channel::<ServerFrame>(64);
    let engine = state.engine.clone();
    tokio::task::spawn_blocking(move || {
        let mut sink = ChannelSink { tx: tx.clone() };
        let terminal = match engine.execute(&req, &mut sink) {
            Ok(outcome) => {
                eprintln!(
                    "run {} done: solved={} cost=${:.4}",
                    outcome.run_id, outcome.solved, outcome.cost_usd
                );
                ServerFrame::Done(outcome)
            }
            Err(e) => ServerFrame::Error(e.to_string()),
        };
        let _ = tx.blocking_send(terminal);
    });

    let stream = ReceiverStream::new(rx).map(|frame| Ok::<Event, Infallible>(frame.into_sse()));
    Ok(Sse::new(stream))
}

/// Constant-time bearer-token check (length leak only, which is standard).
fn authorize(headers: &HeaderMap, expected: &str) -> Result<(), (StatusCode, String)> {
    let token = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));
    match token {
        Some(t) if ct_eq(t, expected) => Ok(()),
        _ => Err((StatusCode::UNAUTHORIZED, "missing or invalid bearer token".to_string())),
    }
}

fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// What flows over the SSE channel.
enum ServerFrame {
    Event(RunEvent),
    Done(RunOutcome),
    Error(String),
}

impl ServerFrame {
    /// Render to an SSE event, using the SSE event-name to type the frame.
    fn into_sse(self) -> Event {
        let fallback = |name: &str| Event::default().event(name).data("{}");
        match self {
            ServerFrame::Event(e) => Event::default()
                .event("trace")
                .json_data(&e)
                .unwrap_or_else(|_| fallback("trace")),
            ServerFrame::Done(o) => Event::default()
                .event("done")
                .json_data(&o)
                .unwrap_or_else(|_| fallback("done")),
            ServerFrame::Error(m) => Event::default().event("error").data(m),
        }
    }
}

/// `TraceSink` that forwards each event onto the SSE channel from the blocking
/// run thread. A full/closed channel (slow or gone client) is ignored.
struct ChannelSink {
    tx: tokio::sync::mpsc::Sender<ServerFrame>,
}

impl TraceSink for ChannelSink {
    fn emit(&mut self, event: RunEvent) -> Result<()> {
        let _ = self.tx.blocking_send(ServerFrame::Event(event));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ct_eq_matches_only_equal_strings() {
        assert!(ct_eq("secret-token", "secret-token"));
        assert!(!ct_eq("secret-token", "secret-toked"));
        assert!(!ct_eq("secret", "secret-longer"));
        assert!(!ct_eq("", "x"));
        assert!(ct_eq("", ""));
    }

    #[test]
    fn agent_kind_maps_to_spec() {
        assert!(matches!(AgentKind::Model.into_spec("m"), AgentSpec::Model { .. }));
        assert!(matches!(AgentKind::Solve.into_spec("m"), AgentSpec::Solve));
        assert!(matches!(AgentKind::Noop.into_spec("m"), AgentSpec::Noop));
    }
}
