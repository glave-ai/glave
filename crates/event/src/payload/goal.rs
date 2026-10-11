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

/// Version 1, which could name a check command glave ran after each turn.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GoalSetV1 {
    objective: String,
    #[serde(default)]
    check: Option<String>,
    token_budget: u64,
    #[serde(default)]
    by: Option<Principal>,
}

impl From<GoalSetV1> for GoalSet {
    /// A check becomes words in the objective, which is where a person now says how the goal
    /// is verified: the model, resuming an old goal, still knows what "met" meant.
    fn from(old: GoalSetV1) -> Self {
        let objective = match old.check {
            Some(check) => format!(
                "{}\n\nThe goal is met when `{check}` passes.",
                old.objective
            ),
            None => old.objective,
        };
        Self {
            objective,
            token_budget: old.token_budget,
            by: old.by,
        }
    }
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
    /// With `paused`, the model's words to the person: what it needs from them. Since
    /// version 2.
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

/// Version 1, when a goal could be met by its check or by the model's word.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GoalEndedV1 {
    reason: GoalReasonV1,
    tokens: u64,
    #[serde(default)]
    by: Option<Principal>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum GoalReasonV1 {
    MetByCheck,
    MetPerModel,
    Stopped,
    Budget,
    Stalled,
}

impl From<GoalEndedV1> for GoalEnded {
    fn from(old: GoalEndedV1) -> Self {
        let reason = match old.reason {
            GoalReasonV1::MetByCheck | GoalReasonV1::MetPerModel => GoalReason::Met,
            GoalReasonV1::Stopped => GoalReason::Stopped,
            GoalReasonV1::Budget => GoalReason::Budget,
            GoalReasonV1::Stalled => GoalReason::Stalled,
        };
        Self {
            reason,
            tokens: old.tokens,
            by: old.by,
            note: None,
        }
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
    const VERSION: u32 = 2;
}

impl Payload for GoalEnded {
    const TYPE: &'static str = ENDED;
    const VERSION: u32 = 2;
}

impl Payload for GoalResumed {
    const TYPE: &'static str = RESUMED;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_set(version: u32, data: &RawValue) -> Option<serde_json::Result<GoalSet>> {
    match version {
        1 => Some(parse::<GoalSetV1>(data).map(GoalSet::from)),
        2 => Some(parse(data)),
        _ => None,
    }
}

pub(crate) fn decode_ended(version: u32, data: &RawValue) -> Option<serde_json::Result<GoalEnded>> {
    match version {
        1 => Some(parse::<GoalEndedV1>(data).map(GoalEnded::from)),
        2 => Some(parse(data)),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(json: &str) -> Box<RawValue> {
        RawValue::from_string(json.to_owned()).unwrap()
    }

    #[test]
    fn a_v1_check_becomes_words_in_the_objective() {
        let set = decode_set(
            1,
            &raw(r#"{"objective":"fix it","check":"cargo test","token_budget":9}"#),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            set.objective,
            "fix it\n\nThe goal is met when `cargo test` passes."
        );
        assert_eq!(set.token_budget, 9);
        let plain = decode_set(1, &raw(r#"{"objective":"fix it","token_budget":9}"#))
            .unwrap()
            .unwrap();
        assert_eq!(plain.objective, "fix it");
    }

    #[test]
    fn both_v1_ways_of_meeting_a_goal_become_met() {
        for old in ["met_by_check", "met_per_model"] {
            let json = format!(r#"{{"reason":"{old}","tokens":5}}"#);
            let ended = decode_ended(1, &raw(&json)).unwrap().unwrap();
            assert_eq!(ended.reason, GoalReason::Met);
            assert_eq!(ended.note, None);
        }
        let stalled = decode_ended(1, &raw(r#"{"reason":"stalled","tokens":5}"#))
            .unwrap()
            .unwrap();
        assert_eq!(stalled.reason, GoalReason::Stalled);
    }

    #[test]
    fn v2_has_no_check_and_no_v1_reasons() {
        let check = r#"{"objective":"x","check":"true","token_budget":1}"#;
        assert!(decode_set(2, &raw(check)).unwrap().is_err());
        let old = r#"{"reason":"met_by_check","tokens":1}"#;
        assert!(decode_ended(2, &raw(old)).unwrap().is_err());
    }
}
