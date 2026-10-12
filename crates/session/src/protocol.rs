//! The messages of the session API (`specs/session.md`): requests by method, their replies, the
//! notifications a turn sends, and errors. Every transport carries these same types; in-process
//! they are passed as they are, elsewhere as JSON-RPC 2.0.

mod models;
mod tasks;

use glave_event::{
    At, CallId, MemoryId, MemoryScope, Money, Origin, Outcome, Principal, RawEvent, ScheduleId,
    Seq, ThreadId, TokenCounts, When,
};
use glave_harness::{GoalProgress, Live, Question, Remembered, TurnSummary};
use glave_ports::{ComputerId, Failure, Space, SpaceId, Standing};
use serde::{Deserialize, Serialize};

pub use glave_event::ApprovalMode;
pub use models::ModelInfo;
pub use tasks::{ScheduleInfo, TaskInfo, TaskKind, TaskStatus};

/// The protocol version this server speaks.
pub const PROTOCOL: &str = "2026-10";

/// A request, by method.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "method", content = "params")]
pub enum Request {
    #[serde(rename = "server/discover")]
    Discover {},
    /// The models this server can run: the default first.
    #[serde(rename = "model/list")]
    ModelList {},
    #[serde(rename = "thread/start")]
    ThreadStart {
        /// The space the thread is in: by default the server's own, for a caller who works in
        /// it, else the caller's personal space.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        space: Option<SpaceId>,
        /// Whether the thread works in a worktree of its own rather than in the folder itself;
        /// the server's default (`worktree` in `config.toml`) when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        worktree: Option<bool>,
        /// For a thread that runs as a task: it works on a worker when the server has them,
        /// even when other threads work here.
        #[serde(default, skip_serializing_if = "is_false")]
        background: bool,
        /// A running computer to work on, such as the person's own laptop dialled in to this
        /// server. Not available yet: refused with `-32022`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        computer: Option<ComputerId>,
        /// The model the thread's turns go to, one `model/list` names; the server's default
        /// when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        /// How much the thread's turns ask before acting, one of the server's `approvals`;
        /// the server's own mode when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        approval: Option<ApprovalMode>,
    },
    #[serde(rename = "thread/list")]
    ThreadList {
        /// The space whose threads to list, by default as `thread/start` picks one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        space: Option<SpaceId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
        /// Which threads: the active ones by default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        show: Option<Show>,
        /// Where the page starts: the `next` of the page before.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cursor: Option<String>,
    },
    /// Gives a thread a title of the person's own.
    #[serde(rename = "thread/rename")]
    ThreadRename { thread: ThreadId, title: String },
    /// Hides a thread from lists, keeping it in the log.
    #[serde(rename = "thread/archive")]
    ThreadArchive { thread: ThreadId },
    /// Brings an archived thread back.
    #[serde(rename = "thread/unarchive")]
    ThreadUnarchive { thread: ThreadId },
    /// Hides a thread for good and marks it for erasure.
    #[serde(rename = "thread/delete")]
    ThreadDelete { thread: ThreadId },
    #[serde(rename = "thread/get")]
    ThreadGet { thread: ThreadId },
    #[serde(rename = "thread/events")]
    ThreadEvents {
        thread: ThreadId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<Seq>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    },
    #[serde(rename = "turn/start")]
    TurnStart {
        thread: ThreadId,
        prompt: String,
        /// Runs the turn as a task, with no one attached: answered at once with the task.
        #[serde(default, skip_serializing_if = "is_false")]
        background: bool,
        /// With `background`, tokens the turn may use, subagents' included.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        budget: Option<u64>,
        /// The model this turn, and the thread's turns after it, go to: one `model/list`
        /// names. The thread's latest choice when absent, else the server's default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        /// How much this turn, and the thread's turns after it, ask before acting: one of the
        /// server's `approvals`. The thread's latest choice when absent, else the server's.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        approval: Option<ApprovalMode>,
        #[serde(rename = "_meta", default, skip_serializing_if = "Meta::is_empty")]
        meta: Meta,
    },
    #[serde(rename = "turn/cancel")]
    TurnCancel { thread: ThreadId },
    #[serde(rename = "thread/compact")]
    ThreadCompact { thread: ThreadId },
    #[serde(rename = "approval/decide")]
    ApprovalDecide {
        thread: ThreadId,
        approval: CallId,
        decision: Answer,
    },
    /// Answers the model's questions: one answer for each, in order.
    #[serde(rename = "question/answer")]
    QuestionAnswer {
        thread: ThreadId,
        question: CallId,
        answers: Vec<String>,
    },
    /// Sets a goal and works toward it across turns; without `objective`, continues the
    /// thread's active goal, as after a restart, or resumes the one the model paused.
    #[serde(rename = "goal/start")]
    GoalStart {
        thread: ThreadId,
        /// What to achieve, with how to verify it in the person's words.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        objective: Option<String>,
        /// Tokens the goal may use; glave's default when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        budget: Option<u64>,
        /// Works toward the goal as a task, with no one attached: answered at once.
        #[serde(default, skip_serializing_if = "is_false")]
        background: bool,
    },
    /// Stops the thread's active goal, cancelling its running turn.
    #[serde(rename = "goal/stop")]
    GoalStop { thread: ThreadId },
    /// The thread's latest goal and how far it has come.
    #[serde(rename = "goal/get")]
    GoalGet { thread: ThreadId },
    /// Tasks running and recently finished, running first, then newest first.
    #[serde(rename = "tasks/list")]
    TasksList {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    },
    /// Stops the task running in a thread.
    #[serde(rename = "tasks/stop")]
    TasksStop { thread: ThreadId },
    /// Runs a prompt, or works toward a goal, as a task at set times.
    #[serde(rename = "schedule/create")]
    ScheduleCreate {
        when: When,
        prompt: String,
        #[serde(default, skip_serializing_if = "is_false")]
        goal: bool,
        /// Tokens each run may use. Required: no schedule runs unbounded.
        budget: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        worktree: Option<bool>,
    },
    #[serde(rename = "schedule/list")]
    ScheduleList {},
    #[serde(rename = "schedule/delete")]
    ScheduleDelete { schedule: ScheduleId },
    /// The spaces the caller works in: owns, or is a member of.
    #[serde(rename = "space/list")]
    SpaceList {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    },
    #[serde(rename = "space/get")]
    SpaceGet { space: SpaceId },
    /// A new project space, owned by the caller, with `members` working in it too.
    #[serde(rename = "space/create")]
    SpaceCreate {
        name: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        members: Vec<glave_event::Principal>,
    },
    /// Renames a space: its owner only.
    #[serde(rename = "space/rename")]
    SpaceRename { space: SpaceId, name: String },
    /// The memories here: the person's and the space's, or one `scope`.
    #[serde(rename = "memory/list")]
    MemoryList {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scope: Option<MemoryScope>,
    },
    #[serde(rename = "memory/save")]
    MemorySave { text: String, scope: MemoryScope },
    #[serde(rename = "memory/edit")]
    MemoryEdit { memory: MemoryId, text: String },
    /// Saved in the new scope, forgotten in the old.
    #[serde(rename = "memory/move")]
    MemoryMove {
        memory: MemoryId,
        scope: MemoryScope,
    },
    /// Forgets one memory, or with `all`, every memory of `scope`.
    #[serde(rename = "memory/forget")]
    MemoryForget {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        memory: Option<MemoryId>,
        #[serde(default, skip_serializing_if = "is_false")]
        all: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scope: Option<MemoryScope>,
    },
    /// Undoes glave's latest change to a memory, within 30 days.
    #[serde(rename = "memory/restore")]
    MemoryRestore { memory: MemoryId },
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if"
)]
fn is_false(value: &bool) -> bool {
    !*value
}

