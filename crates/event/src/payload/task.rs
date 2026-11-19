//! `task.started`, `task.ended`, `schedule.created`, `.paused`, `.skipped` and `.deleted`: a
//! project's tasks
//! (threads that run with no one attached) and the schedules that start them. They are kept in
//! the project's task ledger, glave's own stream under a reserved id (`specs/events.md`), never
//! in a conversation.
//! `schedule.placed`: a project with schedules, in its organization's index of
//! them, until `schedule.unplaced`, as its last schedule goes.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use uuid::Uuid;

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
pub(crate) const SKIPPED: &str = "schedule.skipped";
pub(crate) const PAUSED: &str = "schedule.paused";
pub(crate) const PLACED: &str = "schedule.placed";
pub(crate) const UNPLACED: &str = "schedule.unplaced";

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
    /// When the schedule's run it was came due, so it ends that run alone, never one started
    /// in its chat since. Absent for a task no schedule started, and in version 1, which ends
    /// whatever task its thread has open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<At>,
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
    /// Whenever this five-field cron expression matches, on the clocks of the schedule's time
    /// zone.
    Cron(String),
}

/// A schedule was made, with a chat of its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleCreated {
    pub schedule: ScheduleId,
    /// What the person calls it, which its chat is named: "Morning email summary". Empty in a
    /// schedule made before schedules had chats.
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
    /// Its own chat, which each run adds a turn to. Absent only in a schedule made before
    /// schedules had chats (versions 1 and 2), which runs no more: it is set up again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<ThreadId>,
    /// The chat it was set up from, when it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<ThreadId>,
    /// Who made it: named always from v4; absent in v1 to v3, which left the server's own
    /// person implied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Principal>,
}

/// `schedule.created` as versions 1 and 2 wrote it, with no chat: 1 in UTC with no zone of its
/// own, 2 with one.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScheduleCreatedBefore {
    schedule: ScheduleId,
    when: When,
    #[serde(default)]
    zone: Option<String>,
    prompt: String,
    budget: u64,
    #[serde(default)]
    by: Option<Principal>,
}

impl From<ScheduleCreatedBefore> for ScheduleCreated {
    /// With no name or chat of its own, so it runs no more; in UTC where it had no zone.
    fn from(old: ScheduleCreatedBefore) -> Self {
        Self {
            schedule: old.schedule,
            name: String::new(),
            when: old.when,
            zone: old.zone.unwrap_or_else(|| "UTC".to_owned()),
            prompt: old.prompt,
            budget: old.budget,
            thread: None,
            from: None,
            by: old.by,
        }
    }
}

/// A schedule was paused, so it starts no runs, or resumed, its runs counting again from then,
/// none for the time it was paused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchedulePaused {
    pub schedule: ScheduleId,
    /// Paused, or resumed.
    pub paused: bool,
    /// Who did. Absent: the project's person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Principal>,
}

/// A schedule's run that was due did not start: its run before was still going in its chat, or
/// someone's turn was, or the run was refused. The next starts at its own time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleSkipped {
    pub schedule: ScheduleId,
    /// When the run was due.
    pub due: At,
    pub why: Skipped,
}

/// Why a schedule's run did not start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Skipped {
    /// The run before was still going in its chat, or someone's turn was.
    Going,
    /// It was refused: `reason` names why, as the session API's error codes name a refusal
    /// (`ARCHIVED`), so it is said in each reader's language when it is read. Empty for one
    /// recorded before refusals had reasons (v1).
    Refused { reason: String },
}

/// `schedule.skipped` as version 1 wrote it: a refusal in the server's English.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScheduleSkippedBefore {
    schedule: ScheduleId,
    due: At,
    why: SkippedBefore,
}

/// Why a run did not start, as version 1 wrote it: a refusal's English, which is read and
/// dropped, since no other language can say it.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum SkippedBefore {
    Going,
    Refused {
        #[serde(rename = "why")]
        _why: String,
    },
}

impl From<ScheduleSkippedBefore> for ScheduleSkipped {
    /// A refusal with no reason.
    fn from(old: ScheduleSkippedBefore) -> Self {
        let why = match old.why {
            SkippedBefore::Going => Skipped::Going,
            SkippedBefore::Refused { .. } => Skipped::Refused {
                reason: String::new(),
            },
        };
        Self {
            schedule: old.schedule,
            due: old.due,
            why,
        }
    }
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

/// Someone made a schedule in a project, which the store's list of where schedules are now
/// holds, ids alone: the server that fires schedules reads the task ledger of each project it
/// lists, so a schedule runs though no one asked that server for anything since it started.
/// Recorded once for each project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchedulePlaced {
    pub org: Uuid,
    pub project: Uuid,
}

