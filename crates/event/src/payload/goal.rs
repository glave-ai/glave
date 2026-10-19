//! `goal.set`, `goal.ended` and `goal.resumed`: a goal keeps a thread working across turns
//! until it is met, paused, stopped, or out of budget. The latest `goal.set` with no
//! `goal.ended` after it, or with a `goal.resumed` after a `paused` end, is the thread's active
//! goal.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Payload, parse};
use crate::payload::principal::Principal;

pub(crate) const SET: &str = "goal.set";
pub(crate) const ENDED: &str = "goal.ended";
pub(crate) const RESUMED: &str = "goal.resumed";

/// The person set a goal, or changed it: it replaces any goal the thread had.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalSet {
    /// What to achieve, in the person's words, with how to verify it if they said.
    pub objective: String,
    /// Tokens, in and out, the goal's turns may use before it stops.
    pub token_budget: u64,
    /// Who set it. Absent: the thread's `for`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Principal>,
}

/// The thread's goal ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalEnded {
    pub reason: GoalReason,
    /// Tokens, in and out, the goal's turns used.
    pub tokens: u64,
    /// Who ended it: for `stopped`, the person, absent meaning the thread's `for`; glave for
    /// every other reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Principal>,
    /// With `paused`, the model's words to the person: what it needs from them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Why a goal ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GoalReason {
    /// The model verified the goal is met and said so with `update_goal`.
    Met,
    /// The model cannot go on without the person, and said why with `update_goal`. The goal
    /// can be resumed.
    Paused,
    /// The person stopped it, or cancelled its turn.
    Stopped,
    /// Its token budget ran out.
    Budget,
    /// Three turns in a row ended without the model calling a tool: no progress.
    Stalled,
}

impl GoalReason {
    /// Whether the goal was met.
    pub fn met(self) -> bool {
        self == Self::Met
    }
}

/// The person resumed a paused goal: it is active again, its tokens counting on from where it
/// paused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalResumed {
    /// Who resumed it. Absent: the thread's `for`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Principal>,
}

impl Payload for GoalSet {
    const TYPE: &'static str = SET;
    const VERSION: u32 = 1;
}

impl Payload for GoalEnded {
    const TYPE: &'static str = ENDED;
    const VERSION: u32 = 1;
}

impl Payload for GoalResumed {
    const TYPE: &'static str = RESUMED;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_set(version: u32, data: &RawValue) -> Option<serde_json::Result<GoalSet>> {
    match version {
        1 => Some(parse(data)),
        _ => None,
    }
}

pub(crate) fn decode_ended(version: u32, data: &RawValue) -> Option<serde_json::Result<GoalEnded>> {
    match version {
        1 => Some(parse(data)),
        _ => None,
    }
}

pub(crate) fn decode_resumed(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<GoalResumed>> {
    match version {
        1 => Some(parse(data)),
        _ => None,
    }
}
