//! `task.started`, `task.ended`, `schedule.created` and `schedule.deleted`: a space's tasks
//! (threads that run with no one attached) and the schedules that start them. They are kept in
//! the space's task ledger, glave's own stream under a reserved id (`specs/events.md`), never
//! in a conversation.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Payload, parse};
use crate::envelope::At;
use crate::id::{ScheduleId, ThreadId};
use crate::money::Money;
use crate::payload::model::TokenCounts;
use crate::payload::principal::Principal;
use crate::payload::turn::Outcome;

pub(crate) const STARTED: &str = "task.started";
pub(crate) const ENDED: &str = "task.ended";
pub(crate) const CREATED: &str = "schedule.created";
pub(crate) const DELETED: &str = "schedule.deleted";

/// A thread began running as a task, with no one attached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskStarted {
    pub thread: ThreadId,
    /// What it works on, for lists: the prompt or objective, shortened.
    pub title: String,
    /// The model it was started with, as the server names it.
    pub model: String,
    /// Tokens, in and out, its run may use, subagents' included; no limit when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<u64>,
    /// The schedule that started it, and the time it was due.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<Fired>,
    /// Who started it. Absent: the space's person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Principal>,
}

/// A schedule's run: which schedule, and when it was due.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fired {
    pub schedule: ScheduleId,
    pub due: At,
}

/// A task ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskEnded {
    pub thread: ThreadId,
    /// How its last turn ended: `cancelled` when it was stopped, `limit_reached` when its
    /// budget ran out.
    pub outcome: Outcome,
    /// Tokens its model calls used, subagents' included.
    pub tokens: TokenCounts,
    /// What they cost, when every call reported a cost.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Money>,
    /// Who stopped it, when someone did. Absent: it ended on its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Principal>,
}

/// When a schedule runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum When {
    /// Once, at this time.
    At(At),
    /// Whenever this five-field cron expression matches, in UTC.
    Cron(String),
}

/// A schedule was made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleCreated {
    pub schedule: ScheduleId,
    pub when: When,
    /// The prompt each run starts with; with `goal`, the goal's objective.
    pub prompt: String,
    /// Whether each run works toward `prompt` as a goal across turns, rather than one turn.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub goal: bool,
    /// Tokens, in and out, each run may use, subagents' included. Required: no schedule runs
    /// unbounded.
    pub budget: u64,
    /// Who made it. Absent: the space's person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Principal>,
}

/// Version 1, which could say whether each run got a worktree of its own. Every run now works
/// on a copy of its project's folder, so the choice is dropped.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScheduleCreatedV1 {
    schedule: ScheduleId,
    when: When,
    prompt: String,
    #[serde(default)]
    goal: bool,
    budget: u64,
    #[serde(default, rename = "worktree")]
    _worktree: Option<bool>,
    #[serde(default)]
    by: Option<Principal>,
}

impl From<ScheduleCreatedV1> for ScheduleCreated {
    fn from(old: ScheduleCreatedV1) -> Self {
        Self {
            schedule: old.schedule,
            when: old.when,
            prompt: old.prompt,
            goal: old.goal,
            budget: old.budget,
            by: old.by,
        }
    }
}

/// A schedule was deleted: it starts no more runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleDeleted {
    pub schedule: ScheduleId,
    /// Who deleted it. Absent: the space's person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Principal>,
}

impl Payload for TaskStarted {
    const TYPE: &'static str = STARTED;
    const VERSION: u32 = 1;
}

impl Payload for TaskEnded {
    const TYPE: &'static str = ENDED;
    const VERSION: u32 = 1;
}

impl Payload for ScheduleCreated {
    const TYPE: &'static str = CREATED;
    const VERSION: u32 = 2;
}

impl Payload for ScheduleDeleted {
    const TYPE: &'static str = DELETED;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_started(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<TaskStarted>> {
    (version == 1).then(|| parse(data))
}

pub(crate) fn decode_ended(version: u32, data: &RawValue) -> Option<serde_json::Result<TaskEnded>> {
    (version == 1).then(|| parse(data))
}

pub(crate) fn decode_created(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<ScheduleCreated>> {
    match version {
        1 => Some(parse::<ScheduleCreatedV1>(data).map(ScheduleCreated::from)),
        2 => Some(parse(data)),
        _ => None,
    }
}

pub(crate) fn decode_deleted(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<ScheduleDeleted>> {
    (version == 1).then(|| parse(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_1_reads_without_its_worktree_choice() {
        let v1 = r#"{"schedule":"0192f5c1-0000-7000-8000-000000000001","when":{"cron":"0 9 * * *"},"prompt":"Triage","budget":10,"worktree":false}"#;
        let raw = RawValue::from_string(v1.to_owned()).unwrap();
        let upgraded = decode_created(1, &raw).unwrap().unwrap();
        assert_eq!((upgraded.prompt.as_str(), upgraded.budget), ("Triage", 10));
        assert!(
            decode_created(2, &raw).unwrap().is_err(),
            "version 2 has no worktree"
        );
    }
}
