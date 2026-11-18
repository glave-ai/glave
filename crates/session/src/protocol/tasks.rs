//! What `tasks/*` and `schedule/*` answer with: a task (a thread running with no one attached)
//! as the tasks panel shows it, and a schedule.

use glave_event::{At, Money, Outcome, ScheduleId, ThreadId, TokenCounts, When};
use glave_ports::ProjectId;
use serde::{Deserialize, Serialize};

use super::Told;

/// What kind of work a task is. Background commands the model starts arrive later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    /// A thread: a turn run by the agent.
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
    /// The prompt, shortened.
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
    /// What it is called, as its chat is.
    pub name: String,
    /// The project its chat is in.
    pub project: ProjectId,
    /// Its own chat, which each run adds a turn to; absent for one made before schedules had
    /// chats, which runs no more (`note` says so).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<ThreadId>,
    /// The chat it was set up from, if it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<ThreadId>,
    pub when: When,
    /// The time zone `when` is read in, by its IANA name.
    pub zone: String,
    /// When it repeats, in the server's words: "Every weekday at 8:00 am".
    pub repeats: Told,
    pub prompt: String,
    pub budget: u64,
    pub created_at: At,
    /// Its latest run, once one was due.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<LastRun>,
    /// When it runs next; absent once a one-off has run, and when it runs no more.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<At>,
    /// What the person should know of it beyond that, in the server's words, such as that it
    /// runs no more.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<Told>,
}

/// A schedule's latest run: when it was due, what came of it, and that in the server's words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct LastRun {
    pub due: At,
    pub status: RunStatus,
    /// How it ended, once it has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Outcome>,
    /// What came of it, in a few words: "Done", "Skipped: the run before was still going".
    pub said: Told,
}

/// What came of a schedule's run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// Running now, or waiting for the person.
    Running,
    /// Ended, as `outcome` says.
    Ended,
    /// Not started: the run before was still going in its chat, or someone's turn was.
    Skipped,
    /// Not started: it was refused, as `said` says.
    Refused,
    /// Started; how it ended is no longer kept here.
    Started,
}
