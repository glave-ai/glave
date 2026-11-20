//! The project ledger's records (`specs/storage.md`, "Projects"). `project.folder.set`: one of a
//! project's folders, where its threads work; for each folder, the latest one holds. Where the
//! folder's files come from is kept apart from where they are checked out, so the project can
//! be checked out again, on another computer or after one is reset, from its source alone.
//! `project.removed`: the project was put away, its threads archived; a folder set after it
//! brings it back.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use uuid::Uuid;

use crate::codec::{Payload, parse};
use crate::payload::principal::Principal;

pub(crate) const FOLDER_SET: &str = "project.folder.set";
pub(crate) const REMOVED: &str = "project.removed";

/// A project's folder was set: added, or its source or checkout changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectFolderSet {
    /// Which of the project's folders this is: a UUID v7 chosen when it was first set, which
    /// every later record of the same folder names.
    pub folder: Uuid,
    /// Where its files come from.
    pub source: FolderSource,
    /// Where it is checked out.
    pub checkout: Checkout,
    /// Who set it. Absent: the project's person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Principal>,
}

/// Where a folder's files come from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FolderSource {
    /// A git repository, by the address it is cloned from, without a name, password, query or
    /// fragment it carries: where the folder comes back from.
    Repository { url: String },
    /// A folder on a computer, with no repository it comes from: only its checkout has it.
    Local,
}

/// A repository's address `url` naming the repository alone: for one with a scheme
/// (`https://`, `ssh://`), without the name and password before its host, nor a query or
/// fragment, which a clone made with a token in its address carries
/// (`https://<token>@host/owner/repo`); any other (`git@host:path`, a folder) as it is, since
/// nothing in it signs in. What glave keeps or shows of a repository's address, everywhere.
pub fn repository_address(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_owned();
    };
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    let (authority, path) = rest
        .split_once('/')
        .map_or((rest, None), |(a, p)| (a, Some(p)));
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    match path {
        Some(path) => format!("{scheme}://{host}/{path}"),
        None => format!("{scheme}://{host}"),
    }
}

/// Where a folder is checked out: a computer, and the path on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Checkout {
    /// The computer, by the id it keeps across runs.
    pub computer: Uuid,
    /// The folder's absolute path there, links resolved.
    pub path: String,
}

/// A project was removed: its threads archived, and it is listed no more until a folder is set
/// for it again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectRemoved {
    /// Who removed it. Absent: the project's person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Principal>,
}

impl Payload for ProjectRemoved {
    const TYPE: &'static str = REMOVED;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_removed(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<ProjectRemoved>> {
    (version == 1).then(|| parse(data))
}

impl Payload for ProjectFolderSet {
    const TYPE: &'static str = FOLDER_SET;
    const VERSION: u32 = 2;
}

impl ProjectFolderSet {
    /// Version 1, of the same shape, whose repository's address was kept as it was written,
    /// upgraded: the address named by its repository alone ([`repository_address`]).
    fn from_v1(mut v1: Self) -> Self {
        if let FolderSource::Repository { url } = &mut v1.source {
            *url = repository_address(url);
        }
        v1
    }
}

/// `project.folder.set` at `version`: version 2, whose repository's address names the
/// repository alone, or version 1, upgraded to it as it is read.
pub(crate) fn decode_folder_set(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<ProjectFolderSet>> {
    match version {
        1 => Some(parse(data).map(ProjectFolderSet::from_v1)),
        2 => Some(parse(data)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A repository's address kept by version 1 as it was written, a clone's token and all,
    /// reads as its repository alone (#1585).
    #[test]
    fn an_address_kept_with_a_token_reads_as_its_repository_alone() {
        let data = serde_json::value::to_raw_value(&serde_json::json!({
            "folder": "0192f5c4-0000-7000-8000-000000000001",
            "source": {"kind": "repository", "url": "https://a-token@github.com/o/app#a-token"},
            "checkout": {"computer": "0192f5c5-0000-7000-8000-000000000001", "path": "/code/app"},
        }))
        .unwrap();
        let read = decode_folder_set(1, &data).unwrap().unwrap();
        let url = "https://github.com/o/app".to_owned();
        assert_eq!(read.source, FolderSource::Repository { url });
    }
}
