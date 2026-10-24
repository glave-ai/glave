//! The settings ledger's record of how the person's clients group their threads
//! (`specs/session.md`, "How threads are grouped"). `grouping.set`: the person's choice; the
//! latest replaces every one before it, and with none their threads are grouped by project.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Payload, parse};

pub(crate) const SET: &str = "grouping.set";

/// How a client groups the person's threads: each under its project, or each under what it is
/// doing (waiting for the person, running, idle).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Grouping {
    #[default]
    Project,
    Status,
}

/// How the person chose to have their threads grouped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupingSet {
    pub grouping: Grouping,
}

impl Payload for GroupingSet {
    const TYPE: &'static str = SET;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_set(version: u32, data: &RawValue) -> Option<serde_json::Result<GroupingSet>> {
    (version == 1).then(|| parse(data))
}