impl Request {
    /// The method's name, as JSON-RPC and the `Glave-Method` header carry it.
    pub fn method(&self) -> &'static str {
        match self {
            Self::Discover {} => "server/discover",
            Self::ModelList {} => "model/list",
            Self::ThreadStart { .. } => "thread/start",
            Self::ThreadList { .. } => "thread/list",
            Self::ThreadRename { .. } => "thread/rename",
            Self::ThreadArchive { .. } => "thread/archive",
            Self::ThreadUnarchive { .. } => "thread/unarchive",
            Self::ThreadDelete { .. } => "thread/delete",
            Self::ThreadGet { .. } => "thread/get",
            Self::ThreadEvents { .. } => "thread/events",
            Self::TurnStart { .. } => "turn/start",
            Self::TurnCancel { .. } => "turn/cancel",
            Self::ThreadCompact { .. } => "thread/compact",
            Self::ApprovalDecide { .. } => "approval/decide",
            Self::QuestionAnswer { .. } => "question/answer",
            Self::GoalStart { .. } => "goal/start",
            Self::GoalStop { .. } => "goal/stop",
            Self::GoalGet { .. } => "goal/get",
            Self::TasksList { .. } => "tasks/list",
            Self::TasksStop { .. } => "tasks/stop",
            Self::ScheduleCreate { .. } => "schedule/create",
            Self::ScheduleList {} => "schedule/list",
            Self::ScheduleDelete { .. } => "schedule/delete",
            Self::SpaceList { .. } => "space/list",
            Self::SpaceGet { .. } => "space/get",
            Self::SpaceCreate { .. } => "space/create",
            Self::SpaceRename { .. } => "space/rename",
            Self::MemoryList { .. } => "memory/list",
            Self::MemorySave { .. } => "memory/save",
            Self::MemoryEdit { .. } => "memory/edit",
            Self::MemoryMove { .. } => "memory/move",
            Self::MemoryForget { .. } => "memory/forget",
            Self::MemoryRestore { .. } => "memory/restore",
        }
    }
}

