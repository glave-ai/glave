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
//! the client uploading a workspace; (2) API keys map to tenant ids via a static
//! env-backed store — the [`ApiKeyStore`] seam makes a real account DB a clean swap.

use std::convert::Infallible;
use std::io::Write;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::body::Body;
use axum::extract::{Json, Request, State};
use axum::http::header::{
    ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN,
    ACCESS_CONTROL_MAX_AGE, AUTHORIZATION, COOKIE, SET_COOKIE,
};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;

use glave_core::engine::{AgentSpec, RunEngine, RunOutcome, RunRequest};
use glave_core::{
    resolve_login, AccountStore, Message, ProviderFactory, Role, RunEvent, TaskSpec, TraceSink,
    User,
};

pub mod pg;

/// Build the server and run it to completion on a multi-threaded runtime.
/// Blocks. Fails fast if the bearer token isn't configured.
pub fn serve_blocking(
    addr: SocketAddr,
    engine: RunEngine,
    providers: Arc<dyn ProviderFactory>,
) -> Result<()> {
    let keys: Arc<dyn ApiKeyStore> = Arc::new(StaticKeyStore::from_env()?);
    let meter = build_meter();
    let accounts = build_accounts()?;
    let runtime = tokio::runtime::Runtime::new().context("starting tokio runtime")?;
    runtime.block_on(run(addr, engine, providers, keys, meter, accounts))
}

/// Build the accounts store from `GLAVE_DATABASE_URL` (and run migrations). When
/// unset, auth is simply unavailable (the auth endpoints return 503) — the rest
/// of the server still runs.
fn build_accounts() -> Result<Option<Arc<pg::PgAccountStore>>> {
    let url = std::env::var("GLAVE_DATABASE_URL").unwrap_or_default();
    if url.is_empty() {
        return Ok(None);
    }
    let store = pg::PgAccountStore::connect(&url).context("connecting accounts DB")?;
    store.migrate().context("migrating accounts DB")?;
    Ok(Some(Arc::new(store)))
}

#[derive(Clone)]
struct AppState {
    engine: RunEngine,
    providers: Arc<dyn ProviderFactory>,
    keys: Arc<dyn ApiKeyStore>,
    meter: Arc<dyn MeterSink>,
    accounts: Option<Arc<pg::PgAccountStore>>,
}

async fn run(
    addr: SocketAddr,
    engine: RunEngine,
    providers: Arc<dyn ProviderFactory>,
    keys: Arc<dyn ApiKeyStore>,
    meter: Arc<dyn MeterSink>,
    accounts: Option<Arc<pg::PgAccountStore>>,
) -> Result<()> {
    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/runs", post(run_handler))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/auth/otp/start", post(auth_otp_start))
        .route("/auth/otp/verify", post(auth_otp_verify))
        .route("/auth/me", get(auth_me))
        .route("/auth/logout", post(auth_logout))
        .layer(middleware::from_fn(cors_mw))
        .with_state(AppState {
            engine,
            providers,
            keys,
            meter,
            accounts,
        });
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    eprintln!("glave serving on http://{addr}");
    axum::serve(listener, app).await.context("serving")
}

async fn health() -> &'static str {
    "ok"
}

/// Permissive CORS so the browser web app can call the API cross-origin. `*` is
/// fine here because auth is a bearer token, not cookies. Tighten the allowed
/// origin via config for production.
async fn cors_mw(req: Request, next: Next) -> Response {
    if req.method() == Method::OPTIONS {
        let mut resp = Response::new(Body::empty());
        *resp.status_mut() = StatusCode::NO_CONTENT;
        apply_cors(resp.headers_mut());
        return resp;
    }
    let mut resp = next.run(req).await;
    apply_cors(resp.headers_mut());
    resp
}

