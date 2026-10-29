//! The settings ledger's record of whether glave remembers for the person (`specs/memory.md`,
//! "Defaults and policy"). `memory.set`: the person's choice in Settings; the latest replaces
//! every one before it, and with none memory is on.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Payload, parse};

pub(crate) const SET: &str = "memory.set";

/// Whether the person wants glave to read and write memory in their threads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemorySet {
    pub on: bool,
}

impl Payload for MemorySet {
    const TYPE: &'static str = SET;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_set(version: u32, data: &RawValue) -> Option<serde_json::Result<MemorySet>> {
    (version == 1).then(|| parse(data))
}
