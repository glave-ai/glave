//! An organization's credits as the server says them to a caller (`specs/gateway.md`,
//! "Credits"): amounts, which a client formats for its reader.

use glave_event::{At, Money, PrincipalId};
use serde::{Deserialize, Serialize};

/// An organization's credits, as much of them as the caller may see.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CreditsInfo {
    /// The organization's own account, for its Owners and Admins; none for anyone else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<CreditsAccount>,
    /// What each person spent this month, most first: everyone who spent anything, for its
    /// Owners and Admins; for anyone else, the caller alone.
    pub people: Vec<PersonSpent>,
    /// When this month began, in UTC: what the month's spending counts from.
    pub since: At,
    /// Whether the caller may set the month's limit (`credits/limit`): its Owners.
    pub can_set_limit: bool,
}

/// An organization's account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CreditsAccount {
    /// What is left: everything added, less everything charged. Below nothing only where a
    /// call to a model with no price spent past it.
    pub balance: Money,
    /// The most it may spend in a month, when anything limits it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<Money>,
    /// What it spent this month, glave's own work in it included.
    pub spent: Money,
}

/// What one person spent this month.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PersonSpent {
    pub person: PrincipalId,
    /// The name the organization knows them by, else their email, when it knows either.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub spent: Money,
    /// Whether they are the caller.
    pub you: bool,
}
