//! `gateway.tool.called`: a gateway serving many people forwarded one call of a catalog tool
//! to the server that offers it. Content-free: the server, the tool, how long it took and
//! whether it failed, never the arguments or the result.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Payload, parse};

pub(crate) const TOOL_CALLED: &str = "gateway.tool.called";

/// A catalog tool was called through the gateway.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayToolCalled {
    /// The catalog server that offers the tool, as the gateway's policy names it.
    pub server: String,
    /// The tool's name at that server.
    pub tool: String,
    /// True when the call failed: the server said so, or could not be reached.
    pub is_error: bool,
    /// How long the server took, in milliseconds.
    pub millis: u64,
}

impl Payload for GatewayToolCalled {
    const TYPE: &'static str = TOOL_CALLED;
    const VERSION: u32 = 1;
}

pub(crate) fn decode(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<GatewayToolCalled>> {
    match version {
        1 => Some(parse(data)),
        _ => None,
    }
}
