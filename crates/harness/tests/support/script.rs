//! Shared helpers for the harness's integration tests: scripted answers, and a harness wired
//! to the scripted model, a ticking clock and counting entropy over a real temporary folder.

use std::future::{Future, ready};

use glave_event::{
    Arguments, Body, Event, ModelRef, Part, RawEvent, StopReason, TokenCounts, Usage, decode,
};
use glave_harness::{ApprovalRequest, Approver, Decision, Harness, Parts, Settings, Workspace};
pub(crate) use glave_ports::block_on;
use glave_ports::testing::{CountingEntropy, ManualClock, TempDir};
pub(crate) use glave_ports::testing::{ScriptedModel, Seen};
use glave_ports::{
    Completion, EventLog, InMemoryBlobs, InMemoryLog, OrgId, ProjectId, RequestedCall, Scope,
};

pub(crate) struct Refuse;
impl Approver for Refuse {
    fn decide(&self, _request: &ApprovalRequest) -> impl Future<Output = Decision> + Send {
        ready(Decision::Deny)
    }
}

pub(crate) fn answer(
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
        upstream: None,
    }
}

pub(crate) fn folder(name: &str) -> TempDir {
    TempDir::new(&format!("turn-{name}")).unwrap()
}

pub(crate) fn scope() -> Scope {
    Scope::glaves_own(
        OrgId::from_uuid(uuid_like(1)),
        ProjectId::from_uuid(uuid_like(2)),
    )
}

pub(crate) fn uuid_like(n: u8) -> uuid::Uuid {
    uuid::Uuid::from_bytes([n; 16])
}

pub(crate) async fn harness<A: Approver>(
    dir: &std::path::Path,
    answers: Vec<Completion>,
    approver: A,
) -> Harness<ScriptedModel, InMemoryLog, InMemoryBlobs, A> {
    harness_on(dir, answers, approver, InMemoryLog::new(), 1, settings()).await
}

pub(crate) fn settings() -> Settings {
    Settings::new("test-model", "script")
}

/// Settings for the cautious mode, where changes and commands ask.
pub(crate) fn asking() -> Settings {
    let mut settings = settings();
    settings.approval = glave_harness::ApprovalMode::Ask;
    settings
}

/// A harness in the cautious mode.
pub(crate) async fn asking_harness<A: Approver>(
    dir: &std::path::Path,
    answers: Vec<Completion>,
    approver: A,
) -> Harness<ScriptedModel, InMemoryLog, InMemoryBlobs, A> {
    harness_on(dir, answers, approver, InMemoryLog::new(), 1, asking()).await
}

/// A harness on a given log. `seed` keeps ids distinct between harnesses sharing a log.
pub(crate) async fn harness_on<L: EventLog, A: Approver>(
    dir: &std::path::Path,
    answers: Vec<Completion>,
    approver: A,
    log: L,
    seed: u64,
    settings: Settings,
) -> Harness<ScriptedModel, L, InMemoryBlobs, A> {
    harness_seeing(dir, answers, approver, log, seed, settings)
        .await
        .0
}

/// Like `harness_on`, also returning what the model is sent.
pub(crate) async fn harness_seeing<L: EventLog, A: Approver>(
    dir: &std::path::Path,
    answers: Vec<Completion>,
    approver: A,
    log: L,
    seed: u64,
    settings: Settings,
) -> (Harness<ScriptedModel, L, InMemoryBlobs, A>, Seen) {
    harness_with(workspace(dir).await, answers, approver, log, seed, settings)
}

/// Like `harness_seeing`, on a given workspace.
pub(crate) fn harness_with<L: EventLog, A: Approver>(
    workspace: Workspace,
    answers: Vec<Completion>,
    approver: A,
    log: L,
    seed: u64,
    settings: Settings,
) -> (Harness<ScriptedModel, L, InMemoryBlobs, A>, Seen) {
    let script = ScriptedModel::new(answers);
    let requests = script.seen();
    let parts = Parts {
        provider: script,
        log,
        blobs: InMemoryBlobs::new(),
        approver,
        clock: Box::new(ManualClock::ticking(1_791_000_000_000)),
        entropy: Box::new(CountingEntropy::starting_at(seed)),
        workspace,
    };
    (Harness::new(parts, settings), requests)
}

/// `dir` on this machine as the tools' workspace, as the CLI opens the folder it starts in.
pub(crate) async fn workspace(dir: &std::path::Path) -> Workspace {
    let (computers, info) = glave_computer::LocalComputers::here(dir, std::env::vars_os())
        .await
        .unwrap();
    Workspace::new(computers, info)
}

pub(crate) async fn events<L: EventLog>(log: &L, thread: glave_event::ThreadId) -> Vec<Event> {
    let raw: Vec<RawEvent> = log.read(&scope(), thread, None, usize::MAX).await.unwrap();
    raw.into_iter().map(|e| decode(e).unwrap()).collect()
}

pub(crate) fn kinds(events: &[Event]) -> Vec<&'static str> {
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
            Body::ModelChosen(_) => "model.chosen",
            Body::ApprovalModeChosen(_) => "approval.mode.chosen",
            Body::EffortChosen(_) => "effort.chosen",
            Body::TurnEnded(_) => "turn.ended",
            Body::ApprovalRequested(_) => "approval.requested",
            Body::ApprovalDecided(_) => "approval.decided",
            Body::GoalSet(_) => "goal.set",
            Body::GoalEnded(_) => "goal.ended",
            Body::GoalResumed(_) => "goal.resumed",
            Body::WorkspaceBound(_) => "workspace.bound",
            Body::WorkspaceRemoved(_) => "workspace.removed",
            Body::CommandStarted(_) => "command.started",
            Body::CommandEnded(_) => "command.ended",
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
            Body::ProviderConnected(_) => "provider.connected",
            Body::ProviderRemoved(_) => "provider.removed",
            Body::CredentialConnected(_) => "credential.connected",
            Body::CredentialRemoved(_) => "credential.removed",
            Body::ProjectFolderSet(_) => "project.folder.set",
            Body::ProjectRemoved(_) => "project.removed",
            Body::TrustDecided(_) => "trust.decided",
            Body::BrowserSignedIn(_) => "browser.signed_in",
            Body::BrowserUsed(_) => "browser.used",
            Body::BrowserSignedOut(_) => "browser.signed_out",
            Body::CatalogRefreshed(_) => "catalog.refreshed",
            Body::AcmeRegistered(_) => "acme.registered",
            Body::CertificateIssued(_) => "certificate.issued",
            Body::Unknown(_) => "unknown",
        })
        .collect()
}
