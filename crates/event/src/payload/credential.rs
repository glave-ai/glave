//! `credential.connected` and `credential.removed`: the git hosts a person connects, so git and
//! the host's own command-line tool reach them with the person's token, handed over for each
//! use (`specs/computers.md`, rule 8). They are kept in the person's accounts ledger, beside
//! their model accounts, each sealed under a data key of its own, so removing one destroys
//! its key and the token with it.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Payload, parse};
use crate::envelope::At;
use crate::id::CredentialId;
use crate::payload::principal::Principal;
use crate::secret::Secret;

pub(crate) const CONNECTED: &str = "credential.connected";
pub(crate) const REMOVED: &str = "credential.removed";

/// Which forge a git host runs, as far as glave could tell: what its API and its
/// command-line tool are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Forge {
    /// GitHub, or GitHub Enterprise Server: `gh`.
    Github,
    /// GitLab, its own or self-managed: `glab`.
    Gitlab,
    /// Gitea or Forgejo: `tea`.
    Gitea,
    /// Bitbucket Cloud.
    Bitbucket,
    /// Any other host git reaches over HTTPS.
    Other,
}

/// A git host was connected: the person's token for it, checked with the host when it has a
/// cheap way to. Sealed under the credential's own key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialConnected {
    pub credential: CredentialId,
    pub forge: Forge,
    /// Where the host is, as git names it: `https://github.com`.
    pub url: String,
    /// The user name git sends with the token.
    pub username: String,
    #[serde(serialize_with = "crate::secret::serialize_exposed")]
    pub token: Secret,
    /// Whom the host said the token is for, when it said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
    /// What the host said the token may do, when it said: a classic GitHub token's scopes
    /// (`repo`), or a GitLab token's (`write_repository`). A GitHub App's are asked of GitHub
    /// each time, since the person may install it on more repositories after signing in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scopes: Option<Vec<String>>,
    /// How the token is renewed, for one that expires (a GitHub App's lasts eight hours).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renewal: Option<Renewal>,
    /// Who connected it.
    pub by: Principal,
}

/// How a token that expires is renewed: before `expires`, `refresh` gets a new token, and a
/// new refresh token with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Renewal {
    /// When the token expires.
    pub expires: At,
    #[serde(serialize_with = "crate::secret::serialize_exposed")]
    pub refresh: Secret,
    /// The GitHub App that issued it, the only one that can renew it. Empty on a sign-in from
    /// before glave kept this, which no app renews: its person signs in again.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub client_id: String,
}

/// A git host was removed: its token was destroyed with it. Content-free.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialRemoved {
    pub credential: CredentialId,
    /// The credential that renewed it, in its place: the same sign-in, so a client naming
    /// this one still names it. Kept here, since this event stays readable once the token's
    /// key is destroyed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renewed: Option<CredentialId>,
    /// Who removed it.
    pub by: Principal,
}

impl Payload for CredentialConnected {
    const TYPE: &'static str = CONNECTED;
    const VERSION: u32 = 1;
}

impl Payload for CredentialRemoved {
    const TYPE: &'static str = REMOVED;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_connected(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<CredentialConnected>> {
    (version == 1).then(|| parse(data))
}

pub(crate) fn decode_removed(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<CredentialRemoved>> {
    (version == 1).then(|| parse(data))
}
