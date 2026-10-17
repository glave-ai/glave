//! Projects' folders (`specs/session.md`, "Where a thread works"; `specs/storage.md`,
//! "Spaces"): where a project's files come from (a repository, or a folder of the person's)
//! and where they are checked out (a computer, and the path there), kept in the space ledger
//! as `space.folder.set`, the latest one counting. A project gets its folder from
//! `space/create` (a folder already here, a repository cloned here, or a new one) or from
//! glave run in a folder, and its threads work there. `folder/list` shows the person on this
//! computer the folders to pick one from.

use std::path::Path;

use glave_event::{
    At, Body, Checkout, EventId, FolderSource, Header, Principal, Seq, SpaceFolderSet, ThreadId,
    decode, encode,
};
use glave_harness::{Caller, Looked};
use glave_ports::{
    BlobStore, ComputerError, CredentialHost, EventLog, FolderKey, Forge, LogError, ModelProvider,
    Scope, Space, SpaceId, SpaceKind,
};

use crate::ledger::space_ledger;
use crate::protocol::{ApiError, NewFolder, ProjectFolder, Reply, code};
use crate::providers::local;
use crate::server::Server;

/// Ledger events read per page.
const PAGE: usize = 500;
/// Appends tried before giving up on a ledger other writers keep moving.
const ATTEMPTS: usize = 8;
/// The longest name glave gives a new project's folder.
const FOLDER_NAME: usize = 64;

fn internal(error: &impl std::fmt::Display) -> ApiError {
    ApiError::new(code::INTERNAL, error.to_string())
}

