//! The projects port: each project as a record (a project, or a person's own), who owns it and who
//! else works in it, and which folders lead to it (`specs/storage.md`, "Projects"). A folder is
//! an index into projects, never a project's identity: a moved or re-cloned project finds its project
//! again by its `origin` remote.

use std::fmt;

use glave_event::{At, Principal, PrincipalId};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::scope::{OrgId, ProjectId};
use crate::tools::BoxFuture;

/// The longest name a project may have, in characters.
pub const MAX_NAME: usize = 200;

/// What a project is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProjectKind {
    /// A project: the work of its owner and members, usually tied to folders.
    Project,
    /// A person's own project: their one-off chats and their personal memory. Each
    /// person has exactly one in each organization.
    Personal,
}

impl ProjectKind {
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

/// A project, as its record keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Project {
    pub id: ProjectId,
    pub kind: ProjectKind,
    pub name: String,
    /// Whose it is: a person, or later a team or an agent.
    pub owner: Principal,
    /// Who else works in it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<Principal>,
    pub created_at: At,
    pub updated_at: At,
    /// How many times the record was updated since it was kept. An update names the revision
    /// it read and is refused when another came first, so no writer loses another's change.
    /// The store's own count, never sent.
    #[serde(skip)]
    pub revision: u64,
}

impl Project {
    /// Whether `principal` works in this project: its owner or one of its members.
    pub fn admits(&self, principal: &Principal) -> bool {
        self.owner == *principal || self.members.contains(principal)
    }

    /// Whether the person `person` works in this project.
    pub fn admits_person(&self, person: PrincipalId) -> bool {
        self.admits(&Principal::person(person))
    }

    /// Refuses a name no one could read: empty, or longer than [`MAX_NAME`] characters.
    pub fn check_name(name: &str) -> Result<(), ProjectError> {
        if name.trim().is_empty() || name.chars().count() > MAX_NAME {
            return Err(ProjectError::Invalid(format!(
                "a space's name is 1 to {MAX_NAME} characters, not only spaces"
            )));
        }
        Ok(())
    }

    /// Whether two names are the same to a person: equal but for case and the spaces around.
    pub fn same_name(a: &str, b: &str) -> bool {
        a.trim().to_lowercase() == b.trim().to_lowercase()
    }

    /// `wanted`, or, when one of `taken` is the same name, the first of `wanted 2`, `wanted 3`
    /// and on that none is, cut to fit [`MAX_NAME`]: the name a new project of an owner whose
    /// projects are called `taken` gets.
    pub fn free_name(wanted: &str, taken: &[String]) -> String {
        let free = |name: &str| !taken.iter().any(|other| Self::same_name(name, other));
        let wanted = wanted.trim();
        if free(wanted) {
            return wanted.to_owned();
        }
        // Of `taken.len() + 1` names, one is free.
        (2..=taken.len().saturating_add(2))
            .map(|n| {
                let suffix = format!(" {n}");
                let room = MAX_NAME.saturating_sub(suffix.chars().count());
                let base: String = wanted.chars().take(room).collect();
                format!("{}{suffix}", base.trim_end())
            })
            .find(|name| free(name))
            .unwrap_or_else(|| wanted.to_owned())
    }

    /// Refuses `name` for the project `project` when another of its owner's projects, called
    /// `taken`, has it already.
    pub fn check_free(
        project: ProjectId,
        name: &str,
        taken: &[(ProjectId, String)],
    ) -> Result<(), ProjectError> {
        match taken
            .iter()
            .find(|(other, called)| *other != project && Self::same_name(name, called))
        {
            Some((_, called)) => Err(ProjectError::Conflict(format!(
                "there is a project called {called:?} already; choose another name"
            ))),
            None => Ok(()),
        }
    }
}

/// What leads from a folder to its project: its canonical path, or, for a git repository, its
/// `origin` remote, so another checkout or a clone that moved finds the same project.
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

/// The `origin` remote's url in a repository's `.git/config`, whose text is `config`: read
/// without running git, which a repository's own config can make run programs.
pub fn origin_remote(config: &str) -> Option<String> {
    let mut in_origin = false;
    for line in config.lines().map(str::trim) {
        if line.starts_with('[') {
            in_origin = line == "[remote \"origin\"]";
        } else if in_origin && let Some(url) = line.strip_prefix("url") {
            return url
                .trim_start()
                .strip_prefix('=')
                .map(|url| url.trim().to_owned());
        }
    }
    None
}

impl fmt::Display for FolderKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.stored())
    }
}

/// Why a projects call failed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProjectError {
    #[error("no project {0} here")]
    NotFound(ProjectId),
    /// The project exists already, or the person already has a personal project.
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Invalid(String),
    /// Another update came first since the record was read: read it again.
    #[error("project {0} was changed meanwhile")]
    Changed(ProjectId),
    #[error("project storage failed: {0}")]
    Storage(String),
}

/// Where projects are kept: the data store, which scopes every call by organization. Shaped as
/// if across a network: whole records in and out.
///
/// A project's name is unique among its owner's projects, ignoring case: a new one whose name
/// is taken gets the next free one ([`Project::free_name`]), and renaming one to a taken name is
/// refused. Different owners' projects may share a name.
pub trait Projects: Send + Sync {
    /// Keeps a new project, and answers it as kept, at revision 0: a project under the next free
    /// name when its owner has one called the same. Refused when its id is taken, when its name is not
    /// readable, or when it is a personal project and its owner has one.
    fn create<'a>(
        &'a self,
        org: OrgId,
        project: &'a Project,
    ) -> BoxFuture<'a, Result<Project, ProjectError>>;

    /// The project `project`, if the organization has it.
    fn get(
        &self,
        org: OrgId,
        project: ProjectId,
    ) -> BoxFuture<'_, Result<Option<Project>, ProjectError>>;

    /// The projects `member` owns or is a member of, in id order (oldest first, for UUID v7
    /// ids), at most `limit`.
    fn list<'a>(
        &'a self,
        org: OrgId,
        member: &'a Principal,
        limit: usize,
    ) -> BoxFuture<'a, Result<Vec<Project>, ProjectError>>;

    /// Replaces the project's name, members and `updated_at` with `project`'s, if its revision
    /// is still `project.revision`, and answers it as kept, its revision one more. Its kind,
    /// owner and creation never change. Refused with [`ProjectError::Changed`] when another
    /// update came first, and a name another of its owner's projects has with
    /// [`ProjectError::Conflict`].
    fn update<'a>(
        &'a self,
        org: OrgId,
        project: &'a Project,
    ) -> BoxFuture<'a, Result<Project, ProjectError>>;

    /// The personal project of `new.owner`: the one they have, or `new`, kept now. Two callers at
    /// once get the same one.
    fn personal<'a>(
        &'a self,
        org: OrgId,
        new: &'a Project,
    ) -> BoxFuture<'a, Result<Project, ProjectError>>;

    /// The project a folder leads to: the one the first of `keys` that is known leads to, or
    /// `new`, kept now as `create` keeps it. Every key in `keys` then leads to it, unless it
    /// already led elsewhere.
    fn folder<'a>(
        &'a self,
        org: OrgId,
        keys: &'a [FolderKey],
        new: &'a Project,
    ) -> BoxFuture<'a, Result<Project, ProjectError>>;

    /// The project a folder leads to, as [`Projects::folder`] finds it, making nothing: the
    /// one the first of `keys` that is known leads to, or none.
    fn find_folder<'a>(
        &'a self,
        org: OrgId,
        keys: &'a [FolderKey],
    ) -> BoxFuture<'a, Result<Option<Project>, ProjectError>>;
}
