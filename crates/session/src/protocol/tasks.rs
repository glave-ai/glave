//! What `tasks/*` and `schedule/*` answer with: a task (a thread running with no one attached)
//! as the tasks panel shows it, and a schedule.

use glave_event::{At, Money, Outcome, ScheduleId, ThreadId, TokenCounts, When};
use glave_ports::ProjectId;
use serde::{Deserialize, Serialize};

/// What kind of work a task is. Background commands the model starts arrive later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    /// A thread: a turn, or a goal's turns, run by the agent.
    Agent,
}

/// Whether a task is still going.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Running,
    /// Started and not ended, by a server other than this one (or one that stopped).
    Elsewhere,
    Finished,
}

/// A task, as the tasks panel shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TaskInfo {
    pub thread: ThreadId,
    /// The project its thread is in.
    pub project: ProjectId,
    pub kind: TaskKind,
    /// The prompt or objective, shortened.
    pub title: String,
    pub model: String,
    /// Tokens used so far, subagents' included.
    pub tokens: TokenCounts,
    /// What they cost so far, when every call reported a cost.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Money>,
    /// Tokens the run may use; no limit when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<u64>,
    pub started_at: At,
    /// Milliseconds from its start until now, or until it ended.
    pub elapsed_ms: u64,
    pub status: TaskStatus,
    /// How it ended, once it has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Outcome>,
    /// The schedule that started it, if one did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<ScheduleId>,
}

/// A schedule, as `schedule/list` shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ScheduleInfo {
    pub schedule: ScheduleId,
    pub when: When,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub goal: bool,
    pub budget: u64,
    pub created_at: At,
    /// When it last ran: the time that run was due.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<At>,
    /// When it runs next; absent once a one-off has run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<At>,
}
