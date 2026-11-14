//! The settings ledger's record of whether the person's agent searches the web
//! (`specs/session.md`, "Web search"). `search.set`: the person's choice in Settings; the latest
//! replaces every one before it, and with none it searches where the server can.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Payload, parse};

pub(crate) const SET: &str = "search.set";

/// Whether the person wants their agent to search the web.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchSet {
    pub on: bool,
}

impl Payload for SearchSet {
    const TYPE: &'static str = SET;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_set(version: u32, data: &RawValue) -> Option<serde_json::Result<SearchSet>> {
    (version == 1).then(|| parse(data))
}
