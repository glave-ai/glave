//! A thread's open turn as the threads view keeps it (`specs/one-server.md`, "Every wait is in
//! the log"): where it started, its calls without a result, and what it waits at, folded from
//! its events as the rest of the view is. Content-free: call ids and kinds alone.

use glave_event::{Body, CallId, Event, RawEvent, Seq, decode};
use serde::{Deserialize, Serialize};

/// The tool a subagent's work is started with: its call waits while the subagent's turn does.
pub const AGENT_TOOL: &str = "agent";

/// The tool the model puts questions to the person with: its call waits for their answers.
pub const QUESTION_TOOL: &str = "question";

/// A turn started and not yet ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenTurn {
    /// Its `turn.started`.
    pub started: Seq,
    /// What it waits at, when its first call without a result waits: for the person, or for
    /// subagents' turns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait: Option<Wait>,
    /// Its calls without a result, in the order they were asked for.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unfinished: Vec<Unfinished>,
}

/// What an open turn waits at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Wait {
    pub kind: WaitKind,
    /// The call that waits.
    pub call: CallId,
    /// The event it waits at: the approval's `approval.requested`, or the call's
    /// `tool.call.requested`.
    pub seq: Seq,
}

/// What kind of wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitKind {
    /// An approval asked and not decided.
    Approval,
    /// The model's questions, not answered.
    Question,
    /// An `agent` call: the turn waits while one of its subagents' turns waits, which their
    /// own views say (`ThreadQuery::parent`).
    Agent,
}

/// A call of an open turn that has no result yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Unfinished {
    pub call: CallId,
    pub kind: CallKind,
    /// Its `tool.call.requested`.
    pub seq: Seq,
    /// Its `approval.requested`, while no decision follows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asked: Option<Seq>,
    /// Whether its approval was decided: the turn goes on with it, and waits no more.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub decided: bool,
}

/// What a call is, as far as waiting goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallKind {
    Tool,
    Question,
    Agent,
}

impl CallKind {
    /// The kind of a call of the tool `name`, as glave reads tool names: forgiving case and
    /// stray whitespace.
    pub fn of(name: &str) -> Self {
        let name = name.trim();
        if name.eq_ignore_ascii_case(AGENT_TOOL) {
            Self::Agent
        } else if name.eq_ignore_ascii_case(QUESTION_TOOL) {
            Self::Question
        } else {
            Self::Tool
        }
    }
}

impl OpenTurn {
    /// The turn `turn.started` at `started` begins.
    pub(crate) fn at(started: Seq) -> Self {
        Self {
            started,
            wait: None,
            unfinished: Vec::new(),
        }
    }

    /// Takes in `event`, one of this turn's after its start.
    pub(crate) fn apply(&mut self, event: &RawEvent) {
        let kinds = [
            "tool.call.requested",
            "tool.call.completed",
            "approval.requested",
            "approval.decided",
        ];
        if !kinds.contains(&event.kind.as_str()) {
            return;
        }
        let Ok(Event { body, .. }) = decode(event.clone()) else {
            return;
        };
        match body {
            Body::ToolCallRequested(call) => self.unfinished.push(Unfinished {
                kind: CallKind::of(&call.tool),
                call: call.call_id,
                seq: event.seq,
                asked: None,
                decided: false,
            }),
            Body::ToolCallCompleted(done) => {
                self.unfinished.retain(|call| call.call != done.call_id);
            }
            Body::ApprovalRequested(asked) => self.asked(&asked.call_id, Some(event.seq), false),
            Body::ApprovalDecided(decided) => self.asked(&decided.call_id, None, true),
            _ => {}
        }
        self.wait = self.first_wait();
    }

    fn asked(&mut self, call: &CallId, asked: Option<Seq>, decided: bool) {
        if let Some(unfinished) = self.unfinished.iter_mut().find(|found| found.call == *call) {
            unfinished.asked = asked;
            unfinished.decided = decided;
        }
    }

    /// The wait at its first call without a result, if that call waits.
    fn first_wait(&self) -> Option<Wait> {
        let first = self.unfinished.first()?;
        let (kind, seq) = match (first.asked, first.kind) {
            (Some(asked), _) => (WaitKind::Approval, asked),
            (None, CallKind::Question) => (WaitKind::Question, first.seq),
            (None, CallKind::Agent) => (WaitKind::Agent, first.seq),
            (None, CallKind::Tool) => return None,
        };
        Some(Wait {
            kind,
            call: first.call.clone(),
            seq,
        })
    }
}