impl Request {
    /// The thread a request acts on, when it names one: whose space decides whether the
    /// caller may make it.
    pub fn thread(&self) -> Option<ThreadId> {
        match self {
            Self::ThreadArchive { thread }
            | Self::ThreadUnarchive { thread }
            | Self::ThreadDelete { thread }
            | Self::ThreadRename { thread, .. }
            | Self::ThreadGet { thread }
            | Self::ThreadEvents { thread, .. }
            | Self::TurnStart { thread, .. }
            | Self::TurnCancel { thread }
            | Self::ThreadCompact { thread }
            | Self::ApprovalDecide { thread, .. }
            | Self::QuestionAnswer { thread, .. }
            | Self::GoalStart { thread, .. }
            | Self::GoalStop { thread }
            | Self::GoalGet { thread }
            | Self::TasksStop { thread } => Some(*thread),
            _ => None,
        }
    }

    /// Whether the request acts in the server's own space as a whole: its tasks, schedules
    /// and memory.
    pub(crate) fn in_home(&self) -> bool {
        matches!(
            self,
            Self::TasksList { .. }
                | Self::ScheduleCreate { .. }
                | Self::ScheduleList {}
                | Self::ScheduleDelete { .. }
                | Self::MemoryList { .. }
                | Self::MemorySave { .. }
                | Self::MemoryEdit { .. }
                | Self::MemoryMove { .. }
                | Self::MemoryForget { .. }
                | Self::MemoryRestore { .. }
        )
    }
}

/// What a request may carry besides its params, in `params._meta`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Meta {
    /// For `turn/start`: the same key for the same thread returns the turn already started
    /// instead of starting another, so a client may retry safely.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<IdempotencyKey>,
    /// For `turn/start`: the W3C trace the turn belongs to (`00-<trace id>-<parent id>-<flags>`),
    /// recorded with the turn and joined by glave's spans. One that is not one is ignored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traceparent: Option<String>,
}

impl Meta {
    fn is_empty(&self) -> bool {
        self.idempotency_key.is_none() && self.traceparent.is_none()
    }
}

/// A client's name for one attempt at a turn: 1 to 255 bytes of text.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(try_from = "String", into = "String")]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    /// The longest key, in bytes.
    pub const MAX: usize = 255;

    pub fn new(key: impl Into<String>) -> Result<Self, String> {
        let key = key.into();
        if key.is_empty() || key.len() > Self::MAX {
            return Err(format!(
                "an idempotency key is 1 to {} bytes, got {}",
                Self::MAX,
                key.len()
            ));
        }
        Ok(Self(key))
    }
}

impl TryFrom<String> for IdempotencyKey {
    type Error = String;

    fn try_from(key: String) -> Result<Self, Self::Error> {
        Self::new(key)
    }
}

impl From<IdempotencyKey> for String {
    fn from(key: IdempotencyKey) -> Self {
        key.0
    }
}

/// Which threads `thread/list` shows.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Show {
    /// Threads that are neither archived nor deleted.
    #[default]
    Active,
    Archived,
    /// Deleted threads, waiting for their content to be erased: for the operator that does it.
    Deleted,
}

/// The person's answer to an approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Answer {
    AllowOnce,
    AllowForThread,
    Deny,
}

/// What the server is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
    /// The protocol versions spoken.
    pub protocols: Vec<String>,
    /// How much this server asks before acting in a thread that chose no mode: `--approval` or
    /// `approval` in `config.toml` (`specs/approvals.md`).
    pub approval: ApprovalMode,
    /// The modes a thread may choose (`approval` on `thread/start` and `turn/start`): those
    /// the policy leaves, cautious first.
    pub approvals: Vec<ApprovalMode>,
}

