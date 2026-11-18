//! The settings ledger's record of the person's time zone (`specs/session.md`, "Schedules"):
//! `zone.set`, which their browser gives the server, so a schedule they set up repeats at their
//! own times. The latest replaces every one before it; with none, UTC counts.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Payload, parse};

pub(crate) const SET: &str = "zone.set";

/// The person's time zone, by its IANA name (`Europe/Lisbon`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ZoneSet {
    pub zone: String,
}

impl Payload for ZoneSet {
    const TYPE: &'static str = SET;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_set(version: u32, data: &RawValue) -> Option<serde_json::Result<ZoneSet>> {
    (version == 1).then(|| parse(data))
}
