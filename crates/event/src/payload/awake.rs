//! The settings ledger's record of whether the person's own computer is kept awake while glave
//! works on it (`specs/one-server.md`, owner decision 10). `awake.set`: the person's choice in
//! Settings; the latest replaces every one before it, and with none it is kept awake.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Body, Payload, parse};

pub(crate) const SET: &str = "awake.set";

/// Whether the person wants their computer kept awake while glave works on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AwakeSet {
    pub on: bool,
}

impl Payload for AwakeSet {
    const TYPE: &'static str = SET;
    const VERSION: u32 = 1;
}

/// `awake.set`, or `None` for a type or version that is none of them.
pub(crate) fn decode(
    kind: &str,
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<Body>> {
    (kind == SET && version == 1).then(|| parse(data).map(Body::AwakeSet))
}