/// Refused: folders are on the computer glave runs on, so only the person there reaches them.
fn only_here(message: &str) -> ApiError {
    ApiError::new(code::NOT_ALLOWED, message)
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// The folder of the space `space`, if it has one: its first, while a project has one.
    pub(crate) async fn folder_of(
        &self,
        space: SpaceId,
    ) -> Result<Option<ProjectFolder>, ApiError> {
        Ok(self
            .read_space_ledger(space)
            .await?
            .0
            .map(|set| ProjectFolder {
                source: set.source,
                checkout: set.checkout,
            }))
    }

    /// The path of the space's folder on the computer this server's threads work on, if it
    /// has a folder; refused when the folder is checked out on another computer.
    pub(crate) async fn folder_here(&self, space: SpaceId) -> Result<Option<String>, ApiError> {
        let Some(folder) = self.folder_of(space).await? else {
            return Ok(None);
        };
        if folder.checkout.computer != self.computer() {
            return Err(ApiError::new(
                code::WORKSPACE,
                "this project's folder is on another computer, which glave cannot reach from \
                 here yet",
            ));
        }
        Ok(Some(folder.checkout.path))
    }

    /// Makes the folder at `path` on this server's computer the folder of the project `space`,
    /// unless it already is: what glave run in a folder does for that folder's project, so the
    /// folder a person last opened a project from is where its threads work.
    pub async fn keep_folder(&self, space: SpaceId, path: &str) -> Result<(), ApiError> {
        let looked = self
            .inner
            .harness
            .workspace()
            .look(path)
            .await
            .map_err(|error| no_folder(path, &error))?;
        let source = source_of(&looked);
        self.set_folder(space, source, looked.root, None)
            .await
            .map(|_| ())
    }

    /// The id of the computer this server's threads work on, as checkouts name it.
    fn computer(&self) -> uuid::Uuid {
        self.inner.harness.workspace().info().computer.as_uuid()
    }

    /// The latest record of the space's first folder, and the `seq` of its ledger's last event.
    async fn read_space_ledger(
        &self,
        space: SpaceId,
    ) -> Result<(Option<SpaceFolderSet>, Option<Seq>), ApiError> {
        let ledger = space_ledger().ok_or_else(|| internal(&"the space ledger has no id"))?;
        let scope = self.scope_in(space);
        let (mut folder, mut last) = (None::<SpaceFolderSet>, None);
        loop {
            let page = self
                .inner
                .harness
                .log()
                .read(&scope, ledger, last, PAGE)
                .await
                .map_err(|error| internal(&error))?;
            let full = page.len() == PAGE;
            for raw in page {
                last = Some(raw.seq);
                if let Ok(event) = decode(raw)
                    && let Body::SpaceFolderSet(set) = event.body
                    && folder
                        .as_ref()
                        .is_none_or(|first| first.folder == set.folder)
                {
                    folder = Some(set);
                }
            }
            if !full {
                return Ok((folder, last));
            }
        }
    }

    /// Records the space's folder as coming from `source` and checked out at `path` on this
    /// server's computer, set by `by` (absent: the space's person), unless it already is,
    /// after anything others wrote to the ledger meanwhile.
    async fn set_folder(
        &self,
        space: SpaceId,
        source: FolderSource,
        path: String,
        by: Option<Principal>,
    ) -> Result<ProjectFolder, ApiError> {
        let ledger = space_ledger().ok_or_else(|| internal(&"the space ledger has no id"))?;
        let scope = self.scope_in(space);
        let kept = ProjectFolder {
            source,
            checkout: Checkout {
                computer: self.computer(),
                path,
            },
        };
        for _ in 0..ATTEMPTS {
            let (current, last) = self.read_space_ledger(space).await?;
            if current
                .as_ref()
                .is_some_and(|set| set.source == kept.source && set.checkout == kept.checkout)
            {
                return Ok(kept);
            }
            let now = self.inner.harness.clock().now();
            let entropy = self.inner.harness.entropy();
            let folder = match &current {
                Some(set) => set.folder,
                None => ThreadId::new(now, entropy.bytes())
                    .map_err(|error| internal(&error))?
                    .as_uuid(),
            };
            let header = Header {
                id: EventId::new(now, entropy.bytes()).map_err(|error| internal(&error))?,
                thread: ledger,
                seq: last
                    .map_or(Some(Seq::FIRST), Seq::next)
                    .ok_or_else(|| internal(&format!("the ledger of space {space} is full")))?,
                at: At::from(now),
            };
            let set = SpaceFolderSet {
                folder,
                source: kept.source.clone(),
                checkout: kept.checkout.clone(),
                by: by.clone(),
            };
            let raw = encode(header, &set).map_err(|error| internal(&error))?;
            match self
                .inner
                .harness
                .log()
                .append(&scope, std::slice::from_ref(&raw))
                .await
            {
                Ok(()) => return Ok(kept),
                Err(LogError::Conflict { .. } | LogError::Gap { .. }) => {}
                Err(error) => return Err(internal(&error)),
            }
        }
        Err(internal(&"the project's settings kept changing; try again"))
    }

    /// Answers `space/create` with a folder: the project its folder leads to, made now unless
    /// the folder already led to one the caller works in, with the folder as its own.
    pub(crate) async fn create_project(
        &self,
        caller: &Caller,
        new: Space,
        folder: NewFolder,
    ) -> Result<Reply, ApiError> {
        if !local(caller) {
            return Err(only_here(
                "a project's folder is on the computer glave runs on, so only the person there \
                 can give one",
            ));
        }
        let person = Principal::person(self.person_of(caller)?);
        let (root, source) = self.make_folder(&new.name, folder).await?;
        let mut keys = vec![FolderKey::Path(root.clone())];
        if let FolderSource::Repository { url } = &source {
            keys.push(FolderKey::Remote(url.clone()));
        }
        let space = self
            .inner
            .spaces
            .folder(self.inner.home.org, &keys, &new)
            .await
            .map_err(|error| crate::spaces::failed(&error))?;
        if space.kind != SpaceKind::Project || !space.admits(&person) {
            return Err(ApiError::new(
                code::NOT_ALLOWED,
                format!("{root} belongs to a project you do not work in"),
            ));
        }
        let folder = self
            .set_folder(space.id, source, root, Some(person))
            .await?;
        Ok(Reply::Space {
            space,
            folder: Some(folder),
        })
    }

    /// The folder `folder` asks for on this server's computer, for the project `name`: where
    /// it is, and where its files come from.
    async fn make_folder(
        &self,
        name: &str,
        folder: NewFolder,
    ) -> Result<(String, FolderSource), ApiError> {
        let workspace = self.inner.harness.workspace();
        match folder {
            NewFolder::Path { path } => {
                if !Path::new(&path).is_absolute() {
                    return Err(ApiError::new(
                        code::INVALID_PARAMS,
                        format!("give the folder's full path, not {path:?}"),
                    ));
                }
                let looked = workspace
                    .look(&path)
                    .await
                    .map_err(|error| no_folder(&path, &error))?;
                let source = source_of(&looked);
                Ok((looked.root, source))
            }
            NewFolder::Repository { url } => {
                let url = repository(&url)?;
                let root = workspace
                    .new_project(&folder_name(name), Some(&url))
                    .await
                    .map_err(|error| not_cloned(&url, &error))?;
                Ok((root, FolderSource::Repository { url }))
            }
            NewFolder::Empty => {
                let root = workspace
                    .new_project(&folder_name(name), None)
                    .await
                    .map_err(|error| internal(&format!("could not make the folder: {error}")))?;
                Ok((root, FolderSource::Local))
            }
        }
    }

    /// Answers `folder/list`: the folders inside `path`, or the home folder, by name.
    pub(crate) async fn list_folders(
        &self,
        caller: &Caller,
        path: Option<String>,
    ) -> Result<Reply, ApiError> {
        if !local(caller) {
            return Err(only_here(
                "folders are shown only to the person on the computer glave runs on",
            ));
        }
        let workspace = self.inner.harness.workspace();
        let path = match path.or_else(|| workspace.info().home.clone()) {
            Some(path) if Path::new(&path).is_absolute() => path,
            Some(path) => {
                return Err(ApiError::new(
                    code::INVALID_PARAMS,
                    format!("give a folder's full path, not {path:?}"),
                ));
            }
            None => {
                return Err(ApiError::new(
                    code::INVALID_PARAMS,
                    "this computer has no home folder to start from; give a folder's full path",
                ));
            }
        };
        let inside = workspace
            .folders_in(&path)
            .await
            .map_err(|error| no_folder(&path, &error))?;
        let parent = Path::new(&inside.root)
            .parent()
            .map(|parent| parent.to_string_lossy().into_owned());
        Ok(Reply::Folders {
            path: inside.root,
            parent,
            folders: inside.folders,
        })
    }

    /// The scope of `space` in this server's organization.
    pub(crate) fn scope_in(&self, space: SpaceId) -> Scope {
        Scope {
            org: self.inner.home.org,
            space,
        }
    }
}

