//! The organization's people and their roles, as its admins list and change them
//! (`specs/model.md`, "Roles").

use glave_event::{At, OrgRole, PrincipalId};
use serde::{Deserialize, Serialize};

use super::Request;
use crate::people::Permit;

/// A person who signed in through the organization's identity provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PersonInfo {
    pub id: PrincipalId,
    /// Their name, as the identity provider's token said it, when it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Their email, as the identity provider's token said it, when it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// Who they are to the identity provider: its id for them.
    pub subject: String,
    /// The identity provider's groups they were in when they last signed in.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<String>,
    pub role: OrgRole,
    /// When they first signed in.
    pub joined: At,
}

impl Request {
    /// What a caller's role must permit for them to make it, when it is an admin method.
    pub(crate) fn needs(&self) -> Option<Permit> {
        match self {
            Self::PeopleList { .. } | Self::RoleSet { .. } => Some(Permit::People),
            _ => None,
        }
    }
}
