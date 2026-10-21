//! What `project/*` take and answer with besides the project itself: where a project's work
//! lives.

use glave_event::{Checkout, FolderSource};
use glave_ports::ProjectId;
use serde::{Deserialize, Serialize};

use super::Told;

/// A new project's folder, on the computer glave runs on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NewFolder {
    /// A folder already on this computer, by its full path: worked in where it is.
    Path { path: String },
    /// A repository cloned onto this computer, from its `https://` address, signing in with
    /// the git host connected for it.
    Repository { url: String },
    /// A new, empty folder, made a git repository.
    Empty,
}

/// A project whose folder isn't there anymore, on the computer it is checked out on, with the
/// sentence that says so.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GoneFolder {
    pub project: ProjectId,
    pub message: Told,
}

/// A project's folder: where its files come from, and where they are checked out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProjectFolder {
    pub source: FolderSource,
    pub checkout: Checkout,
}
