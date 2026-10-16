//! The messages of the session API (`specs/session.md`): requests by method, their replies, the
//! notifications a turn sends, and errors. Every transport carries these same types; in-process
//! they are passed as they are, elsewhere as JSON-RPC 2.0.

mod credentials;
mod models;
mod providers;
mod tasks;

use glave_event::{
    At, CallId, CredentialId, MemoryId, MemoryScope, Money, Origin, Outcome, Principal, ProviderId,
    RawEvent, ScheduleId, Secret, Seq, ThreadId, TokenCounts, When,
};
use glave_harness::{GoalProgress, Live, Question, Remembered, TurnSummary};
use glave_ports::{ComputerId, Failure, Space, SpaceId, Standing};
use serde::{Deserialize, Serialize};

pub use credentials::{Ability, CredentialInfo, DeviceCode};
pub use glave_event::ApprovalMode;
pub use glave_event::{Forge, ProviderKind};
pub use models::ModelInfo;
pub use providers::{ProviderInfo, hint};
pub use tasks::{ScheduleInfo, TaskInfo, TaskKind, TaskStatus};

/// The protocol version this server speaks.
pub const PROTOCOL: &str = "2026-10";

/// The one table of methods: each method's name, its request's params, and the replies it
/// answers with, by the titles of their [`Reply`] variants. It makes [`Request`], whose serde
/// names are the methods, [`Request::method`], and [`RESULTS`] for the schema.
macro_rules! methods {
    ($(
        $(#[$doc:meta])*
        $method:literal => $variant:ident { $($params:tt)* } -> [$($reply:ident),+],
    )*) => {
        /// A request, by method.
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
        #[serde(tag = "method", content = "params")]
        pub enum Request {
            $(
                $(#[$doc])*
                #[serde(rename = $method)]
                $variant { $($params)* },
            )*
        }

        impl Request {
            /// The method's name, as JSON-RPC and the `Glave-Method` header carry it.
            pub fn method(&self) -> &'static str {
                match self {
                    $(Self::$variant { .. } => $method,)*
                }
            }
        }

        /// Every method, with the titles of the replies it answers with.
        pub(crate) const RESULTS: &[(&str, &[&str])] = &[
            $(($method, &[$(stringify!($reply)),+]),)*
        ];
    };
}

methods! {
    "server/discover" => Discover {} -> [Server],
    /// The models this server can run: the default first.
    "model/list" => ModelList {} -> [Models],
    "thread/start" => ThreadStart {
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
    } -> [Thread],
    "thread/list" => ThreadList {
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
    } -> [Threads],
    /// Gives a thread a title of the person's own.
    "thread/rename" => ThreadRename { thread: ThreadId, title: String } -> [Thread],
    /// Hides a thread from lists, keeping it in the log.
    "thread/archive" => ThreadArchive { thread: ThreadId } -> [Done],
    /// Brings an archived thread back.
    "thread/unarchive" => ThreadUnarchive { thread: ThreadId } -> [Done],
    /// Hides a thread for good and marks it for erasure.
    "thread/delete" => ThreadDelete { thread: ThreadId } -> [Done],
    "thread/get" => ThreadGet { thread: ThreadId } -> [Thread],
    "thread/events" => ThreadEvents {
        thread: ThreadId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<Seq>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    } -> [Events],
    "turn/start" => TurnStart {
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
    } -> [Turn, Task],
    "turn/cancel" => TurnCancel { thread: ThreadId } -> [Done],
    "thread/compact" => ThreadCompact { thread: ThreadId } -> [Compacted],
    "approval/decide" => ApprovalDecide {
        thread: ThreadId,
        approval: CallId,
        decision: Answer,
    } -> [Done],
    /// Answers the model's questions: one answer for each, in order.
    "question/answer" => QuestionAnswer {
        thread: ThreadId,
        question: CallId,
        answers: Vec<String>,
    } -> [Done],
    /// Sets a goal and works toward it across turns; without `objective`, continues the
    /// thread's active goal, as after a restart, or resumes the one the model paused.
    "goal/start" => GoalStart {
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
    } -> [Turn, Task],
    /// Stops the thread's active goal, cancelling its running turn.
    "goal/stop" => GoalStop { thread: ThreadId } -> [Done],
    /// The thread's latest goal and how far it has come.
    "goal/get" => GoalGet { thread: ThreadId } -> [Goal],
    /// Tasks running and recently finished, running first, then newest first.
    "tasks/list" => TasksList {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    } -> [Tasks],
    /// Stops the task running in a thread.
    "tasks/stop" => TasksStop { thread: ThreadId } -> [Done],
    /// Runs a prompt, or works toward a goal, as a task at set times.
    "schedule/create" => ScheduleCreate {
        when: When,
        prompt: String,
        #[serde(default, skip_serializing_if = "is_false")]
        goal: bool,
        /// Tokens each run may use. Required: no schedule runs unbounded.
        budget: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        worktree: Option<bool>,
    } -> [Schedule],
    "schedule/list" => ScheduleList {} -> [Schedules],
    "schedule/delete" => ScheduleDelete { schedule: ScheduleId } -> [Done],
    /// The spaces the caller works in: owns, or is a member of.
    "space/list" => SpaceList {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    } -> [Spaces],
    "space/get" => SpaceGet { space: SpaceId } -> [Space],
    /// A new project space, owned by the caller, with `members` working in it too.
    "space/create" => SpaceCreate {
        name: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        members: Vec<glave_event::Principal>,
    } -> [Space],
    /// Renames a space: its owner only.
    "space/rename" => SpaceRename { space: SpaceId, name: String } -> [Space],
    /// The memories here: the person's and the space's, or one `scope`.
    "memory/list" => MemoryList {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scope: Option<MemoryScope>,
    } -> [Memories],
    "memory/save" => MemorySave { text: String, scope: MemoryScope } -> [Memories],
    "memory/edit" => MemoryEdit { memory: MemoryId, text: String } -> [Memories],
    /// Saved in the new scope, forgotten in the old.
    "memory/move" => MemoryMove {
        memory: MemoryId,
        scope: MemoryScope,
    } -> [Memories],
    /// Forgets one memory, or with `all`, every memory of `scope`.
    "memory/forget" => MemoryForget {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        memory: Option<MemoryId>,
        #[serde(default, skip_serializing_if = "is_false")]
        all: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scope: Option<MemoryScope>,
    } -> [Memories],
    /// Undoes glave's latest change to a memory, within 30 days.
    "memory/restore" => MemoryRestore { memory: MemoryId } -> [Memories],
    /// Connects a model account: the key is checked with the service, then kept sealed, and
    /// the account's models join `model/list`. Connecting the same service (and URL) again
    /// replaces its account. Only the person on this computer.
    "provider/connect" => ProviderConnect {
        kind: ProviderKind,
        /// The account's key, sent once and never answered back.
        #[serde(serialize_with = "glave_event::serialize_exposed")]
        key: Secret,
        /// The server's API, for a `compatible` account: `https`, or `http` on this computer.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
    } -> [Provider],
    /// The model accounts connected here.
    "provider/list" => ProviderList {} -> [Providers],
    /// Removes a model account, destroying the key it was kept with.
    "provider/remove" => ProviderRemove { provider: ProviderId } -> [Done],
    /// Connects a git host: the token is checked with the host where it has a way to, then
    /// kept sealed, and git and the host's own tool sign in with it, asking at each use.
    /// Connecting the same host again replaces it. Only the person on this computer.
    "credential/connect" => CredentialConnect {
        /// Where the host is, such as `https://github.com`: `https`, or `http` on this computer.
        url: String,
        /// The token, sent once and never answered back.
        #[serde(serialize_with = "glave_event::serialize_exposed")]
        token: Secret,
        /// The user name git sends with the token, for a host that needs one; else whom the
        /// host says the token is for.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        username: Option<String>,
    } -> [Credential],
    /// The git hosts connected here, and what signing in to GitHub in the browser offers.
    "credential/list" => CredentialList {} -> [Credentials],
    /// Removes a git host, destroying its token.
    "credential/remove" => CredentialRemove { credential: CredentialId } -> [Done],
    /// Starts signing in to GitHub in the browser: the code to enter, and where.
    "credential/device/start" => CredentialDeviceStart {} -> [Device],
    /// Waits a little for the sign-in `flow`: the host, connected, once the person approved
    /// it on GitHub; else the flow again, to ask again.
    "credential/device/poll" => CredentialDevicePoll { flow: String } -> [Credential, Waiting],
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if"
)]
fn is_false(value: &bool) -> bool {
    !*value
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
    /// A model account, as it was connected.
    #[schemars(title = "Provider")]
    Provider { provider: ProviderInfo },
    #[schemars(title = "Providers")]
    Providers { providers: Vec<ProviderInfo> },
    /// A git host, as it was connected.
    #[schemars(title = "Credential")]
    Credential { credential: CredentialInfo },
    #[schemars(title = "Credentials")]
    Credentials {
        credentials: Vec<CredentialInfo>,
        /// Whether GitHub's sign-in in the browser (`credential/device/*`) is set up here;
        /// without it, a GitHub token is pasted.
        device: bool,
        /// Where the person chooses which repositories GitHub's app may use, when it has
        /// such a page.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        install: Option<String>,
    },
    #[schemars(title = "Device")]
    Device { device: DeviceCode },
    /// A sign-in in the browser still waiting for the person.
    #[schemars(title = "Waiting")]
    Waiting { flow: String },
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

/// Every error a request can be answered with (`specs/session.md`): JSON-RPC's own, then
/// glave's. Each is a constant and a row of [`code::ALL`], from one table.
pub mod code {
    macro_rules! codes {
        ($($name:ident = $code:literal, $meaning:literal;)*) => {
            $(
                #[doc = $meaning]
                pub const $name: i64 = $code;
            )*

            /// Every code: its number, a name for clients, and its meaning.
            pub const ALL: &[(i64, &str, &str)] = &[$(($code, stringify!($name), $meaning)),*];
        };
    }

    codes! {
        PARSE = -32700, "The message is not JSON-RPC";
        INVALID_REQUEST = -32600, "The message is not a valid request";
        NO_METHOD = -32601, "No such method";
        INVALID_PARAMS = -32602, "The params are not valid";
        INTERNAL = -32603, "The server failed";
        NO_THREAD = -32001, "No such thread";
        TURN_RUNNING = -32002, "A turn is already running in this thread";
        NO_TURN = -32003, "No turn is running in this thread";
        NO_APPROVAL = -32004, "No such approval is pending";
        CANCELLED = -32005, "The operation was cancelled";
        NO_QUESTION = -32006, "No such question is waiting";
        NO_GOAL = -32007, "The thread has no active goal (or, to resume, no paused one)";
        PROMPT_BLOCKED = -32008, "A hook stopped the prompt";
        WORKSPACE = -32009, "The thread's workspace could not be opened";
        NOT_ALLOWED = -32012, "The caller may not do this";
        NO_SIGN_IN = -32013, "No valid sign-in (network transports)";
        ARCHIVED = -32014, "The thread is archived; unarchive it first";
        LIMIT = -32015, "A policy limit was reached";
        NO_SCHEDULE = -32016, "No such schedule";
        NO_MEMORY = -32017, "No such memory here";
        MEMORY_OFF = -32018, "Memory is off for this server";
        NO_SPACE = -32019, "No such space";
        PROVIDER = -32020, "The model provider failed";
        NO_MODEL = -32021, "No such model here, or not one this server may run";
        NOT_AVAILABLE = -32022, "Not available yet, such as a thread on a computer of the person's own";
        NO_PROVIDER = -32023, "No such model account here";
        NO_CREDENTIAL = -32024, "No such git host, or sign-in, here";
        HOST = -32025, "The git host refused, or could not be reached";
    }
}

impl ApiError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
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
