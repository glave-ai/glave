//! Shared helpers for the harness's integration tests: a scripted model, a ticking clock,
//! counting entropy, and a harness wired to them over a real temporary folder.

use std::collections::VecDeque;
use std::future::{Future, ready};
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use glave_event::{
    Arguments, Body, Event, ModelRef, Part, RawEvent, StopReason, TokenCounts, Usage, decode,
};
use glave_harness::{
    ApprovalRequest, Approver, Clock, Decision, Entropy, Harness, Parts, Settings, Workspace,
};
use glave_ports::{
    Completion, Delta, EventLog, MemoryBlobs, MemoryLog, ModelProvider, ModelRequest, OrgId,
    ProviderError, RequestedCall, Scope, SpaceId,
};
use jiff::Timestamp;

pub struct Script {
    pub answers: Mutex<VecDeque<Completion>>,
    pub seen: Arc<Mutex<Vec<ModelRequest>>>,
}

impl ModelProvider for Script {
    fn complete(
        &self,
        request: &ModelRequest,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> impl Future<Output = Result<Completion, ProviderError>> + Send {
        self.seen.lock().unwrap().push(request.clone());
        let next = self.answers.lock().unwrap().pop_front();
        if let Some(Part::Text { text }) = next.as_ref().and_then(|c| c.parts.first()) {
            on_delta(Delta::Text(text.clone()));
        }
        // Not a transient error, so a test that runs out never waits through retries.
        ready(next.ok_or_else(|| ProviderError::Protocol("script exhausted".into())))
    }
}

pub struct Ticking(pub AtomicI64);
impl Clock for Ticking {
    fn now(&self) -> Timestamp {
        let ms = self.0.fetch_add(1, Ordering::SeqCst);
        Timestamp::from_millisecond(ms).unwrap()
    }
}

pub struct Counting(pub AtomicU64);
impl Entropy for Counting {
    fn bytes(&self) -> [u8; 10] {
        let n = self.0.fetch_add(1, Ordering::SeqCst).to_be_bytes();
        [n[0], n[1], n[2], n[3], n[4], n[5], n[6], n[7], 0, 0]
    }
}

pub struct Refuse;
impl Approver for Refuse {
    fn decide(&self, _request: &ApprovalRequest) -> impl Future<Output = Decision> + Send {
        ready(Decision::Deny)
    }
}

pub fn answer(
    text: Option<&str>,
    calls: Vec<(&str, serde_json::Value)>,
    stop: StopReason,
) -> Completion {
    let counts = TokenCounts {
        input: 100,
        cache_read: 40,
        cache_write: 0,
        output: 10,
        reasoning: 0,
    };
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
        tool_calls: calls
            .into_iter()
            .map(|(tool, args)| RequestedCall {
                provider_call_id: Some(format!("p_{tool}")),
                tool: tool.into(),
                arguments: Arguments::Json(args.as_object().unwrap().clone()),
            })
            .collect(),
        stop,
        provider_stop: None,
        usage: Usage::new(counts, false, None).unwrap(),
        cost: None,
        served: None,
    }
}

pub fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("glave-turn-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub fn scope() -> Scope {
    Scope {
        org: OrgId::from_uuid(uuid_like(1)),
        space: SpaceId::from_uuid(uuid_like(2)),
    }
}

pub fn uuid_like(n: u8) -> uuid::Uuid {
    uuid::Uuid::from_bytes([n; 16])
}

pub fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

pub async fn harness<A: Approver>(
    dir: &std::path::Path,
    answers: Vec<Completion>,
    approver: A,
) -> Harness<Script, MemoryLog, MemoryBlobs, A> {
    harness_on(dir, answers, approver, MemoryLog::new(), 1, settings()).await
}

pub fn settings() -> Settings {
    Settings::new("test-model", "script")
}

/// Settings for the cautious mode, where changes and commands ask.
pub fn asking() -> Settings {
    let mut settings = settings();
    settings.approval = glave_harness::ApprovalMode::Ask;
    settings
}

/// A harness in the cautious mode.
pub async fn asking_harness<A: Approver>(
    dir: &std::path::Path,
    answers: Vec<Completion>,
    approver: A,
) -> Harness<Script, MemoryLog, MemoryBlobs, A> {
    harness_on(dir, answers, approver, MemoryLog::new(), 1, asking()).await
}

