//! Connections as clients see them (`connection/*`): the remote MCP server, by its address and
//! the name the person's settings give it, and whether the person is signed in to it and the
//! sign-in still works, never a token. And the page a person opens to sign in.

use serde::{Deserialize, Serialize};

/// A connection's sign-in to its remote MCP server, or a server that asks for one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ConnectionInfo {
    /// The MCP server's address: `https://mcp.example.com/mcp`.
    pub url: String,
    /// What the person's settings call the server at that address, when they name one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub status: ConnectionStatus,
}

/// Whether the person is signed in, and the sign-in still works.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionStatus {
    /// Its token works, or renews before it expires.
    SignedIn,
    /// Its token expired with nothing to renew it: the person signs in again.
    Expired,
    /// Not signed in: one of the person's remote servers, which asks them to sign in.
    SignedOut,
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
