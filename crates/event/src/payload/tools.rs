//! `tools.loaded`: connected tools a thread's model found with `tool_search`, on offer for the
//! rest of the thread.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Payload, parse};

pub(crate) const LOADED: &str = "tools.loaded";

/// Tools joined the end of the thread's list, in this order. Glave's own record, beside the
/// call that found them; every `tools.loaded` so far is what the thread has loaded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolsLoaded {
    /// Their names, as the model calls them.
    pub tools: Vec<String>,
}

impl Payload for ToolsLoaded {
    const TYPE: &'static str = LOADED;
    const VERSION: u32 = 1;
}

pub(crate) fn decode(version: u32, data: &RawValue) -> Option<serde_json::Result<ToolsLoaded>> {
    (version == 1).then(|| parse(data))
}
