//! Connections as clients see them (`connection/*`): the remote MCP server, by its address and
//! the name the person's settings give it, whether the person is signed in to it and the
//! sign-in still works, never a token, and how much it was used; or one glave suggests, off
//! until they turn it on. And the page a person opens to sign in.

use serde::{Deserialize, Serialize};

use super::Told;

/// A connection's sign-in to its remote MCP server, a server that asks for one, or a server
/// glave suggests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ConnectionInfo {
    /// The MCP server's address: `https://mcp.example.com/mcp`.
    pub url: String,
    /// What the person's settings call the server at that address, when they name one; for a
    /// suggestion, what they will call it once it is on, which turns it on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub status: ConnectionStatus,
    /// What it is for, when glave suggests it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested: Option<Suggested>,
    /// How much it was used, once it is in the person's settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

/// Whether the person is signed in, and the sign-in still works; or, for a server glave
/// suggests, whether it is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionStatus {
    /// Its token works, or renews before it expires.
    SignedIn,
    /// Its token expired with nothing to renew it: the person signs in again.
    Expired,
    /// Not signed in: one of the person's remote servers, which asks them to sign in.
    SignedOut,
    /// In the person's settings and working with no sign-in: today, one glave suggests, once
    /// it is turned on.
    On,
    /// One glave suggests, not in the person's settings: `connection/add` turns it on.
    Off,
}

/// A connection glave suggests, as the person reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Suggested {
    /// What it does: "Search the web".
    pub title: Told,
    /// What turning it on means: what it costs, and who is sent what.
    pub detail: Told,
}

/// How much a connection was used since this month began (UTC).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Usage {
    /// The calls the person's agent asked of its tools, refused ones too.
    pub calls: u64,
    /// The calls, as the person reads them: "412 calls this month".
    pub sentence: Told,
}

/// Signing in to a remote server in the browser: the person opens `url`, its authorization server's
/// page, then the client waits on `flow` with `connection/poll`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SignInLink {
    /// Names this sign-in to `connection/poll`.
    pub flow: String,
    /// The page where the person signs in and approves glave.
    pub url: String,
}
