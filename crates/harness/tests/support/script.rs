//! Shared helpers for the harness's integration tests: scripted answers, a harness wired to the
//! scripted model, a ticking clock and counting entropy over a real temporary folder, its
//! threads' turns, and what they said.

use std::future::{Future, ready};
use std::sync::Mutex;

use glave_event::{
    Arguments, Body, Event, ModelRef, Part, RawEvent, StopReason, TokenCounts, TurnResult, Usage,
    decode,
};
use glave_harness::{
    AllowAll, ApprovalRequest, Approver, Cancel, Decision, Harness, Parts, Place, Settings, Thread,
    Workspace,
};
pub(crate) use glave_ports::block_on;
use glave_ports::testing::{CountingEntropy, ManualClock, TempDir};
pub(crate) use glave_ports::testing::{ScriptedModel, Seen};
use glave_ports::{
    BlobStore, Caller, Completion, EventLog, InMemoryBlobs, InMemoryLog, Message, ModelProvider,
    OrgId, ProjectId, RequestedCall, Scope,
};

pub(crate) struct Refuse;
impl Approver for Refuse {
    fn decide(&self, _request: &ApprovalRequest) -> impl Future<Output = Decision> + Send {
        ready(Decision::Deny)
    }
}

/// Answers with each decision in turn, and remembers what it was asked.
pub(crate) struct Answers {
    decisions: Mutex<Vec<Decision>>,
    asked: Mutex<Vec<ApprovalRequest>>,
}

impl Answers {
    pub(crate) fn new(decisions: &[Decision]) -> Self {
        let mut decisions = decisions.to_vec();
        decisions.reverse();
        Self {
            decisions: Mutex::new(decisions),
            asked: Mutex::new(Vec::new()),
        }
    }

    /// What it was asked, in order.
    pub(crate) fn asked(&self) -> Vec<ApprovalRequest> {
        self.asked.lock().unwrap().clone()
    }
}