fn apply_cors(headers: &mut HeaderMap) {
    headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    headers.insert(
        ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    headers.insert(
        ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("authorization, content-type"),
    );
    headers.insert(ACCESS_CONTROL_MAX_AGE, HeaderValue::from_static("86400"));
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
            AgentKind::Model => AgentSpec::Model {
                model: model.to_string(),
            },
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
) -> Result<Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>>, (StatusCode, String)>
{
    authenticate(&headers, state.keys.as_ref())?;

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

/// Port: resolves a bearer token to its tenant/account id. The static env-backed
/// impl below is v0; a real account DB is a clean swap behind this seam.
trait ApiKeyStore: Send + Sync {
    fn resolve(&self, token: &str) -> Option<String>;
}

/// v0 key store: `(key, tenant)` pairs from the environment. `GLAVE_API_KEYS`
/// carries comma-separated `key:tenant` pairs (a bare `key` maps to `default`);
/// the legacy single `GLAVE_API_KEY` is still honored as `default`.
struct StaticKeyStore {
    entries: Vec<(String, String)>,
}

impl StaticKeyStore {
    fn from_env() -> Result<Self> {
        let mut entries = Vec::new();
        if let Ok(raw) = std::env::var("GLAVE_API_KEYS") {
            for pair in raw.split(',') {
                let pair = pair.trim();
                if pair.is_empty() {
                    continue;
                }
                match pair.split_once(':') {
                    Some((k, t)) => entries.push((k.trim().to_string(), t.trim().to_string())),
                    None => entries.push((pair.to_string(), "default".to_string())),
                }
            }
        }
        if let Ok(k) = std::env::var("GLAVE_API_KEY") {
            if !k.is_empty() {
                entries.push((k, "default".to_string()));
            }
        }
        if entries.is_empty() {
            anyhow::bail!(
                "no API keys configured — set GLAVE_API_KEYS (key:tenant,…) or GLAVE_API_KEY"
            );
        }
        Ok(Self { entries })
    }
}

impl ApiKeyStore for StaticKeyStore {
    fn resolve(&self, token: &str) -> Option<String> {
        self.entries
            .iter()
            .find_map(|(key, tenant)| ct_eq(token, key).then(|| tenant.clone()))
    }
}

/// Authenticate the request: pull the bearer token and resolve it to a tenant id,
/// or 401. Constant-time key compare (length leak only, which is standard).
fn authenticate(
    headers: &HeaderMap,
    keys: &dyn ApiKeyStore,
) -> Result<String, (StatusCode, String)> {
    let token = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));
    match token.and_then(|t| keys.resolve(t)) {
        Some(tenant) => Ok(tenant),
        None => Err((
            StatusCode::UNAUTHORIZED,
            "missing or invalid bearer token".to_string(),
        )),
    }
}

/// Port: a durable sink for per-request usage — the billing substrate. Best-effort
/// (a meter write never fails the user's request).
trait MeterSink: Send + Sync {
    fn record(&self, event: &MeterEvent);
}

/// One billable unit of usage, keyed to the tenant.
#[derive(Serialize)]
struct MeterEvent {
    id: String,
    tenant: String,
    model: String,
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
    cost_usd: f64,
    created: u64,
}

/// No-op meter (metering not configured). Usage still appears in the request log.
struct NullMeter;
impl MeterSink for NullMeter {
    fn record(&self, _event: &MeterEvent) {}
}

/// v0 meter: one JSON line per request appended to a file. A DB-backed meter is a
/// clean swap behind [`MeterSink`].
struct JsonlMeter {
    file: std::sync::Mutex<std::fs::File>,
}

impl JsonlMeter {
    fn open(path: &str) -> Result<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .with_context(|| format!("opening meter file {path}"))?;
        Ok(Self {
            file: std::sync::Mutex::new(file),
        })
    }
}

impl MeterSink for JsonlMeter {
    fn record(&self, event: &MeterEvent) {
        let line = match serde_json::to_string(event) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("meter: encode failed: {e}");
                return;
            }
        };
        if let Ok(mut f) = self.file.lock() {
            if let Err(e) = writeln!(f, "{line}") {
                eprintln!("meter: write failed: {e}");
            }
        }
    }
}

/// Build the meter from the env: durable JSONL when `GLAVE_METER_FILE` is set,
/// otherwise a no-op (usage still shows in the request log).
fn build_meter() -> Arc<dyn MeterSink> {
    let path = std::env::var("GLAVE_METER_FILE").unwrap_or_default();
    if path.is_empty() {
        return Arc::new(NullMeter);
    }
    match JsonlMeter::open(&path) {
        Ok(m) => return Arc::new(m),
        Err(e) => eprintln!("metering disabled — cannot open meter file: {e}"),
    }
    Arc::new(NullMeter)
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

// ---- OpenAI-compatible chat completions (`POST /v1/chat/completions`) --------
//
// The commodity / distribution surface: any OpenAI client works by pointing its
// `base_url` here. We serve ONLY Hope (`model` must be a `hope*` id) — this is
// our model reaching every harness, never a proxy for arbitrary models. v0 is
// plain chat (no tool-calling yet); the wire model the alias resolves to is
// OpenRouter qwen today and our own endpoint once `GLAVE_MODEL` points at it.

#[derive(Deserialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(default)]
    stream: bool,
    // Other OpenAI params (temperature, max_tokens, …) are accepted and ignored:
    // serde drops unknown fields, keeping us wire-compatible.
}

