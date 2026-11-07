//! What the person sees while a turn runs, and what they send it.

use std::time::Duration;

use glave_event::{CallId, Caution, EventId, MemoryId, Part, ThreadId};

use glave_ports::Caller;

use crate::memory::MemoryVerb;
use crate::question::Question;

/// What the person sees while a turn runs. Never stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Live {
    Text(String),
    Reasoning(String),
    /// The reasoning's headline changed: its last bold title, else its last whole sentence.
    Thinking(String),
    /// The reasoning ended, after this long: the model wrote something else, or its call ended.
    Thought(Duration),
    ToolStarted {
        summary: String,
    },
    ToolFinished {
        summary: String,
        is_error: bool,
    },
    /// An action that did not run: the person said no, or no one was there to ask. `caution`
    /// is why it asked, as the person reads it, when a caution did, and `reason` what it was
    /// written from, as its `approval.requested` records them.
    ToolDeclined {
        summary: String,
        caution: Option<String>,
        reason: Option<Caution>,
        no_one_there: bool,
    },
    /// The conversation was summarized to fit the context window; sizes are estimates.
    Compacted {
        before_tokens: u64,
        after_tokens: u64,
    },
    /// A needed compaction failed; the turn goes on without it.
    CompactionFailed(String),
    /// The model asked the person questions, and the turn waits for the answers.
    Asked {
        call_id: CallId,
        questions: Vec<Question>,
    },
    /// The call's `approval.requested` waits for the person's answer, which someone can give.
    /// One the judge let run is decided at once, and never pending.
    ApprovalPending {
        call_id: CallId,
    },
    /// A subagent started, in a child thread.
    SubagentStarted {
        thread: ThreadId,
        description: String,
    },
    /// What a subagent shows, under its own thread.
    Child {
        thread: ThreadId,
        update: Box<Live>,
    },
    /// A subagent is done; `ok` when it finished its work.
    SubagentEnded {
        thread: ThreadId,
        ok: bool,
    },
    /// A memory was saved, edited or forgotten in this turn: shown at once in every client.
    Memory {
        verb: MemoryVerb,
        memory: MemoryId,
        /// Its text, when it was saved or edited.
        text: Option<String>,
    },
    /// A message the person sent while the turn ran that the model will not read: a hook
    /// stopped it (`reason`, as the hook gave it), or the turn was stopped or failed before its
    /// next call (`None`). `id` is the one it was queued with.
    Unread {
        id: EventId,
        reason: Option<String>,
    },
    /// The model was asked, and nothing has come back yet.
    Waiting,
    /// A model call failed in a way that may pass, and is tried again after `wait`.
    Retrying {
        retry: u32,
        of: u32,
        wait: Duration,
        error: String,
    },
}

/// A person's message: its text, and the files they added to it, as `file` parts.
#[derive(Debug, Clone, Copy)]
pub struct Said<'a> {
    pub text: &'a str,
    pub files: &'a [Part],
}

/// What the person asked for, and who asked.
#[derive(Clone, Copy)]
pub(crate) struct Prompt<'a> {
    pub(crate) text: &'a str,
    /// The files the person added to it, as `file` parts after its text.
    pub(crate) files: &'a [Part],
    pub(crate) caller: &'a Caller,
    /// Whether glave wrote the prompt, as a plan's build: recorded as glave's, and
    /// never read as the person's say-so.
    pub(crate) by_glave: bool,
    /// The id its message is recorded with, when it was made already: a message sent while
    /// the turn before ran keeps the id it was queued with.
    pub(crate) id: Option<EventId>,
}

impl<'a> Prompt<'a> {
    /// `text`, as `caller` wrote it.
    pub(crate) fn new(text: &'a str, caller: &'a Caller) -> Self {
        Self {
            text,
            files: &[],
            caller,
            by_glave: false,
            id: None,
        }
    }
}
