//! `goal.set` and `goal.ended`: a goal keeps a thread working across turns until it is met,
//! stopped, or out of budget. The latest `goal.set` with no `goal.ended` after it is the
//! thread's active goal.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Payload, parse};
use crate::payload::principal::Principal;

pub(crate) const SET: &str = "goal.set";
pub(crate) const ENDED: &str = "goal.ended";

/// The person set a goal, or changed it: it replaces any goal the thread had.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalSet {
    /// What to achieve, in the person's words.
    pub objective: String,
    /// A command whose success means the goal is met, such as `cargo test`. Absent: the
    /// model decides when the goal is met.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<String>,
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
}

/// Why a goal ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalReason {
    /// The check command passed: met, as the computer decided.
    MetByCheck,
    /// The model said the goal is met; there was no check.
    MetPerModel,
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
        matches!(self, Self::MetByCheck | Self::MetPerModel)
    }
}

impl Payload for GoalSet {
    const TYPE: &'static str = SET;
    const VERSION: u32 = 1;
}

impl Payload for GoalEnded {
    const TYPE: &'static str = ENDED;
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