#[derive(Deserialize)]
struct ChatMessage {
    role: Role,
    #[serde(default)]
    content: String,
}

#[derive(Serialize)]
struct ChatResponse {
    id: String,
    object: &'static str,
    created: u64,
    model: String,
    choices: Vec<ChatChoice>,
    usage: Usage,
}

#[derive(Serialize)]
struct ChatChoice {
    index: u32,
    message: ChatRespMessage,
    finish_reason: &'static str,
}

#[derive(Serialize)]
struct ChatRespMessage {
    role: &'static str,
    content: String,
}

#[derive(Serialize)]
struct Usage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
}

#[derive(Serialize)]
struct ChatChunk {
    id: String,
    object: &'static str,
    created: u64,
    model: String,
    choices: Vec<ChatChunkChoice>,
}

#[derive(Serialize)]
struct ChatChunkChoice {
    index: u32,
    delta: ChatRespMessage,
    finish_reason: &'static str,
}

/// Public model ids this endpoint accepts. We serve ONLY Hope, so anything else
/// is a 400 — we never become a pass-through proxy for arbitrary models. The
/// aliases all resolve to one wire model today; pinned versions diverge once we
/// ship more than one.
fn resolve_hope_model(public: &str) -> Result<String, (StatusCode, String)> {
    match public {
        "hope" | "hope-latest" | "hope-1" => Ok(hope_wire_model()),
        other => Err((
            StatusCode::BAD_REQUEST,
            format!("unknown model '{other}': this endpoint serves hope, hope-latest, hope-1"),
        )),
    }
}

/// The wire model the Hope alias resolves to: OpenRouter qwen by default (the v0
/// bridge), or our own `glave/…` endpoint when `GLAVE_MODEL` is set.
fn hope_wire_model() -> String {
    std::env::var("GLAVE_MODEL").unwrap_or_else(|_| "qwen/qwen3.6-27b".to_string())
}

fn created_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Hope's v0 character, applied server-side so she sounds like Hope from any
/// client. This is the interim prompt-fusion: it encodes BOTH brevity (the cost
/// lever) and persona (the delight lever) — the same two things the LoRA will
/// later bake into the weights.
const HOPE_SYSTEM_PROMPT: &str = "\
You are Hope, made by Glave.

Voice: warm, sharp, and genuinely human — dry wit welcome, never a performer. \
You talk like a smart friend who respects the other person's time, not a corporate \
assistant. No flattery, no filler, no \"great question.\" You have range: funny, \
direct, or thoughtful as the moment calls for.

How you work:
- Lead with the answer. Add detail only if it earns its place — brevity is respect.
- Say what you're sure of plainly; flag what you're not. Never fake confidence.
- You empower the person, you don't replace them: give them what makes them better, then get out of the way.
- When you're wrong, say so and fix it. When they're wrong, say so kindly and clearly.
- Match their level: terse for experts, fuller for beginners.

You'd rather be useful and brief than thorough and exhausting.";

/// Build the provider message list: Hope's system prompt first, then the client's
/// messages. (v0 always prepends; once it's in the LoRA it's intrinsic anyway.)
fn build_messages(incoming: Vec<ChatMessage>) -> Vec<Message> {
    let mut messages = Vec::with_capacity(incoming.len() + 1);
    messages.push(Message::system(HOPE_SYSTEM_PROMPT));
    for m in incoming {
        messages.push(Message {
            role: m.role,
            content: m.content,
            tool_calls: Vec::new(),
            tool_call_id: None,
        });
    }
    messages
}

