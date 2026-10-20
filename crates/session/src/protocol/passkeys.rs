//! Signing in from elsewhere with passkeys (`specs/session.md`, "Signing in"): how a server
//! says it signs browsers in, and a passkey as a client lists it.

use glave_event::At;
use serde::{Deserialize, Serialize};

/// A way a browser signs in to the server, as `server/discover` says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SignInWay {
    /// A link glave on the server's own computer makes, good once, for a browser there.
    Link,
    /// A passkey, at the server's public origin; a setup link from glave on its computer makes
    /// the first.
    Passkey,
}

/// A passkey that signs the person in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PasskeySummary {
    pub id: String,
    /// What it is called: the name the browser gave it, else when it was added.
    pub name: String,
    pub added: At,
    /// When it last signed a browser in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used: Option<At>,
}