/// Where the files of a folder glave looked at come from: the repository it is the root of,
/// when that has an `origin`, else the folder itself.
fn source_of(looked: &Looked) -> FolderSource {
    looked
        .origin
        .clone()
        .map_or(FolderSource::Local, |url| FolderSource::Repository { url })
}

/// A folder the computer could not open, as the person is told it.
fn no_folder(path: &str, error: &ComputerError) -> ApiError {
    match error {
        ComputerError::NotFound(_) => ApiError::new(
            code::INVALID_PARAMS,
            format!("there is no folder at {path}"),
        ),
        ComputerError::Refused(message) => ApiError::new(code::INVALID_PARAMS, message.clone()),
        _ => internal(error),
    }
}

/// A clone that failed, with what git said.
fn not_cloned(url: &str, error: &ComputerError) -> ApiError {
    ApiError::new(
        code::HOST,
        format!(
            "could not clone {url}: {}. If the repository is private, connect its git host \
             first",
            error.message()
        ),
    )
}

/// `url`, trimmed, when it is a repository's address glave clones from: `https://` and a host
/// (`http://` only to this computer), with no user name or token in it, then a path; else why
/// not.
fn repository(url: &str) -> Result<String, ApiError> {
    let url = url.trim();
    let refused = || {
        ApiError::new(
            code::INVALID_PARAMS,
            format!(
                "{url:?} is not a repository's address: give its https:// address, such as \
                 https://github.com/acme/app, with no user name or token in it"
            ),
        )
    };
    let (scheme, rest) = url.split_once("://").ok_or_else(refused)?;
    let (host, path) = rest.split_once('/').ok_or_else(refused)?;
    let origin = format!("{scheme}://{host}");
    let plain = |c: char| !c.is_control() && !c.is_whitespace() && c != '\'';
    if CredentialHost::named(&origin, Forge::Other).is_none()
        || path.trim_matches('/').is_empty()
        || !path.chars().all(plain)
    {
        return Err(refused());
    }
    Ok(url.to_owned())
}

/// A folder's name for the project `name`: its letters and digits, with `-` for spaces and
/// other marks, at most [`FOLDER_NAME`] characters; `project` when nothing is left.
fn folder_name(name: &str) -> String {
    let mut folder = String::new();
    for c in name.chars() {
        let kept = if c.is_alphanumeric() || c == '_' || c == '.' {
            c
        } else {
            '-'
        };
        if kept == '-' && (folder.is_empty() || folder.ends_with('-')) {
            continue;
        }
        if folder.chars().count() >= FOLDER_NAME {
            break;
        }
        folder.push(kept);
    }
    let folder = folder.trim_start_matches('.').trim_end_matches(['-', '.']);
    if folder.is_empty() {
        "project".to_owned()
    } else {
        folder.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{folder_name, repository};

    #[test]
    fn a_project_s_folder_is_named_after_it() {
        assert_eq!(folder_name("My App"), "My-App");
        assert_eq!(folder_name("  café / notes!! "), "café-notes");
        assert_eq!(folder_name("..secret"), "secret");
        assert_eq!(folder_name("v1.2"), "v1.2");
        assert_eq!(folder_name("???"), "project");
        assert_eq!(folder_name(&"x".repeat(100)).chars().count(), 64);
    }

    #[test]
    fn only_an_https_address_without_a_sign_in_is_cloned() {
        for good in [
            "https://github.com/acme/app",
            " https://git.example.com:8443/team/app.git ",
            "http://127.0.0.1:3000/repo.git",
        ] {
            assert!(repository(good).is_ok(), "{good}");
        }
        for bad in [
            "git@github.com:acme/app.git",
            "http://github.com/acme/app",
            "https://ada:ghp_token@github.com/acme/app",
            "https://github.com",
            "https://github.com/",
            "https://github.com/it's",
            "https://github.com/a b",
            "file:///etc",
            "-uhttps://x/y",
        ] {
            assert!(repository(bad).is_err(), "{bad}");
        }
    }
}