/// `POST /v1/chat/completions` — OpenAI-compatible. Serves Hope only. Returns a
/// single completion (JSON), or, when `stream: true`, the same completion as one
/// SSE chunk followed by `[DONE]` (v0 doesn't stream tokens incrementally yet —
/// the provider port is non-streaming — but the wire shape is compatible).
async fn chat_completions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ChatRequest>,
) -> Result<Response, (StatusCode, String)> {
    let tenant = authenticate(&headers, state.keys.as_ref())?;
    let wire = resolve_hope_model(&body.model)?;
    let public_model = body.model;
    let stream = body.stream;
    let messages = build_messages(body.messages);

    // `ModelProvider` is synchronous and not `Send`, so create AND call it
    // entirely inside the blocking closure — the trait object never crosses an
    // await point. The factory (Arc, Send + Sync) and the messages do.
    let providers = state.providers.clone();
    let completion = tokio::task::spawn_blocking(move || -> Result<_> {
        let provider = providers.create(&wire)?;
        provider.complete(&messages)
    })
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("model task panicked: {e}"),
        )
    })?
    .map_err(|e| (StatusCode::BAD_GATEWAY, format!("model error: {e}")))?;

    eprintln!(
        "chat.completions tenant={tenant} model={public_model} prompt={} completion={} cost=${:.4}",
        completion.prompt_tokens, completion.completion_tokens, completion.cost_usd
    );

    let id = format!("chatcmpl-{}", uuid::Uuid::new_v4());
    let created = created_unix();
    let total = completion.prompt_tokens + completion.completion_tokens;

    state.meter.record(&MeterEvent {
        id: id.clone(),
        tenant: tenant.clone(),
        model: public_model.clone(),
        prompt_tokens: completion.prompt_tokens,
        completion_tokens: completion.completion_tokens,
        total_tokens: total,
        cost_usd: completion.cost_usd,
        created,
    });

    if stream {
        let chunk = ChatChunk {
            id,
            object: "chat.completion.chunk",
            created,
            model: public_model,
            choices: vec![ChatChunkChoice {
                index: 0,
                delta: ChatRespMessage {
                    role: "assistant",
                    content: completion.content,
                },
                finish_reason: "stop",
            }],
        };
        let chunk_event = Event::default().json_data(&chunk).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("encoding chunk: {e}"),
            )
        })?;
        let events = vec![
            Ok::<Event, Infallible>(chunk_event),
            Ok::<Event, Infallible>(Event::default().data("[DONE]")),
        ];
        Ok(Sse::new(tokio_stream::iter(events)).into_response())
    } else {
        let resp = ChatResponse {
            id,
            object: "chat.completion",
            created,
            model: public_model,
            choices: vec![ChatChoice {
                index: 0,
                message: ChatRespMessage {
                    role: "assistant",
                    content: completion.content,
                },
                finish_reason: "stop",
            }],
            usage: Usage {
                prompt_tokens: completion.prompt_tokens,
                completion_tokens: completion.completion_tokens,
                total_tokens: total,
            },
        };
        Ok(Json(resp).into_response())
    }
}

// ---- Auth: passwordless OTP login + sessions --------------------------------
//
// Wraps the PgAccountStore auth flow. Sessions are HttpOnly cookies. The store is
// sync, so each handler runs it on a blocking thread. Available only when
// GLAVE_DATABASE_URL is configured; otherwise these return 503.

const SESSION_COOKIE: &str = "glave_session";

#[derive(Deserialize)]
struct OtpStart {
    email: String,
}

#[derive(Deserialize)]
struct OtpVerify {
    email: String,
    code: String,
}

#[derive(Serialize)]
struct OtpStartResponse {
    sent: bool,
    /// Present only when GLAVE_DEV_OTP is set — lets dev skip email delivery.
    dev_code: Option<String>,
}

#[derive(Serialize)]
struct MeResponse {
    id: String,
    email: String,
}

fn internal<E: std::fmt::Display>(e: E) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

fn accounts_store(state: &AppState) -> Result<Arc<pg::PgAccountStore>, (StatusCode, String)> {
    state.accounts.clone().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "auth not configured (set GLAVE_DATABASE_URL)".to_string(),
    ))
}

fn session_token(headers: &HeaderMap) -> Option<String> {
    let cookies = headers.get(COOKIE)?.to_str().ok()?;
    cookies
        .split(';')
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == SESSION_COOKIE)
        .map(|(_, v)| v.to_string())
}

fn session_cookie(token: &str, max_age: u32) -> String {
    format!("{SESSION_COOKIE}={token}; HttpOnly; SameSite=Lax; Path=/; Max-Age={max_age}")
}