/// A thread, as a list shows it: from the threads view, with what it is doing now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ThreadSummary {
    pub id: ThreadId,
    pub space: SpaceId,
    pub status: Status,
    pub standing: Standing,
    /// The title the person gave it, else the first line of their first message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// How much its turns ask before acting: the mode last chosen for it, else the server's.
    pub approval: ApprovalMode,
    /// Whose work it is; absent in threads from before it was recorded (the local person).
    #[serde(rename = "for", default, skip_serializing_if = "Option::is_none")]
    pub for_: Option<Principal>,
    pub origin: Origin,
    /// The thread whose call started this one, for a subagent's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<ThreadId>,
    pub created_at: At,
    pub updated_at: At,
    pub turns: u64,
    /// How its latest turn ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Outcome>,
    /// Tokens its own model calls used.
    pub tokens: TokenCounts,
    /// What they cost, when every call reported a cost.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Money>,
}

/// What a thread is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Idle,
    Running,
    /// A turn is paused until someone answers an approval.
    Waiting,
}

/// How a turn ended and what it cost.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TurnResult {
    pub outcome: Outcome,
    pub model_calls: u32,
    pub tool_calls: u32,
    pub tokens: TokenCounts,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Money>,
    /// True when a usage count was estimated, not reported.
    #[serde(default)]
    pub estimated: bool,
}

impl From<TurnSummary> for TurnResult {
    fn from(summary: TurnSummary) -> Self {
        Self {
            outcome: summary.outcome,
            model_calls: summary.model_calls,
            tool_calls: summary.tool_calls,
            tokens: summary.tokens,
            cost: summary.cost,
            estimated: summary.estimated,
        }
    }
}

/// A reply, by the shape of its result.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum Reply {
    #[schemars(title = "Server")]
    Server(ServerInfo),
    #[schemars(title = "Thread")]
    Thread { thread: ThreadSummary },
    #[schemars(title = "Threads")]
    Threads {
        threads: Vec<ThreadSummary>,
        /// The cursor of the next page, when there is one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next: Option<String>,
    },
    #[schemars(title = "Events")]
    Events {
        events: Vec<RawEvent>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next: Option<Seq>,
    },
    #[schemars(title = "Turn")]
    Turn { turn: TurnResult },
    #[schemars(title = "Compacted")]
    Compacted {
        before_tokens: u64,
        after_tokens: u64,
    },
    #[schemars(title = "Task")]
    Task { task: TaskInfo },
    #[schemars(title = "Tasks")]
    Tasks { tasks: Vec<TaskInfo> },
    #[schemars(title = "Schedule")]
    Schedule { schedule: ScheduleInfo },
    #[schemars(title = "Schedules")]
    Schedules { schedules: Vec<ScheduleInfo> },
    /// A space: one asked for, made or renamed.
    #[schemars(title = "Space")]
    Space { space: Space },
    #[schemars(title = "Spaces")]
    Spaces { spaces: Vec<Space> },
    /// Memories listed, or the ones a change touched.
    #[schemars(title = "Memories")]
    Memories { memories: Vec<Remembered> },
    /// The models a thread may be sent to, the default first.
    #[schemars(title = "Models")]
    Models { models: Vec<ModelInfo> },
    // After every reply with a field of its own, since a missing `goal` reads as `null`;
    // before `Done`, which takes any object.
    /// The thread's latest goal, or `null`.
    #[schemars(title = "Goal")]
    Goal { goal: Option<GoalProgress> },
    #[schemars(title = "Done")]
    Done {},
}

/// Something a turn or a watched thread sends while it runs.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "method", content = "params")]
pub enum Notification {
    /// A stored event, as it is appended.
    #[serde(rename = "thread/event")]
    Event { thread: ThreadId, event: RawEvent },
    /// Progress, never stored.
    #[serde(rename = "turn/live")]
    Live { thread: ThreadId, update: Update },
    /// A task ended: sent to the thread's watchers. Never stored as such; the ledger keeps it.
    #[serde(rename = "task/finished")]
    TaskFinished { thread: ThreadId, task: TaskInfo },
}

