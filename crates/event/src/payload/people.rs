//! The people ledger's records (`specs/model.md`, "Roles"): one stream of the organization's
//! own. `person.joined`: who signed in through the identity provider and what its token says
//! of them, recorded again when that changes. `role.set`: the role a person was given, the
//! latest replacing every one before it. `person.removed`: someone an admin took out of the
//! organization, who comes back only by an invite. Invites (`specs/model.md`, "Invites"): one made with
//! the SHA-256 of its code and the role it gives (`invite.created`), spent by the person who
//! opened it (`invite.accepted`), or revoked before (`invite.revoked`).

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Body, Payload, parse};
use crate::envelope::At;
use crate::id::{InviteId, PrincipalId};
use crate::payload::principal::Principal;

pub(crate) const JOINED: &str = "person.joined";
pub(crate) const ROLE_SET: &str = "role.set";
pub(crate) const REMOVED: &str = "person.removed";
pub(crate) const INVITE_CREATED: &str = "invite.created";
pub(crate) const INVITE_ACCEPTED: &str = "invite.accepted";
pub(crate) const INVITE_REVOKED: &str = "invite.revoked";

/// What a person may do in their organization. A person given no role is a Member.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum OrgRole {
    /// Uses glave within what is granted.
    #[default]
    Member,
    /// Manages people, policy, the approved connections, models, the budget and audit.
    Admin,
    /// Everything, billing and ownership included.
    Owner,
}

/// What an identity provider's token says of a person besides who they are: what a client
/// shows for them, and the groups that will add grants.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// The token's `groups` claim, as the identity provider names them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<String>,
}

/// A person who joined the organization: through its company login, as its token describes
/// them (`issuer`, `subject`); or with a glave account, by an invite, with neither.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonJoined {
    pub person: PrincipalId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issuer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(default)]
    pub profile: Profile,
}

/// The role `person` was given, and by whom.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleSet {
    pub person: PrincipalId,
    pub role: OrgRole,
    pub by: Principal,
}

/// An invite made: who may open it is whoever holds its code, of which only the hash is kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InviteCreated {
    pub invite: InviteId,
    /// The SHA-256 of its code, in hex: never the code.
    pub hash: String,
    /// The role it gives whoever opens it.
    pub role: OrgRole,
    pub by: Principal,
    /// When it stops working, opened or not.
    pub expires: At,
    /// The address its link was emailed to, when it was: whom it waits for, and the email of
    /// a glave account it makes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

/// An invite opened by `person`, and spent: their role is raised to `role`, the invite's,
/// when it is higher, never lowered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InviteAccepted {
    pub invite: InviteId,
    pub person: PrincipalId,
    pub role: OrgRole,
}

/// Someone taken out of the organization by an admin (`specs/model.md`, "Identity"): their
/// role and record go, and they come back only by an invite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonRemoved {
    pub person: PrincipalId,
    pub by: Principal,
}

/// An invite revoked before anyone opened it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InviteRevoked {
    pub invite: InviteId,
    pub by: Principal,
}

impl Payload for PersonJoined {
    const TYPE: &'static str = JOINED;
    const VERSION: u32 = 1;
}

impl Payload for RoleSet {
    const TYPE: &'static str = ROLE_SET;
    const VERSION: u32 = 1;
}

impl Payload for PersonRemoved {
    const TYPE: &'static str = REMOVED;
    const VERSION: u32 = 1;
}

impl Payload for InviteCreated {
    const TYPE: &'static str = INVITE_CREATED;
    const VERSION: u32 = 1;
}

impl Payload for InviteAccepted {
    const TYPE: &'static str = INVITE_ACCEPTED;
    const VERSION: u32 = 1;
}

impl Payload for InviteRevoked {
    const TYPE: &'static str = INVITE_REVOKED;
    const VERSION: u32 = 1;
}

/// The people ledger's events, or `None` for a type or version that is none of them.
pub(crate) fn decode(
    kind: &str,
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<Body>> {
    if version != 1 {
        return None;
    }
    Some(match kind {
        JOINED => parse(data).map(Body::PersonJoined),
        ROLE_SET => parse(data).map(Body::RoleSet),
        REMOVED => parse(data).map(Body::PersonRemoved),
        INVITE_CREATED => parse(data).map(Body::InviteCreated),
        INVITE_ACCEPTED => parse(data).map(Body::InviteAccepted),
        INVITE_REVOKED => parse(data).map(Body::InviteRevoked),
        _ => return None,
    })
}
