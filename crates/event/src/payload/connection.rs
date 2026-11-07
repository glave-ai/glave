//! `connection.connected` and `connection.removed`: the connections a person signs in to,
//! remote MCP servers, as MCP's authorization says, so the agent uses each as them
//! (`specs/session.md`, "Connections"). They are kept in the person's accounts ledger beside
//! their git hosts, each sealed under a data key of its own, so removing one destroys its key
//! and the tokens with it.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Payload, parse};
use crate::envelope::At;
use crate::id::CredentialId;
use crate::payload::principal::Principal;
use crate::secret::Secret;

pub(crate) const CONNECTED: &str = "connection.connected";
pub(crate) const REMOVED: &str = "connection.removed";

/// A connection was signed in to: the token its authorization server granted the person, and what
/// renewing it needs. Sealed under the connection's own key. Its `Debug` shows neither token,
/// nor who.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionConnected {
    pub connection: CredentialId,
    /// The MCP server's address, as the person's settings name it:
    /// `https://mcp.example.com/mcp`.
    pub url: String,
    /// The person's name for the account of a connector this signs in to, where they connected
    /// more than one: none for their own server's, or a connector's account never named. One
    /// sign-in is kept for each address and account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(serialize_with = "crate::secret::serialize_exposed")]
    pub token: Secret,
    /// When the token expires, when the server said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<At>,
    /// What renews the token, when the server gave one.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::secret::serialize_exposed_some"
    )]
    pub refresh: Option<Secret>,
    pub client: SignInClient,
    /// Who the person signed in as, in the service's words, where its authorization server
    /// said: `Acme Corp`. The person's own, shown only to them, never logged, and kept only
    /// here, under the connection's key, so it goes with the sign-in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub who: Option<String>,
    /// Whether the person left its account unnamed: its `account` is glave's (`Notion 2`), and
    /// clients show `who` where its name would go.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unnamed: bool,
    /// Who connected it.
    pub by: Principal,
}

impl std::fmt::Debug for ConnectionConnected {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConnectionConnected")
            .field("connection", &self.connection)
            .field("url", &self.url)
            .field("account", &self.account)
            .field("token", &self.token)
            .field("expires", &self.expires)
            .field("refresh", &self.refresh)
            .field("client", &self.client)
            .field("unnamed", &self.unnamed)
            .field("by", &self.by)
            .finish_non_exhaustive()
    }
}

/// The authorization server a sign-in went through, and the client glave registered there:
/// what renewing its token needs, beside what the server publishes, which a renewal reads
/// again. No secret: glave registers as a public client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignInClient {
    /// The authorization server's issuer, when its metadata named one: a renewal goes only
    /// to the same.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issuer: Option<String>,
    pub client_id: String,
    /// The scopes the person granted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
}

/// A connection's sign-in was removed: its tokens were destroyed with it. Content-free.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionRemoved {
    pub connection: CredentialId,
    /// Who removed it.
    pub by: Principal,
}

impl Payload for ConnectionConnected {
    const TYPE: &'static str = CONNECTED;
    const VERSION: u32 = 1;
}

impl Payload for ConnectionRemoved {
    const TYPE: &'static str = REMOVED;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_connected(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<ConnectionConnected>> {
    (version == 1).then(|| parse(data))
}

pub(crate) fn decode_removed(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<ConnectionRemoved>> {
    (version == 1).then(|| parse(data))
}
