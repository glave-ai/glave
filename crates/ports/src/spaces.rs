//! The spaces port: each space as a record (a project, or a person's own), who owns it and who
//! else works in it, and which folders lead to it (`specs/storage.md`, "Spaces"). A folder is
//! an index into spaces, never a space's identity: a moved or re-cloned project finds its space
//! again by its `origin` remote.

mod memory;

use std::fmt;

use glave_event::{At, Principal, PrincipalId};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::scope::{OrgId, SpaceId};
use crate::tools::BoxFuture;

pub use memory::MemorySpaces;

/// The longest name a space may have, in characters.
pub const MAX_NAME: usize = 200;

/// What a space is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SpaceKind {
    /// A project: the work of its owner and members, usually tied to folders.
    Project,
    /// A person's own space (the Inbox): their one-off chats and their personal memory. Each
    /// person has exactly one in each organization.
    Personal,
}

impl SpaceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Personal => "personal",
        }
    }

    pub fn named(name: &str) -> Option<Self> {
        match name {
            "project" => Some(Self::Project),
            "personal" => Some(Self::Personal),
            _ => None,
        }
    }
}

/// A space, as its record keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Space {
    pub id: SpaceId,
    pub kind: SpaceKind,
    pub name: String,
    /// Whose it is: a person, or later a team or an agent.
    pub owner: Principal,
    /// Who else works in it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<Principal>,
    pub created_at: At,
    pub updated_at: At,
}

impl Space {
    /// Whether `principal` works in this space: its owner or one of its members.
    pub fn admits(&self, principal: &Principal) -> bool {
        self.owner == *principal || self.members.contains(principal)
    }

    /// Whether the person `person` works in this space.
    pub fn admits_person(&self, person: PrincipalId) -> bool {
        self.admits(&Principal::person(person))
    }

    /// Refuses a name no one could read: empty, or longer than [`MAX_NAME`] characters.
    pub fn check_name(name: &str) -> Result<(), SpaceError> {
        if name.trim().is_empty() || name.chars().count() > MAX_NAME {
            return Err(SpaceError::Invalid(format!(
                "a space's name is 1 to {MAX_NAME} characters, not only spaces"
            )));
        }
        Ok(())
    }
}

/// What leads from a folder to its space: its canonical path, or, for a git repository, its
/// `origin` remote, so another checkout or a clone that moved finds the same space.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FolderKey {
    Path(String),
    Remote(String),
}

impl FolderKey {
    /// The key as stored: `path:<path>` or `remote:<url>`.
    pub fn stored(&self) -> String {
        match self {
            Self::Path(path) => format!("path:{path}"),
            Self::Remote(url) => format!("remote:{url}"),
        }
    }
}

impl fmt::Display for FolderKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.stored())
    }
}

/// Why a spaces call failed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SpaceError {
    #[error("no space {0} here")]
    NotFound(SpaceId),
    /// The space exists already, or the person already has a personal space.
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Invalid(String),
    #[error("space storage failed: {0}")]
    Storage(String),
}

/// Where spaces are kept: the data store, which scopes every call by organization. Shaped as
/// if across a network: whole records in and out.
pub trait Spaces: Send + Sync {
    /// Keeps a new space. Refused when its id is taken, when its name is not readable, or
    /// when it is a personal space and its owner has one.
    fn create<'a>(&'a self, org: OrgId, space: &'a Space) -> BoxFuture<'a, Result<(), SpaceError>>;

    /// The space `space`, if the organization has it.
    fn get(&self, org: OrgId, space: SpaceId) -> BoxFuture<'_, Result<Option<Space>, SpaceError>>;

    /// The spaces `member` owns or is a member of, in id order (oldest first, for UUID v7
    /// ids), at most `limit`.
    fn list<'a>(
        &'a self,
        org: OrgId,
        member: &'a Principal,
        limit: usize,
    ) -> BoxFuture<'a, Result<Vec<Space>, SpaceError>>;

    /// Replaces the space's name, members and `updated_at` with `space`'s. Its kind, owner and
    /// creation never change.
    fn update<'a>(&'a self, org: OrgId, space: &'a Space) -> BoxFuture<'a, Result<(), SpaceError>>;

    /// The personal space of `new.owner`: the one they have, or `new`, kept now. Two callers at
    /// once get the same one.
    fn personal<'a>(
        &'a self,
        org: OrgId,
        new: &'a Space,
    ) -> BoxFuture<'a, Result<Space, SpaceError>>;

    /// The space a folder leads to: the one the first of `keys` that is known leads to, or
    /// `new`, kept now. Every key in `keys` then leads to it, unless it already led elsewhere.
    fn folder<'a>(
        &'a self,
        org: OrgId,
        keys: &'a [FolderKey],
        new: &'a Space,
    ) -> BoxFuture<'a, Result<Space, SpaceError>>;
}
