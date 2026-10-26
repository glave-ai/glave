//! `plan.updated`: a thread's plan, the steps of its work, each with its status. The latest one
//! is the thread's plan.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Payload, parse};
use crate::id::CallId;

pub(crate) const UPDATED: &str = "plan.updated";

/// The thread's plan changed: it replaces the plan before it. Glave's own record, beside the
/// call that made it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanUpdated {
    /// The steps, in order.
    pub steps: Vec<PlanStep>,
    /// The model's call that set it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<CallId>,
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

impl Payload for PlanUpdated {
    const TYPE: &'static str = UPDATED;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_updated(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<PlanUpdated>> {
    (version == 1).then(|| parse(data))
}
