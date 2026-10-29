//! Trusting a project from an app (`specs/session.md`, "Project trust"): what a thread's
//! project brings that waits for the person on this computer to trust it, and their answer.

use serde::{Deserialize, Serialize};

use crate::protocol::Told;

/// What a thread's project brings that runs only once the person on this computer trusts it:
/// its tool servers, its hooks and settings that loosen how its commands run
/// (`specs/policy.md`, "Project trust").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TrustAsk {
    /// Names exactly what is asked: `trust/answer` names it back, so only what the person was
    /// shown is trusted.
    pub ask: String,
    /// Each thing in plain words, in the order the project lists them.
    pub items: Vec<TrustItem>,
}

/// One thing a project brings, in plain words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TrustItem {
    /// What it does, in a sentence: `Runs a tool server`.
    pub text: Told,
    /// The thing itself, when there is one to show: the command it runs, the address, the
    /// model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// The person's answer to a project's ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TrustChoice {
    /// Everything the ask listed is on from the thread's next turn, and the answer is kept.
    Trust,
    /// It stays off, nothing is kept, and this thread stops asking.
    NotNow,
}
