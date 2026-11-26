//! The trust ledger's record (`specs/policy.md`, "Project trust"). `trust.decided`: what the
//! person decided about one project's items, found by its canonical path or, for a clone that
//! moved, by its `origin` remote; it replaces every earlier decision about that project. What
//! is trusted is each item's content (a server's definition), not just its name, so a
//! repository that changes a server's command asks again.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Body, Payload, parse};
use crate::payload::principal::Principal;
use crate::payload::project::repository_address;

pub(crate) const DECIDED: &str = "trust.decided";

/// What the person decided about one project's items, replacing what was decided before.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustDecided {
    /// The project's folder, its canonical path.
    pub path: String,
    /// Its `origin` remote, when it is a git repository with one, named by its repository
    /// alone ([`repository_address`]): from version 2; version 1 kept it as written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
    /// Each item by name. Empty: every decision about the project is forgotten.
    #[serde(default)]
    pub items: BTreeMap<String, Decision>,
    /// Who decided. Absent: the project's person.
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
    const VERSION: u32 = 2;
}

impl TrustDecided {
    /// Version 1, of the same shape, whose remote was kept as `.git/config` wrote it, upgraded:
    /// the remote named by its repository alone ([`repository_address`]).
    fn from_v1(mut v1: Self) -> Self {
        v1.remote = v1.remote.as_deref().map(repository_address);
        v1
    }
}

/// `trust.decided`, or `None` for a type or version that is none of them. Version 1 is upgraded as
/// it is read to version 2, whose remote names the repository alone.
pub(crate) fn decode(
    kind: &str,
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<Body>> {
    Some(match (kind, version) {
        (DECIDED, 1) => parse(data).map(|v1| Body::TrustDecided(TrustDecided::from_v1(v1))),
        (DECIDED, 2) => parse(data).map(Body::TrustDecided),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A remote kept by version 1 as `.git/config` wrote it, a clone's token and all, reads as
    /// its repository alone (#1585); version 2 keeps it so.
    #[test]
    fn a_remote_kept_with_a_token_reads_as_its_repository_alone() {
        let data = |remote: &str| {
            serde_json::value::to_raw_value(&serde_json::json!({
                "path": "/code/app",
                "remote": remote,
            }))
            .unwrap()
        };
        let signed = data("https://x-access-token:a-token@github.com/o/app.git?t=a-token");
        let decided = |version, raw: &RawValue| match decode(DECIDED, version, raw) {
            Some(Ok(Body::TrustDecided(read))) => read,
            _ => panic!("a trust decided"),
        };
        let read = decided(1, &signed);
        let named = Some("https://github.com/o/app.git".to_owned());
        assert_eq!(read.remote, named);
        let kept = decided(2, &data("https://github.com/o/app.git"));
        assert_eq!(kept.remote, named);
    }
}
