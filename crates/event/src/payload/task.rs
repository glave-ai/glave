//! `task.started`, `task.ended`, and `schedule.created`, `.paused`, `.resumed`, `.skipped` and
//! `.deleted`: a project's tasks (threads that run with no one attached) and the schedules that
//! start them. They are kept in the project's task ledger, glave's own stream under a reserved id
//! (`specs/events.md`), never in a conversation.

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
pub(crate) const PAUSED: &str = "schedule.paused";
pub(crate) const RESUMED: &str = "schedule.resumed";
pub(crate) const SKIPPED: &str = "schedule.skipped";

/// The time zone a schedule made before schedules kept one runs in.
const UTC: &str = "UTC";

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
    /// Who started it. Absent: the project's person.
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
    /// Whenever this five-field cron expression matches, in the schedule's time zone.
    Cron(String),
}

/// A schedule was made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleCreated {
    pub schedule: ScheduleId,
    /// What the person calls it, which its chat is named: "Morning email summary".
    pub name: String,
    pub when: When,
    /// The time zone its cron expression is read in, by its IANA name (`Europe/Lisbon`): the
    /// person's when it was made.
    pub zone: String,
    /// The prompt each run starts with.
    pub prompt: String,
    /// Tokens, in and out, each run may use, subagents' included. Required: no schedule runs
    /// unbounded.
    pub budget: u64,
    /// Its own chat, which each run adds a turn to. Absent in a schedule made before schedules
    /// had one, each of whose runs starts a thread of its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<ThreadId>,
    /// The chat it was asked for in, when it was set up from one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<ThreadId>,
    /// Who made it. Absent: the project's person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Principal>,
}

/// `schedule.created` as version 1 wrote it: no name, time zone or chat of its own.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScheduleCreatedV1 {
    schedule: ScheduleId,
    when: When,
    prompt: String,
    budget: u64,
    #[serde(default)]
    by: Option<Principal>,
}

impl From<ScheduleCreatedV1> for ScheduleCreated {
    /// Named by its prompt's first line, in UTC, as it ran; each run in a thread of its own.
    fn from(old: ScheduleCreatedV1) -> Self {
        let name = old
            .prompt
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty());
        Self {
            schedule: old.schedule,
            name: name.unwrap_or_default().to_owned(),
            when: old.when,
            zone: UTC.to_owned(),
            prompt: old.prompt,
            budget: old.budget,
            thread: None,
            from: None,
            by: old.by,
        }
    }
}

/// A schedule was paused: it starts no runs until it is resumed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchedulePaused {
    pub schedule: ScheduleId,
    /// Who paused it. Absent: the project's person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Principal>,
}

/// A paused schedule was resumed: its runs count again from now, none for the time it was
/// paused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleResumed {
    pub schedule: ScheduleId,
    /// Who resumed it. Absent: the project's person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Principal>,
}

/// A schedule's run that was due did not start, because its run before was still going in its
/// chat, or someone's turn was: the next starts at its own time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleSkipped {
    pub schedule: ScheduleId,
    /// When the run was due.
    pub due: At,
}

/// A schedule was deleted: it starts no more runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleDeleted {
    pub schedule: ScheduleId,
    /// Who deleted it. Absent: the project's person.
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

impl Payload for SchedulePaused {
    const TYPE: &'static str = PAUSED;
    const VERSION: u32 = 1;
}

impl Payload for ScheduleResumed {
    const TYPE: &'static str = RESUMED;
    const VERSION: u32 = 1;
}

impl Payload for ScheduleSkipped {
    const TYPE: &'static str = SKIPPED;
    const VERSION: u32 = 1;
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

pub(crate) fn decode_paused(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<SchedulePaused>> {
    (version == 1).then(|| parse(data))
}

pub(crate) fn decode_resumed(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<ScheduleResumed>> {
    (version == 1).then(|| parse(data))
}

pub(crate) fn decode_skipped(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<ScheduleSkipped>> {
    (version == 1).then(|| parse(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_schedule_made_before_schedules_had_chats_reads_as_one_in_utc_named_by_its_prompt() {
        let old = r#"{"schedule":"0192f5c1-0000-7000-8000-000000000002","when":{"cron":"0 9 * * *"},"prompt":"\n  Check the release\nthen say","budget":50000}"#;
        let raw = RawValue::from_string(old.to_owned()).unwrap();
        let read = decode_created(1, &raw).unwrap().unwrap();
        assert_eq!(read.name, "Check the release");
        assert_eq!(read.zone, "UTC");
        assert_eq!(read.when, When::Cron("0 9 * * *".into()));
        assert_eq!((read.thread, read.from, read.by), (None, None, None));
    }
}
