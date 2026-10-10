//! The messages of the session API (`specs/session.md`): requests by method, their replies, the
//! notifications a turn sends, and errors. Every transport carries these same types; in-process
//! they are passed as they are, elsewhere as JSON-RPC 2.0.

use glave_event::{CallId, Money, Outcome, RawEvent, Seq, ThreadId, TokenCounts};
use glave_harness::{GoalProgress, Live, Question, TurnSummary};
use glave_ports::Failure;
use serde::{Deserialize, Serialize};

/// The protocol version this server speaks.
pub const PROTOCOL: &str = "2026-10";

/// A request, by method.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum Request {
    #[serde(rename = "server/discover")]
    Discover {},
    #[serde(rename = "thread/start")]
    ThreadStart {
        /// Whether the thread works in a worktree of its own rather than in the folder itself;
        /// the server's default (`worktree` in `config.toml`) when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        worktree: Option<bool>,
    },
    #[serde(rename = "thread/list")]
    ThreadList {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    },
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
    /// thread's active goal, as after a restart.
    #[serde(rename = "goal/start")]
    GoalStart {
        thread: ThreadId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        objective: Option<String>,
        /// A command whose success means the goal is met.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        check: Option<String>,
        /// Tokens the goal may use; glave's default when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        budget: Option<u64>,
    },
    /// Stops the thread's active goal, cancelling its running turn.
    #[serde(rename = "goal/stop")]
    GoalStop { thread: ThreadId },
    /// The thread's latest goal and how far it has come.
    #[serde(rename = "goal/get")]
    GoalGet { thread: ThreadId },
}

impl Request {
    /// The method's name, as JSON-RPC and the `Glave-Method` header carry it.
    pub fn method(&self) -> &'static str {
        match self {
            Self::Discover {} => "server/discover",
            Self::ThreadStart { .. } => "thread/start",
            Self::ThreadList { .. } => "thread/list",
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
        }
    }
}

/// What a request may carry besides its params, in `params._meta`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    /// For `turn/start`: the same key for the same thread returns the turn already started
    /// instead of starting another, so a client may retry safely.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<IdempotencyKey>,
}

impl Meta {
    fn is_empty(&self) -> bool {
        self.idempotency_key.is_none()
    }
}

/// A client's name for one attempt at a turn: 1 to 255 bytes of text.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

/// The person's answer to an approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Answer {
    AllowOnce,
    AllowForThread,
    Deny,
}

/// What the server is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
    /// The protocol versions spoken.
    pub protocols: Vec<String>,
}

/// A thread, as a list shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadSummary {
    pub id: ThreadId,
    pub status: Status,
}

/// What a thread is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Idle,
    Running,
    /// A turn is paused until someone answers an approval.
    Waiting,
}

/// How a turn ended and what it cost.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Reply {
    Server(ServerInfo),
    Thread {
        thread: ThreadSummary,
    },
    Threads {
        threads: Vec<ThreadSummary>,
    },
    Events {
        events: Vec<RawEvent>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next: Option<Seq>,
    },
    Turn {
        turn: TurnResult,
    },
    Compacted {
        before_tokens: u64,
        after_tokens: u64,
    },
    /// Before `Done`, which takes any object.
    Goal {
        goal: Option<GoalProgress>,
    },
    Done {},
}

/// Something a turn or a watched thread sends while it runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum Notification {
    /// A stored event, as it is appended.
    #[serde(rename = "thread/event")]
    Event { thread: ThreadId, event: RawEvent },
    /// Progress, never stored.
    #[serde(rename = "turn/live")]
    Live { thread: ThreadId, update: Update },
}

/// Progress while a turn runs: the live side of `Live`, in a form any transport carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
            // Published under the child's thread; see `Server::start_turn`.
            Live::Child { update, .. } => Self::from(*update),
        }
    }
}

/// A request that failed, as JSON-RPC carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<ErrorData>,
}

/// Why a model call failed, for a client to say what to fix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    pub const WORKSPACE: i64 = -32008;
    pub const CANCELLED: i64 = -32005;
    /// No valid sign-in, on a network transport.
    pub const NO_SIGN_IN: i64 = -32013;
    pub const PROVIDER: i64 = -32020;
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
