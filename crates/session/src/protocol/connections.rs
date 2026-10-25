//! Connections as clients see them (`connection/*`): the remote MCP server, by its address and
//! the name the person's settings give it, whether the person is signed in to it and the
//! sign-in still works, never a token, and how much it was used. And the page a person opens to
//! sign in. Web search (`search/*`) too, a connection under the hood that the person sees as a
//! setting of its own.

use serde::{Deserialize, Serialize};

use super::Told;

/// A connection's sign-in to its remote MCP server, or a server that asks for one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ConnectionInfo {
    /// The MCP server's address: `https://mcp.example.com/mcp`.
    pub url: String,
    /// What the person's settings call the server at that address, when they name one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub status: ConnectionStatus,
    /// How much it was used, once it is in the person's settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
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

/// How much a connection was used since this month began (UTC).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Usage {
    /// The calls the person's agent asked of its tools, refused ones too.
    pub calls: u64,
    /// The calls, as the person reads them: "412 calls this month", or for web search "412
    /// searches this month".
    pub sentence: Told,
}

/// Whether the person's agent may search the web (`specs/session.md`, "Web search").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SearchInfo {
    /// Not the person's to choose here: their organization chooses its search among its
    /// approved connections, or this server keeps no connections of the person's own.
    Never,
    /// The person's to turn on or off: on when `on`; `detail` says who searches and what it
    /// costs; `usage`, while it is on, how much it searched this month.
    Chosen {
        on: bool,
        detail: Told,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
    },
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
