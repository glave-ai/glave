//! `plan.updated` and `plan.approved`: a thread's plan, the steps of its work, each with its
//! status. The latest `plan.updated` is the thread's plan, whoever made it: the model with its
//! plan tool, or the person editing it. One proposed in plan mode waits for the person until a
//! `plan.approved` follows it.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Payload, parse};
use crate::id::CallId;
use crate::payload::principal::Principal;

pub(crate) const UPDATED: &str = "plan.updated";
pub(crate) const APPROVED: &str = "plan.approved";

/// The thread's plan changed: it replaces the plan before it. Glave's own record, beside the
/// call that made it, or the person's edit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanUpdated {
    /// The steps, in order.
    pub steps: Vec<PlanStep>,
    /// The model's call that set it; absent when a person edited it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<CallId>,
    /// Whether it is a proposal: made in plan mode, it waits for the person's approval before
    /// anything is built, and a person's edit of it is one too. Absent is false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub proposed: bool,
    /// Who edited it, for an edit. Absent: the thread's `for`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Principal>,
}

/// One step of a plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanStep {
    /// What to do, in a line.
    pub step: String,
    pub status: PlanStatus,
}

/// How far a step has come: to do, doing, done.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Pending,
    InProgress,
    Completed,
}

/// The person approved the thread's proposed plan, its latest `plan.updated`: it is built from
/// now on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanApproved {
    /// Who approved it. Absent: the thread's `for`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Principal>,
}

impl Payload for PlanUpdated {
    const TYPE: &'static str = UPDATED;
    const VERSION: u32 = 1;
}

impl Payload for PlanApproved {
    const TYPE: &'static str = APPROVED;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_updated(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<PlanUpdated>> {
    (version == 1).then(|| parse(data))
}

pub(crate) fn decode_approved(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<PlanApproved>> {
    (version == 1).then(|| parse(data))
}
