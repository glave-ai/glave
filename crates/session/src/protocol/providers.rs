//! Model accounts as clients see them (`provider/connect`, `provider/list`, `provider/remove`):
//! which service, and a hint to tell keys apart, never the key.

use glave_event::{ProviderId, ProviderKind};
use serde::{Deserialize, Serialize};

/// A model account the person connected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProviderInfo {
    pub id: ProviderId,
    pub kind: ProviderKind,
    /// The server's API, for a `compatible` account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The models it offers, the one glave uses by default first.
    pub models: Vec<String>,
}
