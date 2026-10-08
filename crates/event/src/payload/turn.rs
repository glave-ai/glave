//! `turn.started` and `turn.ended`: the bounds of one turn.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use super::model::ModelRef;
use crate::blob::BlobRef;
use crate::codec::{Payload, parse};

pub(crate) const STARTED: &str = "turn.started";
pub(crate) const ENDED: &str = "turn.ended";

/// A turn begins. `config` points to the turn's full configuration (system prompt, tools,
/// parameters), stored once by hash so every turn can be reproduced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnStarted {
    pub model: ModelRef,
    pub config: BlobRef,
}

impl Payload for TurnStarted {
    const TYPE: &'static str = STARTED;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_started(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<TurnStarted>> {
    match version {
        1 => Some(parse(data)),
        _ => None,
    }
}

/// A turn ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnEnded {
    pub outcome: Outcome,
}

/// How a turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Completed,
    Cancelled,
    Failed,
    Interrupted,
    LimitReached,
}

impl Payload for TurnEnded {
    const TYPE: &'static str = ENDED;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_ended(version: u32, data: &RawValue) -> Option<serde_json::Result<TurnEnded>> {
    match version {
        1 => Some(parse(data)),
        _ => None,
    }
}