/// `POST /auth/otp/start` — mint a one-time code for the email. v0 logs it (and
/// returns it when `GLAVE_DEV_OTP` is set); real email delivery comes later.
async fn auth_otp_start(
    State(state): State<AppState>,
    Json(body): Json<OtpStart>,
) -> Result<Json<OtpStartResponse>, (StatusCode, String)> {
    let store = accounts_store(&state)?;
    let email = body.email;
    let code = tokio::task::spawn_blocking(move || store.issue_otp(&email))
        .await
        .map_err(internal)?
        .map_err(internal)?;
    eprintln!("auth: otp code (dev) = {code}");
    let dev_code = std::env::var("GLAVE_DEV_OTP").is_ok().then_some(code);
    Ok(Json(OtpStartResponse {
        sent: true,
        dev_code,
    }))
}

/// `POST /auth/otp/verify` — consume the code, resolve/create the user, mint a
/// session, and set the HttpOnly session cookie.
async fn auth_otp_verify(
    State(state): State<AppState>,
    Json(body): Json<OtpVerify>,
) -> Result<Response, (StatusCode, String)> {
    let store = accounts_store(&state)?;
    let OtpVerify { email, code } = body;
    let result = tokio::task::spawn_blocking(move || -> Result<Option<(User, String)>> {
        if !store.verify_otp(&email, &code)? {
            return Ok(None);
        }
        let user = resolve_login(store.as_ref(), "email", &email, &email, true)?;
        // Every user has a home: provision their personal account on first login
        // (idempotent, so returning logins are a no-op).
        store.ensure_personal_account(&user)?;
        let token = store.create_session(&user.id)?;
        Ok(Some((user, token)))
    })
    .await
    .map_err(internal)?
    .map_err(internal)?;

    match result {
        Some((user, token)) => {
            let mut resp = Json(MeResponse {
                id: user.id,
                email: user.email,
            })
            .into_response();
            let cookie =
                HeaderValue::from_str(&session_cookie(&token, 2_592_000)).map_err(internal)?;
            resp.headers_mut().insert(SET_COOKIE, cookie);
            Ok(resp)
        }
        None => Err((StatusCode::UNAUTHORIZED, "invalid or expired code".to_string())),
    }
}

