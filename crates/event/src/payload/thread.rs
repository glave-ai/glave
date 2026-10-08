//! `thread.started`: the first event of every thread.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Payload, parse};
use crate::envelope::Seq;
use crate::id::ThreadId;

pub(crate) const TYPE: &str = "thread.started";

/// A thread begins. Always at `seq` 1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadStarted {
    pub producer: Producer,
    /// Set when this thread continues another thread's events up to and including `seq`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork: Option<ForkPoint>,
}

/// The software that started the thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Producer {
    pub name: String,
    pub version: String,
}

/// The point in a parent thread that a fork continues from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForkPoint {
    pub thread: ThreadId,
    pub seq: Seq,
}

impl Payload for ThreadStarted {
    const TYPE: &'static str = TYPE;
    const VERSION: u32 = 1;
}

pub(crate) fn decode(version: u32, data: &RawValue) -> Option<serde_json::Result<ThreadStarted>> {
    match version {
        1 => Some(parse(data)),
        _ => None,
    }
}
