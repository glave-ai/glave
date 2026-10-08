//! The messages of the session API (`specs/session.md`): requests by method, their replies, the
//! notifications a turn sends, and errors. Every transport carries these same types; in-process
//! they are passed as they are, elsewhere as JSON-RPC 2.0.

use glave_event::{CallId, Money, Outcome, RawEvent, Seq, ThreadId, TokenCounts};
use glave_harness::{Live, Question, TurnSummary};
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
    ThreadStart {},
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
    TurnStart { thread: ThreadId, prompt: String },
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
    pub const CANCELLED: i64 = -32005;
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
