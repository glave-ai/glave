//! The people ledger's records (`specs/model.md`, "Roles"): one stream of the organization's
//! own. `person.joined`: who signed in through the identity provider and what its token says
//! of them, recorded again when that changes. `role.set`: the role a person was given, the
//! latest replacing every one before it.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Payload, parse};
use crate::id::PrincipalId;
use crate::payload::principal::Principal;

pub(crate) const JOINED: &str = "person.joined";
pub(crate) const ROLE_SET: &str = "role.set";

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

/// A person who signed in through the identity provider, as its token describes them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonJoined {
    pub person: PrincipalId,
    pub issuer: String,
    pub subject: String,
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

impl Payload for PersonJoined {
    const TYPE: &'static str = JOINED;
    const VERSION: u32 = 1;
}

impl Payload for RoleSet {
    const TYPE: &'static str = ROLE_SET;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_joined(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<PersonJoined>> {
    (version == 1).then(|| parse(data))
}

pub(crate) fn decode_role_set(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<RoleSet>> {
    (version == 1).then(|| parse(data))
}
