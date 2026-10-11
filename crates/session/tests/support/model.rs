//! Shared helpers for the session tests: a scripted model, a ticking clock, counting
//! entropy, and a server wired to them over a real temporary folder.

use std::collections::VecDeque;
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use glave_computer::LocalComputers;
use glave_event::{Arguments, ModelRef, Part, StopReason, TokenCounts, Usage};
use glave_harness::{Clock, Entropy, Settings, Workspace};
use glave_ports::{
    Completion, Delta, EventLog, MemoryBlobs, MemoryLog, ModelProvider, ModelRequest, OrgId,
    ProviderError, RequestedCall, Scope, SpaceId,
};
use glave_session::{Answering, Server, Wiring};
use jiff::Timestamp;
use tokio::sync::Notify;

/// A model that answers from a script, optionally waiting at a gate before each answer.
pub struct Script {
    answers: Mutex<VecDeque<Completion>>,
    gate: Option<Arc<Notify>>,
    /// Every request received, as JSON.
    seen: Arc<Mutex<Vec<String>>>,
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

pub struct Ticking(AtomicI64);
impl Clock for Ticking {
    fn now(&self) -> Timestamp {
        Timestamp::from_millisecond(self.0.fetch_add(1, Ordering::SeqCst)).unwrap()
    }
}

pub struct Counting(AtomicU64);
impl Entropy for Counting {
    fn bytes(&self) -> [u8; 10] {
        let n = self.0.fetch_add(1, Ordering::SeqCst).to_be_bytes();
        [n[0], n[1], n[2], n[3], n[4], n[5], n[6], n[7], 0, 0]
    }
}

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

pub fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("glave-session-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub fn scope() -> Scope {
    Scope {
        org: OrgId::from_uuid(uuid::Uuid::from_u128(1)),
        space: SpaceId::from_uuid(uuid::Uuid::from_u128(2)),
    }
}

pub fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

pub async fn server_on<L: EventLog + 'static>(
    dir: &PathBuf,
    answers: Vec<Completion>,
    gate: Option<Arc<Notify>>,
    log: L,
    answering: Answering,
    seed: u64,
) -> Server<Script, L, MemoryBlobs> {
    // The cautious mode, so commands ask: these tests are about the questions.
    let mut settings = Settings::new("test-model", "script");
    settings.approval = glave_harness::ApprovalMode::Ask;
    server_with(dir, answers, gate, log, answering, seed, settings).await
}

/// Like `server_on`, with `settings`.
#[expect(
    clippy::too_many_arguments,
    reason = "a test helper that mirrors server_on"
)]
pub async fn server_with<L: EventLog + 'static>(
    dir: &PathBuf,
    answers: Vec<Completion>,
    gate: Option<Arc<Notify>>,
    log: L,
    answering: Answering,
    seed: u64,
    settings: Settings,
) -> Server<Script, L, MemoryBlobs> {
    build(dir, answers, gate, log, answering, seed, settings, None)
        .await
        .0
}

/// Like `server_with`, also giving every request the model receives (as JSON), and keeping
/// worktrees under `worktrees` when it is given.
#[expect(
    clippy::too_many_arguments,
    reason = "a test helper that mirrors server_with"
)]
pub async fn build<L: EventLog + 'static>(
    dir: &PathBuf,
    answers: Vec<Completion>,
    gate: Option<Arc<Notify>>,
    log: L,
    answering: Answering,
    seed: u64,
    settings: Settings,
    worktrees: Option<PathBuf>,
) -> (Server<Script, L, MemoryBlobs>, Arc<Mutex<Vec<String>>>) {
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let computers = match &worktrees {
        Some(base) => LocalComputers::for_here(std::env::vars_os()).with_worktrees(base),
        None => LocalComputers::for_here(std::env::vars_os()),
    };
    let wiring = Wiring {
        provider: Script {
            answers: Mutex::new(answers.into()),
            gate,
            seen: Arc::clone(&recorded),
        },
        log,
        blobs: MemoryBlobs::new(),
        clock: Box::new(Ticking(AtomicI64::new(
            1_791_000_000_000 + 1_000_000 * i64::try_from(seed).unwrap(),
        ))),
        entropy: Box::new(Counting(AtomicU64::new(seed << 32))),
        workspace: {
            let (computers, info) = computers.open_here(dir).await.unwrap();
            Workspace::new(computers, info)
        },
        tools: Vec::new(),
        worktree: false,
        hooks: glave_harness::Hooks::default(),
        worktrees: worktrees.map(|base| glave_session::cleanup::Cleaning {
            base,
            environment: std::env::vars_os().collect(),
            home: None,
            data_home: None,
        }),
    };
    (Server::new(wiring, settings, scope(), answering), recorded)
}

pub async fn server(
    dir: &PathBuf,
    answers: Vec<Completion>,
    answering: Answering,
) -> Server<Script, MemoryLog, MemoryBlobs> {
    server_on(dir, answers, None, MemoryLog::new(), answering, 1).await
}