/// Progress while a turn runs: the live side of `Live`, in a form any transport carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Update {
    Text {
        text: String,
    },
    Reasoning {
        text: String,
    },
    ToolStarted {
        summary: String,
    },
    ToolFinished {
        summary: String,
        is_error: bool,
    },
    ToolDeclined {
        summary: String,
        no_one_there: bool,
    },
    Waiting,
    Retrying {
        retry: u32,
        of: u32,
        wait_ms: u64,
        error: String,
    },
    Compacted {
        before_tokens: u64,
        after_tokens: u64,
    },
    CompactionFailed {
        error: String,
    },
    /// The model asked the person questions; answer with `question/answer`.
    Asked {
        question: CallId,
        questions: Vec<Question>,
    },
    /// A subagent started; its notifications follow under its own thread.
    SubagentStarted {
        subagent: ThreadId,
        description: String,
    },
    /// A subagent is done; `ok` when it finished its work.
    SubagentEnded {
        subagent: ThreadId,
        ok: bool,
    },
    /// A memory was saved, edited or forgotten: `change` is `remembered`, `edited` or `forgot`.
    Memory {
        change: String,
        memory: MemoryId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
}

impl From<Live> for Update {
    fn from(live: Live) -> Self {
        match live {
            Live::Text(text) => Self::Text { text },
            Live::Reasoning(text) => Self::Reasoning { text },
            Live::ToolStarted { summary } => Self::ToolStarted { summary },
            Live::ToolFinished { summary, is_error } => Self::ToolFinished { summary, is_error },
            Live::ToolDeclined {
                summary,
                no_one_there,
            } => Self::ToolDeclined {
                summary,
                no_one_there,
            },
            Live::Waiting => Self::Waiting,
            Live::Retrying {
                retry,
                of,
                wait,
                error,
            } => Self::Retrying {
                retry,
                of,
                wait_ms: u64::try_from(wait.as_millis()).unwrap_or(u64::MAX),
                error,
            },
            Live::Compacted {
                before_tokens,
                after_tokens,
            } => Self::Compacted {
                before_tokens,
                after_tokens,
            },
            Live::CompactionFailed(error) => Self::CompactionFailed { error },
            Live::Asked { call_id, questions } => Self::Asked {
                question: call_id,
                questions,
            },
            Live::SubagentStarted {
                thread,
                description,
            } => Self::SubagentStarted {
                subagent: thread,
                description,
            },
            Live::SubagentEnded { thread, ok } => Self::SubagentEnded {
                subagent: thread,
                ok,
            },
            Live::Memory { verb, memory, text } => Self::Memory {
                change: verb.as_str().to_owned(),
                memory,
                text,
            },
            // Published under the child's thread; see `turn::publish`.
            Live::Child { update, .. } => Self::from(*update),
        }
    }
}

/// A request that failed, as JSON-RPC carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ApiError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<ErrorData>,
}

/// Why a model call failed, for a client to say what to fix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ErrorData {
    pub failure: Failure,
}

/// glave's error codes (`specs/session.md`).
pub mod code {
    pub const INTERNAL: i64 = -32603;
    pub const NO_THREAD: i64 = -32001;
    pub const TURN_RUNNING: i64 = -32002;
    pub const NO_TURN: i64 = -32003;
    pub const NO_APPROVAL: i64 = -32004;
    pub const NO_QUESTION: i64 = -32006;
    pub const NO_GOAL: i64 = -32007;
    /// The thread's workspace could not be opened, such as a worktree outside a repository.
    pub const WORKSPACE: i64 = -32009;
    /// The thread is archived: unarchive it before running a turn in it.
    pub const ARCHIVED: i64 = -32014;
    /// A hook stopped the prompt before the model saw it.
    pub const PROMPT_BLOCKED: i64 = -32008;
    pub const CANCELLED: i64 = -32005;
    /// No valid sign-in, on a network transport.
    pub const NO_SIGN_IN: i64 = -32013;
    pub const PROVIDER: i64 = -32020;
    /// No such model here, or not one this server may run.
    pub const NO_MODEL: i64 = -32021;
    /// A policy limit was reached: too many tasks running, or too many schedules.
    pub const LIMIT: i64 = -32015;
    /// No such schedule.
    pub const NO_SCHEDULE: i64 = -32016;
    /// The caller may not do this, such as change another person's memory.
    pub const NOT_ALLOWED: i64 = -32012;
    /// No such memory here.
    pub const NO_MEMORY: i64 = -32017;
    /// Memory is off here.
    pub const MEMORY_OFF: i64 = -32018;
    /// No such space, or none the caller works in.
    pub const NO_SPACE: i64 = -32019;
    /// The request's parameters do not make sense together.
    pub const INVALID_PARAMS: i64 = -32602;
    /// What was asked for is not built yet, such as a thread on a computer that dialled in.
    pub const NOT_AVAILABLE: i64 = -32022;
}

impl ApiError {
    pub(crate) fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ApiError {}
