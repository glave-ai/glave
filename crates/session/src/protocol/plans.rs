//! A thread's plan, as `thread/plan` and `plan/edit` give it.

use glave_event::PlanStep;
use serde::{Deserialize, Serialize};

use super::Told;

/// A thread's plan: its latest `plan.updated`, by the model or the person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PlanInfo {
    /// The steps, in order, each with its status.
    pub steps: Vec<PlanStep>,
    /// Whether it waits for the person: proposed in plan mode and not approved yet, to build
    /// (`plan/build`), edit (`plan/edit`) or change with a message.
    pub proposed: bool,
    /// What it is, in a sentence: "Proposed plan: 5 steps", "Plan: 2 of 5 steps done".
    pub sentence: Told,
}
