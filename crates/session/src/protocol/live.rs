//! Progress while a turn runs, as `turn/live` carries it.

use glave_event::{CallId, EventId, MemoryId, ThreadId};
use glave_harness::{Live, Question};
use serde::{Deserialize, Serialize};

use crate::protocol::Told;

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
    /// The reasoning's headline changed: its last bold title, else its last whole sentence, as
    /// the model wrote it. A client shows the latest one beside the word that it thinks.
    Thinking {
        headline: String,
    },
    /// The reasoning ended after `ms` milliseconds: the model wrote something else, or its call
    /// ended. Its message records the same (`message.added`'s `thought`).
    Thought {
        ms: u64,
    },
    ToolStarted {
        summary: String,
    },
    ToolFinished {
        summary: String,
        is_error: bool,
    },
    /// An action that did not run. `summary` is what would have run; `caution`, why it asked,
    /// in sentences the person reads, when a caution did: in the caller's language where it
    /// was recorded with its reason.
    ToolDeclined {
        summary: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        caution: Option<Told>,
        no_one_there: bool,
    },
    /// A model call began, and nothing has come back yet: each step of a turn starts with
    /// one, so text and reasoning after it are the new step's, and what streamed before it
    /// from a call that was not recorded (one tried again) is gone.
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
    /// The `approval.requested` of call `approval` waits for an answer; give it with
    /// `approval/decide`. One the judge let run is decided at once and sends none.
    ApprovalPending {
        approval: CallId,
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
    /// A message sent while the turn ran (its `queued` id) that the model will not read, given
    /// back: a hook stopped it (`reason`, the hook's), or the turn was stopped or failed first.
    Unread {
        id: EventId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
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
            Live::Thinking(headline) => Self::Thinking { headline },
            Live::Thought(took) => Self::Thought {
                ms: u64::try_from(took.as_millis()).unwrap_or(u64::MAX),
            },
            Live::ToolStarted { summary } => Self::ToolStarted { summary },
            Live::ToolFinished { summary, is_error } => Self::ToolFinished { summary, is_error },
            Live::ToolDeclined {
                summary,
                caution,
                reason,
                no_one_there,
            } => Self::ToolDeclined {
                summary,
                caution: reason
                    .and_then(Told::caution)
                    .or_else(|| caution.map(Told::from)),
                no_one_there,
            },
            Live::Unread { id, reason } => Self::Unread { id, reason },
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
            Live::ApprovalPending { call_id } => Self::ApprovalPending { approval: call_id },
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