/// A harness on a given log. `seed` keeps ids distinct between harnesses sharing a log.
pub async fn harness_on<L: EventLog, A: Approver>(
    dir: &std::path::Path,
    answers: Vec<Completion>,
    approver: A,
    log: L,
    seed: u64,
    settings: Settings,
) -> Harness<Script, L, MemoryBlobs, A> {
    harness_seeing(dir, answers, approver, log, seed, settings)
        .await
        .0
}

/// Every request the scripted model received, in order.
pub type Seen = Arc<Mutex<Vec<ModelRequest>>>;

/// Like `harness_on`, also returning what the model is sent.
pub async fn harness_seeing<L: EventLog, A: Approver>(
    dir: &std::path::Path,
    answers: Vec<Completion>,
    approver: A,
    log: L,
    seed: u64,
    settings: Settings,
) -> (Harness<Script, L, MemoryBlobs, A>, Seen) {
    harness_with(workspace(dir).await, answers, approver, log, seed, settings)
}

/// Like `harness_seeing`, on a given workspace.
pub fn harness_with<L: EventLog, A: Approver>(
    workspace: Workspace,
    answers: Vec<Completion>,
    approver: A,
    log: L,
    seed: u64,
    settings: Settings,
) -> (Harness<Script, L, MemoryBlobs, A>, Seen) {
    let requests: Seen = Arc::new(Mutex::new(Vec::new()));
    let script = Script {
        answers: Mutex::new(answers.into()),
        seen: Arc::clone(&requests),
    };
    let parts = Parts {
        provider: script,
        log,
        blobs: MemoryBlobs::new(),
        approver,
        clock: Box::new(Ticking(AtomicI64::new(1_791_000_000_000))),
        entropy: Box::new(Counting(AtomicU64::new(seed))),
        workspace,
    };
    (Harness::new(parts, settings), requests)
}

/// `dir` on this machine as the tools' workspace, as the CLI opens the folder it starts in.
pub async fn workspace(dir: &std::path::Path) -> Workspace {
    let (computers, info) = glave_computer::LocalComputers::here(dir, std::env::vars_os())
        .await
        .unwrap();
    Workspace::new(computers, info)
}

pub async fn events<L: EventLog>(log: &L, thread: glave_event::ThreadId) -> Vec<Event> {
    let raw: Vec<RawEvent> = log.read(&scope(), thread, None, usize::MAX).await.unwrap();
    raw.into_iter().map(|e| decode(e).unwrap()).collect()
}

pub fn kinds(events: &[Event]) -> Vec<&'static str> {
    events
        .iter()
        .map(|e| match e.body {
            Body::ThreadStarted(_) => "thread.started",
            Body::ThreadCompacted(_) => "thread.compacted",
            Body::ThreadArchived(_) => "thread.archived",
            Body::ThreadUnarchived(_) => "thread.unarchived",
            Body::ThreadDeleted(_) => "thread.deleted",
            Body::ThreadRenamed(_) => "thread.renamed",
            Body::TurnStarted(_) => "turn.started",
            Body::MessageAdded(_) => "message.added",
            Body::ToolCallRequested(_) => "tool.call.requested",
            Body::ToolCallCompleted(_) => "tool.call.completed",
            Body::ModelResponded(_) => "model.responded",
            Body::TurnEnded(_) => "turn.ended",
            Body::ApprovalRequested(_) => "approval.requested",
            Body::ApprovalDecided(_) => "approval.decided",
            Body::GoalSet(_) => "goal.set",
            Body::GoalEnded(_) => "goal.ended",
            Body::GoalResumed(_) => "goal.resumed",
            Body::WorkspaceBound(_) => "workspace.bound",
            Body::WorkspaceRemoved(_) => "workspace.removed",
            Body::HookRan(_) => "hook.ran",
            Body::TaskStarted(_) => "task.started",
            Body::TaskEnded(_) => "task.ended",
            Body::ScheduleCreated(_) => "schedule.created",
            Body::ScheduleDeleted(_) => "schedule.deleted",
            Body::GatewayToolCalled(_) => "gateway.tool.called",
            Body::MemorySaved(_) => "memory.saved",
            Body::MemoryEdited(_) => "memory.edited",
            Body::MemoryForgotten(_) => "memory.forgotten",
            Body::MemoryRestored(_) => "memory.restored",
            Body::MemoryTidied(_) => "memory.tidied",
            Body::Unknown(_) => "unknown",
        })
        .collect()
}