/// A project's last schedule went, so the store's list of where schedules are no longer holds
/// it: what the list holds is the projects with schedules now, never every one that once had
/// one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleUnplaced {
    pub org: Uuid,
    pub project: Uuid,
}

impl Payload for TaskStarted {
    const TYPE: &'static str = STARTED;
    const VERSION: u32 = 1;
}

impl Payload for TaskEnded {
    const TYPE: &'static str = ENDED;
    const VERSION: u32 = 2;
}

impl Payload for ScheduleCreated {
    const TYPE: &'static str = CREATED;
    const VERSION: u32 = 4;
}

impl Payload for SchedulePaused {
    const TYPE: &'static str = PAUSED;
    const VERSION: u32 = 1;
}

impl Payload for ScheduleSkipped {
    const TYPE: &'static str = SKIPPED;
    const VERSION: u32 = 2;
}

impl Payload for ScheduleDeleted {
    const TYPE: &'static str = DELETED;
    const VERSION: u32 = 1;
}

impl Payload for SchedulePlaced {
    const TYPE: &'static str = PLACED;
    const VERSION: u32 = 1;
}

impl Payload for ScheduleUnplaced {
    const TYPE: &'static str = UNPLACED;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_started(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<TaskStarted>> {
    (version == 1).then(|| parse(data))
}

pub(crate) fn decode_ended(version: u32, data: &RawValue) -> Option<serde_json::Result<TaskEnded>> {
    // Version 1 had no `due`, which reads as absent: it ends whatever its thread has open.
    (version == 1 || version == 2).then(|| parse(data))
}

pub(crate) fn decode_created(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<ScheduleCreated>> {
    match version {
        1 | 2 => Some(parse::<ScheduleCreatedBefore>(data).map(ScheduleCreated::from)),
        3 => Some(parse(data)),
        // Named always from v4: none is no schedule of this version.
        4 => Some(
            parse(data).and_then(|created: ScheduleCreated| match created.by {
                Some(_) => Ok(created),
                None => Err(serde::de::Error::missing_field("by")),
            }),
        ),
        _ => None,
    }
}

pub(crate) fn decode_paused(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<SchedulePaused>> {
    (version == 1).then(|| parse(data))
}

pub(crate) fn decode_skipped(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<ScheduleSkipped>> {
    match version {
        1 => Some(parse::<ScheduleSkippedBefore>(data).map(ScheduleSkipped::from)),
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

pub(crate) fn decode_placed(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<SchedulePlaced>> {
    (version == 1).then(|| parse(data))
}

pub(crate) fn decode_unplaced(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<ScheduleUnplaced>> {
    (version == 1).then(|| parse(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_schedule_made_before_schedules_had_a_zone_reads_as_one_in_utc() {
        let old = r#"{"schedule":"0192f5c1-0000-7000-8000-000000000002","when":{"cron":"0 9 * * *"},"prompt":"Check the release","budget":50000}"#;
        let raw = RawValue::from_string(old.to_owned()).unwrap();
        let read = decode_created(1, &raw).unwrap().unwrap();
        assert_eq!(read.zone, "UTC");
        assert_eq!(read.when, When::Cron("0 9 * * *".into()));
        assert_eq!(
            (read.thread, read.name.as_str()),
            (None, ""),
            "no chat: it runs no more"
        );
    }

    #[test]
    fn a_run_refused_before_refusals_had_reasons_reads_with_none() {
        let old = r#"{"schedule":"0192f5c1-0000-7000-8000-000000000002","due":"2026-11-19T08:00:00.000Z","why":{"kind":"refused","why":"the chat is archived"}}"#;
        let raw = RawValue::from_string(old.to_owned()).unwrap();
        let read = decode_skipped(1, &raw).unwrap().unwrap();
        assert_eq!(
            read.why,
            Skipped::Refused {
                reason: String::new()
            }
        );
        let going = old.replace(
            r#"{"kind":"refused","why":"the chat is archived"}"#,
            r#"{"kind":"going"}"#,
        );
        let raw = RawValue::from_string(going).unwrap();
        assert_eq!(
            decode_skipped(1, &raw).unwrap().unwrap().why,
            Skipped::Going
        );
    }
}
