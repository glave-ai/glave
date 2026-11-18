//! The people ledger's records (`specs/model.md`, "Roles"): one stream of the organization's
//! own. `person.joined`: who signed in through the identity provider and what its token says
//! of them, recorded again when that changes. `role.set`: the role a person was given, the
//! latest replacing every one before it. `person.removed`: someone an admin took out of the
//! organization, who comes back only by an invite. Invites (`specs/model.md`, "Invites"): one made with
//! the SHA-256 of its code and the role it gives (`invite.created`), spent by the person who
//! opened it (`invite.accepted`), or revoked before (`invite.revoked`). `organization.made`: what
//! glave made the organization as, first in its ledger: a person's personal space, as they signed
//! up, or an organization someone created to share.

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
pub(crate) const MADE: &str = "organization.made";

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

/// What glave made an organization as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrganizationKind {
    /// A person's own, made as they signed up: never shared, so it takes no invites.
    Personal,
    /// One someone created to share with people they invite.
    Shared,
}

/// An organization glave made, and as what: the first event of its people ledger, in the same
/// append as its first Owner's `person.joined` and `role.set`. An organization glave made by
/// sign-up before this was recorded is personal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrganizationMade {
    pub kind: OrganizationKind,
}

/// What an identity provider's token says of a person besides who they are, or what someone
/// with a glave account gave: what a client shows for them, and the groups that will add
/// grants.
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
    /// Whether its code went to `email` alone, never back to whoever made it (v2): so whoever
    /// opens it holds that mailbox, and the account it makes gets back in with the address.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mailed_only: bool,
}

/// `invite.created` at version 1, when every invite's code was answered to whoever made it,
/// emailed or not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InviteCreatedV1 {
    invite: InviteId,
    hash: String,
    role: OrgRole,
    by: Principal,
    expires: At,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    email: Option<String>,
}

impl From<InviteCreatedV1> for InviteCreated {
    fn from(old: InviteCreatedV1) -> Self {
        Self {
            invite: old.invite,
            hash: old.hash,
            role: old.role,
            by: old.by,
            expires: old.expires,
            email: old.email,
            mailed_only: false,
        }
    }
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
    const VERSION: u32 = 2;
}

impl Payload for InviteAccepted {
    const TYPE: &'static str = INVITE_ACCEPTED;
    const VERSION: u32 = 1;
}

impl Payload for InviteRevoked {
    const TYPE: &'static str = INVITE_REVOKED;
    const VERSION: u32 = 1;
}

impl Payload for OrganizationMade {
    const TYPE: &'static str = MADE;
    const VERSION: u32 = 1;
}

/// The people ledger's events, or `None` for a type or version that is none of them.
pub(crate) fn decode(
    kind: &str,
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<Body>> {
    Some(match (kind, version) {
        (JOINED, 1) => parse(data).map(Body::PersonJoined),
        (ROLE_SET, 1) => parse(data).map(Body::RoleSet),
        (REMOVED, 1) => parse(data).map(Body::PersonRemoved),
        (INVITE_CREATED, 1) => {
            parse::<InviteCreatedV1>(data).map(|old| Body::InviteCreated(old.into()))
        }
        (INVITE_CREATED, 2) => parse(data).map(Body::InviteCreated),
        (INVITE_ACCEPTED, 1) => parse(data).map(Body::InviteAccepted),
        (INVITE_REVOKED, 1) => parse(data).map(Body::InviteRevoked),
        (MADE, 1) => parse(data).map(Body::OrganizationMade),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use crate::{Body, RawEvent, decode};

    /// An invite made before v2 had its code answered to whoever made it, emailed or not: it
    /// reads as shown to them, so joining by it proves nothing of the address.
    #[test]
    fn an_emailed_invite_from_before_v2_reads_as_shown_to_its_maker() {
        let v1 = r#"{"glave":1,"id":"0192f5c2-7000-7000-8000-000000000093","thread":"00000000-0000-7000-8000-000000000009","seq":3,"at":"2026-10-29T09:10:00.000Z","type":"invite.created","v":1,"data":{"invite":"0192f5c7-0000-7000-8000-000000000001","hash":"9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08","role":"admin","by":{"kind":"person","id":"0192f5c0-11aa-7e21-8c3d-5f9b2e4a6c11"},"expires":"2026-11-05T09:10:00.000Z","email":"ada@example.com"}}"#;
        let Body::InviteCreated(created) = decode(RawEvent::from_json(v1).unwrap()).unwrap().body
        else {
            panic!("an invite");
        };
        assert_eq!(created.email.as_deref(), Some("ada@example.com"));
        assert!(!created.mailed_only);
    }
}
