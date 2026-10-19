//! The trust ledger's record (`specs/policy.md`, "Project trust"). `trust.decided`: what the
//! person decided about one project's items, found by its canonical path or, for a clone that
//! moved, by its `origin` remote; it replaces every earlier decision about that project. What
//! is trusted is each item's content (a server's definition), not just its name, so a
//! repository that changes a server's command asks again.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Payload, parse};
use crate::payload::principal::Principal;

pub(crate) const DECIDED: &str = "trust.decided";

/// What the person decided about one project's items, replacing what was decided before.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustDecided {
    /// The project's folder, its canonical path.
    pub path: String,
    /// Its `origin` remote, when it is a git repository with one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
    /// Each item by name. Empty: every decision about the project is forgotten.
    #[serde(default)]
    pub items: BTreeMap<String, Decision>,
    /// Who decided. Absent: the space's person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Principal>,
}

/// The person's decision on one item, and exactly what it was about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    /// The item's content as it was shown: a server's definition, a setting's value.
    pub definition: String,
    pub trusted: bool,
}

impl Payload for TrustDecided {
    const TYPE: &'static str = DECIDED;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_decided(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<TrustDecided>> {
    (version == 1).then(|| parse(data))
}
