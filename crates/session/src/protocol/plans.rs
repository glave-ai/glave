//! A thread's plan, as `thread/plan` gives it.

use glave_event::PlanStep;
use serde::{Deserialize, Serialize};

use super::Told;

/// A thread's plan: its latest `plan.updated`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PlanInfo {
    /// The steps, in order, each with its status.
    pub steps: Vec<PlanStep>,
    /// How far it has come, in a sentence: "Plan: 2 of 5 steps done".
    pub sentence: Told,
}
