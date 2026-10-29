//! The organization's people and their roles, as its admins list and change them
//! (`specs/model.md`, "Roles").

use glave_event::{At, InviteId, OrgRole, Principal, PrincipalId};
use glave_ports::OrgId;
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
    /// Who they are to the company login: its id for them; none with a glave account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// The identity provider's groups they were in when they last signed in.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<String>,
    pub role: OrgRole,
    /// When they first signed in; none for the person on the server's computer, who never
    /// joins: they own its organization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub joined: Option<At>,
    /// Whether they are the person on the server's computer, its organization's owner always.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub local: bool,
    /// Whether they are the caller.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub you: bool,
    /// The roles the caller may give them now, theirs among them; none when the caller may
    /// not change their role.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<OrgRole>,
    /// Whether the caller may take them out of the organization.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub removable: bool,
}

/// An invite waiting to be opened. Its code is never shown again after it is made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct InviteInfo {
    pub id: InviteId,
    /// The role it gives whoever opens it.
    pub role: OrgRole,
    /// Who made it.
    pub by: Principal,
    pub made: At,
    /// When it stops working, opened or not.
    pub expires: At,
    /// The address its link was emailed to, when it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

/// How the one who holds an invite joins the organization it is to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JoinWay {
    /// Not signed in: by making a passkey, a new glave account.
    Passkey,
    /// Not signed in: by signing in through the organization's company login.
    Company,
    /// Signed in: by accepting it, the organization one more of theirs.
    Accept,
    /// Signed in, and in the organization already: nothing to join.
    Member,
}

/// What an invite's link opens, for whoever holds it: never its code, nor whom it was sent to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ShownInvite {
    /// The organization it is to, for a client to work in once its person joins.
    #[schemars(with = "String")]
    pub organization: OrgId,
    /// The role it gives.
    pub role: OrgRole,
    /// Who made it, by the name the organization knows them by, else their email; none for
    /// the person who runs this glave, who has neither here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    pub joining: JoinWay,
}

impl Request {
    /// What a caller's role must permit for them to make it, when it is an admin method.
    pub(crate) fn needs(&self) -> Option<Permit> {
        match self {
            Self::PeopleList { .. } | Self::RoleSet { .. } | Self::PersonRemove { .. } => {
                Some(Permit::People)
            }
            Self::InviteCreate { .. } | Self::InviteList {} | Self::InviteRevoke { .. } => {
                Some(Permit::Invites)
            }
            _ => None,
        }
    }
}