impl Approver for Answers {
    fn decide(&self, request: &ApprovalRequest) -> impl Future<Output = Decision> + Send {
        self.asked.lock().unwrap().push(request.clone());
        ready(
            self.decisions
                .lock()
                .unwrap()
                .pop()
                .unwrap_or(Decision::Deny),
        )
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

/// An answer that reads `path`.
pub(crate) fn reads(path: &str) -> Completion {
    answer(
        None,
        vec![("read", serde_json::json!({ "path": path }))],
        StopReason::ToolUse,
    )
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

/// Settings where nothing asks, risky actions included: Full access, since a test's commands run
/// without a sandbox unless it gives them one ([`in_the_sandbox`]).
pub(crate) fn settings() -> Settings {
    let mut settings = Settings::new("test-model", "script");
    settings.permissions = glave_harness::Permissions::FullAccess;
    settings
}

/// Settings for the cautious mode, Ask first, where changes and commands ask.
pub(crate) fn asking() -> Settings {
    let mut settings = settings();
    settings.permissions = glave_harness::Permissions::Ask;
    settings
}

/// Settings for glave's default mode, Auto: in the sandbox a test gives its workspace,
/// nothing inside it asks.
pub(crate) fn in_the_sandbox() -> Settings {
    let mut settings = settings();
    settings.permissions = glave_harness::Permissions::Auto;
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

/// A harness on `dir` that allows everything, whose model reads `a.txt` there and says
/// "Done.", and what the model is sent: the plain one a test compares another with.
pub(crate) async fn reading(
    dir: &std::path::Path,
) -> (
    Harness<ScriptedModel, InMemoryLog, InMemoryBlobs, AllowAll>,
    Seen,
) {
    std::fs::write(dir.join("a.txt"), "hi\n").unwrap();
    let answers = vec![
        reads("a.txt"),
        answer(Some("Done."), vec![], StopReason::EndTurn),
    ];
    harness_seeing(dir, answers, AllowAll, InMemoryLog::new(), 1, settings()).await
}

/// What the model of [`reading`]'s harness is sent over a turn in a new thread: what another
/// harness, configured some other way, must send byte for byte.
pub(crate) async fn sent_reading(dir: &std::path::Path) -> String {
    let (harness, seen) = reading(dir).await;
    let mut thread = harness.start_thread(scope()).await.unwrap();
    turn(&harness, &mut thread, "read it").await;
    format!("{:?}", seen.lock().unwrap())
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

/// A new thread on `harness` for the person, started where `place` says.
pub(crate) async fn thread_in<P, L, B, A>(harness: &Harness<P, L, B, A>, place: Place) -> Thread
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    harness
        .start_thread_in(scope(), &Caller::default(), place)
        .await
        .unwrap()
}

/// Runs one turn of `thread` on `prompt`, with nothing to cancel it and nothing watching.
pub(crate) async fn turn<P, L, B, A>(
    harness: &Harness<P, L, B, A>,
    thread: &mut Thread,
    prompt: &str,
) -> TurnResult
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    harness
        .run_turn(thread, prompt, &Cancel::new(), &mut |_| {})
        .await
        .unwrap()
}

pub(crate) async fn events<L: EventLog>(log: &L, thread: glave_event::ThreadId) -> Vec<Event> {
    let raw: Vec<RawEvent> = log.read(&scope(), thread, None, usize::MAX).await.unwrap();
    raw.into_iter().map(|e| decode(e).unwrap()).collect()
}

/// The first text of each tool result in `messages`, in order.
pub(crate) fn results(messages: &[Message]) -> Vec<String> {
    messages
        .iter()
        .filter_map(|message| match message {
            Message::Tool { output, .. } => output.iter().find_map(|part| match part {
                Part::Text { text } => Some(text.clone()),
                _ => None,
            }),
            _ => None,
        })
        .collect()
}

/// The text of `message`, its parts' text joined.
pub(crate) fn text_of(message: &Message) -> String {
    let parts = match message {
        Message::User { parts }
        | Message::Tool { output: parts, .. }
        | Message::Assistant { parts, .. } => parts,
    };
    parts
        .iter()
        .filter_map(|part| match part {
            Part::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
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
            Body::PermissionsChosen(_) => "permissions.chosen",
            Body::EffortChosen(_) => "effort.chosen",
            Body::TurnEnded(_) => "turn.ended",
            Body::ApprovalRequested(_) => "approval.requested",
            Body::ApprovalDecided(_) => "approval.decided",
            Body::GoalSet(_) => "goal.set",
            Body::GoalEnded(_) => "goal.ended",
            Body::GoalResumed(_) => "goal.resumed",
            Body::PlanUpdated(_) => "plan.updated",
            Body::PlanApproved(_) => "plan.approved",
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
            Body::ConnectionConnected(_) => "connection.connected",
            Body::ConnectionRemoved(_) => "connection.removed",
            Body::ProjectFolderSet(_) => "project.folder.set",
            Body::ProjectRemoved(_) => "project.removed",
            Body::TrustDecided(_) => "trust.decided",
            Body::BrowserSignedIn(_) => "browser.signed_in",
            Body::BrowserUsed(_) => "browser.used",
            Body::BrowserSignedOut(_) => "browser.signed_out",
            Body::CatalogRefreshed(_) => "catalog.refreshed",
            Body::AcmeRegistered(_) => "acme.registered",
            Body::CertificateIssued(_) => "certificate.issued",
            Body::PasskeyAdded(_) => "passkey.added",
            Body::PasskeyUsed(_) => "passkey.used",
            Body::PasskeyRemoved(_) => "passkey.removed",
            Body::AwakeSet(_) => "awake.set",
            Body::LanguageSet(_) => "language.set",
            Body::GroupingSet(_) => "grouping.set",
            Body::MemorySet(_) => "memory.set",
            Body::PersonJoined(_) => "person.joined",
            Body::RoleSet(_) => "role.set",
            Body::Unknown(_) => "unknown",
        })
        .collect()
}