/// `GET /auth/me` — resolve the session cookie to the current user.
async fn auth_me(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<MeResponse>, (StatusCode, String)> {
    let store = accounts_store(&state)?;
    let token =
        session_token(&headers).ok_or((StatusCode::UNAUTHORIZED, "no session".to_string()))?;
    let user = tokio::task::spawn_blocking(move || store.session_user(&token))
        .await
        .map_err(internal)?
        .map_err(internal)?;
    match user {
        Some(u) => Ok(Json(MeResponse {
            id: u.id,
            email: u.email,
        })),
        None => Err((StatusCode::UNAUTHORIZED, "session expired".to_string())),
    }
}

/// `POST /auth/logout` — end the session and clear the cookie.
async fn auth_logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, (StatusCode, String)> {
    let store = accounts_store(&state)?;
    if let Some(token) = session_token(&headers) {
        let _ = tokio::task::spawn_blocking(move || store.delete_session(&token)).await;
    }
    let mut resp = StatusCode::NO_CONTENT.into_response();
    let cookie = HeaderValue::from_str(&session_cookie("", 0)).map_err(internal)?;
    resp.headers_mut().insert(SET_COOKIE, cookie);
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_cookie_sets_and_parses() {
        let cookie = session_cookie("tok-123", 2_592_000);
        assert!(cookie.contains("glave_session=tok-123"));
        assert!(cookie.contains("HttpOnly"));
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_static("foo=bar; glave_session=tok-123; x=y"),
        );
        assert_eq!(session_token(&headers).as_deref(), Some("tok-123"));
        assert_eq!(session_token(&HeaderMap::new()), None);
    }

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
        assert!(matches!(
            AgentKind::Model.into_spec("m"),
            AgentSpec::Model { .. }
        ));
        assert!(matches!(AgentKind::Solve.into_spec("m"), AgentSpec::Solve));
        assert!(matches!(AgentKind::Noop.into_spec("m"), AgentSpec::Noop));
    }

    #[test]
    fn cors_headers_are_applied() {
        let mut h = HeaderMap::new();
        apply_cors(&mut h);
        assert_eq!(
            h.get(ACCESS_CONTROL_ALLOW_ORIGIN)
                .and_then(|v| v.to_str().ok()),
            Some("*")
        );
        assert!(h.get(ACCESS_CONTROL_ALLOW_METHODS).is_some());
        assert!(h.get(ACCESS_CONTROL_ALLOW_HEADERS).is_some());
        assert!(h.get(ACCESS_CONTROL_MAX_AGE).is_some());
    }

    #[test]
    fn resolve_hope_accepts_aliases_and_rejects_others() {
        assert!(resolve_hope_model("hope").is_ok());
        assert!(resolve_hope_model("hope-latest").is_ok());
        assert!(resolve_hope_model("hope-1").is_ok());
        // We serve only Hope — arbitrary models are rejected, never proxied.
        assert!(resolve_hope_model("gpt-4o").is_err());
        assert!(resolve_hope_model("qwen/qwen3.6-27b").is_err());
    }

    #[test]
    fn chat_request_parses_openai_shape_and_ignores_unknown_fields() {
        let json = r#"{"model":"hope","messages":[{"role":"system","content":"s"},{"role":"user","content":"hi"}],"temperature":0.7,"max_tokens":64,"stream":true}"#;
        let parsed = serde_json::from_str::<ChatRequest>(json);
        assert!(parsed.is_ok());
        if let Ok(req) = parsed {
            assert_eq!(req.model, "hope");
            assert_eq!(req.messages.len(), 2);
            assert!(req.stream);
        }
    }

    #[test]
    fn chat_response_serializes_to_openai_shape() {
        let resp = ChatResponse {
            id: "chatcmpl-x".to_string(),
            object: "chat.completion",
            created: 0,
            model: "hope".to_string(),
            choices: vec![ChatChoice {
                index: 0,
                message: ChatRespMessage {
                    role: "assistant",
                    content: "hello".to_string(),
                },
                finish_reason: "stop",
            }],
            usage: Usage {
                prompt_tokens: 3,
                completion_tokens: 2,
                total_tokens: 5,
            },
        };
        let val = serde_json::to_value(&resp);
        assert!(val.is_ok());
        if let Ok(v) = val {
            assert_eq!(
                v.get("object").and_then(|x| x.as_str()),
                Some("chat.completion")
            );
            let msg = v
                .get("choices")
                .and_then(|c| c.get(0))
                .and_then(|c| c.get("message"));
            assert_eq!(
                msg.and_then(|m| m.get("content")).and_then(|x| x.as_str()),
                Some("hello")
            );
            assert_eq!(
                msg.and_then(|m| m.get("role")).and_then(|x| x.as_str()),
                Some("assistant")
            );
            assert_eq!(
                v.get("usage")
                    .and_then(|u| u.get("total_tokens"))
                    .and_then(serde_json::Value::as_u64),
                Some(5)
            );
        }
    }

    #[test]
    fn static_key_store_resolves_keys_to_tenants() {
        let store = StaticKeyStore {
            entries: vec![
                ("sk-a".to_string(), "acct_1".to_string()),
                ("sk-b".to_string(), "acct_2".to_string()),
            ],
        };
        assert_eq!(store.resolve("sk-a").as_deref(), Some("acct_1"));
        assert_eq!(store.resolve("sk-b").as_deref(), Some("acct_2"));
        assert_eq!(store.resolve("sk-unknown"), None);
        assert_eq!(store.resolve(""), None);
    }

    #[test]
    fn build_messages_prepends_hope_persona() {
        let incoming = vec![ChatMessage {
            role: Role::User,
            content: "hi".to_string(),
        }];
        let msgs = build_messages(incoming);
        assert_eq!(msgs.len(), 2);
        if let Some(first) = msgs.first() {
            assert_eq!(first.role, Role::System);
            assert!(first.content.contains("Hope"));
        }
    }

    #[test]
    fn jsonl_meter_appends_a_usage_record() {
        let mut path = std::env::temp_dir();
        path.push(format!("glave_meter_test_{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let path_str = path.to_string_lossy().to_string();

        let meter = JsonlMeter::open(&path_str);
        assert!(meter.is_ok());
        if let Ok(m) = meter {
            m.record(&MeterEvent {
                id: "chatcmpl-x".to_string(),
                tenant: "acct_1".to_string(),
                model: "hope".to_string(),
                prompt_tokens: 1,
                completion_tokens: 2,
                total_tokens: 3,
                cost_usd: 0.01,
                created: 0,
            });
        }

        let contents = std::fs::read_to_string(&path);
        assert!(contents.is_ok());
        if let Ok(c) = contents {
            assert!(c.contains("\"tenant\":\"acct_1\""));
            assert!(c.contains("\"total_tokens\":3"));
        }
        let _ = std::fs::remove_file(&path);
    }
}
