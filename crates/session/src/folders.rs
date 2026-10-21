//! Projects' folders (`specs/session.md`, "Where a thread works"; `specs/storage.md`,
//! "Projects"): where a project's files come from (a repository, or a folder of the person's)
//! and where they are checked out (a computer, and the path there), kept in the project ledger
//! as `project.folder.set`, the latest one counting. A project gets its folder from
//! `project/create` (a folder already here, a repository cloned here, or a new one) or from
//! glave run in a folder, and its threads work there. `folder/list` shows the person on this
//! computer the folders to pick one from.

use std::path::Path;

use glave_event::{
    At, Body, Checkout, EventId, FolderSource, Header, Payload, Principal, ProjectFolderSet, Seq,
    ThreadId, decode, encode,
};
use glave_harness::Looked;
use glave_ports::computer::{ComputerError, CredentialHost, Forge};
use glave_ports::stream::{Reserved, append_retrying, catch_up};
use glave_ports::{
    BlobStore, Caller, EventLog, FolderKey, ModelProvider, Project, ProjectId, ProjectKind, Scope,
};

use crate::protocol::{ApiError, NewFolder, ProjectFolder, Reply, code};
use crate::providers::local;
use crate::server::Server;
use crate::words::Sentence;

/// The longest name glave gives a new project's folder.
const FOLDER_NAME: usize = 64;

/// What a project's ledger says, as far as it was read.
#[derive(Debug, Default)]
pub(crate) struct ProjectLedger {
    /// The latest record of the project's first folder.
    folder: Option<ProjectFolderSet>,
    /// Whether the project was removed, and given no folder since.
    pub(crate) removed: bool,
    /// The `seq` of the ledger's last event.
    last: Option<Seq>,
}

/// Refused: folders are on the computer glave runs on, so only the person there reaches them.
pub(crate) fn only_here(sentence: Sentence) -> ApiError {
    ApiError::new(code::NOT_ALLOWED, sentence)
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// The folder of the project `project`, if it has one: its first, while a project has one.
    pub(crate) async fn folder_of(
        &self,
        project: ProjectId,
    ) -> Result<Option<ProjectFolder>, ApiError> {
        Ok(self
            .read_project_ledger(project)
            .await?
            .folder
            .map(|set| ProjectFolder {
                source: set.source,
                checkout: set.checkout,
            }))
    }

    /// Whether the project `project` was removed, and not given a folder since.
    pub(crate) async fn removed(&self, project: ProjectId) -> Result<bool, ApiError> {
        Ok(self.read_project_ledger(project).await?.removed)
    }

    /// The path of the project's folder on the computer this server's threads work on, if it
    /// has a folder; refused when the folder is checked out on another computer.
    pub(crate) async fn folder_here(&self, project: ProjectId) -> Result<Option<String>, ApiError> {
        let Some(folder) = self.folder_of(project).await? else {
            return Ok(None);
        };
        if folder.checkout.computer != self.computer() {
            return Err(ApiError::new(code::WORKSPACE, Sentence::FolderElsewhere));
        }
        Ok(Some(folder.checkout.path))
    }

    /// The path of the project's folder here, as [`Server::folder_here`] finds it, once glave
    /// has looked that it is there: refused with `-32029` when it isn't anymore.
    pub(crate) async fn folder_found(
        &self,
        project: ProjectId,
    ) -> Result<Option<String>, ApiError> {
        let Some(path) = self.folder_here(project).await? else {
            return Ok(None);
        };
        match self.inner.harness.workspace().look(&path).await {
            Err(ComputerError::NotFound(_)) => Err(crate::gone::folder_gone()),
            _ => Ok(Some(path)),
        }
    }

    /// Makes the folder at `path` on this server's computer the folder of the project `project`,
    /// unless it already is: what glave run in a folder does for that folder's project, so the
    /// folder a person last opened a project from is where its threads work.
    pub async fn keep_folder(&self, project: ProjectId, path: &str) -> Result<(), ApiError> {
        let looked = self
            .inner
            .harness
            .workspace()
            .look(path)
            .await
            .map_err(|error| no_folder(path, &error))?;
        let source = source_of(&looked);
        self.set_folder(project, source, looked.root, None)
            .await
            .map(|_| ())
    }

    /// The id of the computer this server's threads work on, as checkouts name it.
    fn computer(&self) -> uuid::Uuid {
        self.inner.harness.workspace().info().computer.as_uuid()
    }

    /// What the project's ledger says: its first folder's latest record, whether the project was
    /// removed since, and the `seq` of the ledger's last event.
    async fn read_project_ledger(&self, project: ProjectId) -> Result<ProjectLedger, ApiError> {
        // Glave's own: the project's ledger of folders.
        let scope = Scope::glaves_own(self.inner.org, project);
        let mut read = ProjectLedger::default();
        let log = self.inner.harness.log();
        catch_up(log, &scope, Reserved::Project.id(), None, |page| {
            for raw in page {
                read.last = Some(raw.seq);
                match decode(raw).map(|event| event.body) {
                    Ok(Body::ProjectFolderSet(set))
                        if read
                            .folder
                            .as_ref()
                            .is_none_or(|first| first.folder == set.folder) =>
                    {
                        read.folder = Some(set);
                        read.removed = false;
                    }
                    Ok(Body::ProjectRemoved(_)) => read.removed = true,
                    _ => {}
                }
            }
        })
        .await?;
        Ok(read)
    }

    /// Appends to the project's ledger what `next` makes of it as it stands, unless it makes
    /// nothing, after anything others wrote to it meanwhile.
    pub(crate) async fn write_project_ledger<T: Payload>(
        &self,
        project: ProjectId,
        next: impl Fn(&ProjectLedger) -> Result<Option<T>, ApiError>,
    ) -> Result<(), ApiError> {
        let ledger = Reserved::Project.id();
        // Glave's own: the project's ledger of folders.
        let scope = Scope::glaves_own(self.inner.org, project);
        let (this, next) = (self, &next);
        let log = self.inner.harness.log();
        append_retrying(log, &scope, ledger, move || async move {
            let current = this.read_project_ledger(project).await?;
            let Some(payload) = next(&current)? else {
                return Ok(None);
            };
            let now = this.inner.harness.clock().now();
            let header = Header {
                id: EventId::new(now, this.inner.harness.entropy().bytes()?)?,
                thread: ledger,
                seq: current
                    .last
                    .map_or(Some(Seq::FIRST), Seq::next)
                    .ok_or_else(|| {
                        ApiError::internal(&format!("the ledger of project {project} is full"))
                    })?,
                at: At::from(now),
            };
            let raw = encode(header, &payload)?;
            Ok::<_, ApiError>(Some((vec![raw], ())))
        })
        .await?;
        Ok(())
    }

    /// Records the project's folder as coming from `source` and checked out at `path` on this
    /// server's computer, set by `by` (absent: the project's person), unless it already is and
    /// the project was not removed since.
    pub(crate) async fn set_folder(
        &self,
        project: ProjectId,
        source: FolderSource,
        path: String,
        by: Option<Principal>,
    ) -> Result<ProjectFolder, ApiError> {
        let kept = ProjectFolder {
            source,
            checkout: Checkout {
                computer: self.computer(),
                path,
            },
        };
        self.write_project_ledger(project, |current| {
            if !current.removed
                && current
                    .folder
                    .as_ref()
                    .is_some_and(|set| set.source == kept.source && set.checkout == kept.checkout)
            {
                return Ok(None);
            }
            let folder = match &current.folder {
                Some(set) => set.folder,
                None => ThreadId::new(
                    self.inner.harness.clock().now(),
                    self.inner.harness.entropy().bytes()?,
                )?
                .as_uuid(),
            };
            Ok(Some(ProjectFolderSet {
                folder,
                source: kept.source.clone(),
                checkout: kept.checkout.clone(),
                by: by.clone(),
            }))
        })
        .await?;
        Ok(kept)
    }

    /// Answers `project/create` with a folder: the project its folder leads to, made now unless
    /// the folder already led to one the caller works in, with the folder as its own.
    pub(crate) async fn create_with_folder(
        &self,
        caller: &Caller,
        new: Project,
        folder: NewFolder,
    ) -> Result<Reply, ApiError> {
        if !local(caller) {
            return Err(only_here(Sentence::NewFolderOnlyHere));
        }
        let person = Principal::person(self.person_of(caller)?);
        let (root, source) = self.make_folder(&new.name, folder).await?;
        let mut keys = vec![FolderKey::Path(root.clone())];
        if let FolderSource::Repository { url } = &source {
            keys.push(FolderKey::Remote(url.clone()));
        }
        let project = self
            .inner
            .projects
            .folder(self.inner.org, &keys, &new)
            .await?;
        if project.kind != ProjectKind::Project || !project.admits(&person) {
            return Err(ApiError::new(
                code::NOT_ALLOWED,
                Sentence::FolderNotYours { folder: root },
            ));
        }
        let folder = self
            .set_folder(project.id, source, root, Some(person))
            .await?;
        Ok(Reply::Project {
            project,
            folder: Some(folder),
        })
    }

    /// Answers `project/find`: the project the folder at `path` leads to, when the caller works
    /// in it, making nothing.
    pub(crate) async fn find_project(
        &self,
        caller: &Caller,
        path: &str,
    ) -> Result<Reply, ApiError> {
        if !local(caller) {
            return Err(only_here(Sentence::LookOnlyHere));
        }
        let person = Principal::person(self.person_of(caller)?);
        let looked = self
            .inner
            .harness
            .workspace()
            .look(path)
            .await
            .map_err(|error| no_folder(path, &error))?;
        let mut keys = vec![FolderKey::Path(looked.root)];
        if let Some(url) = looked.origin {
            keys.push(FolderKey::Remote(url));
        }
        let found = self
            .inner
            .projects
            .find_folder(self.inner.org, &keys)
            .await?
            .filter(|project| project.kind == ProjectKind::Project && project.admits(&person));
        let (projects, gone) = self.listed(found.into_iter().collect()).await?;
        Ok(Reply::Projects { projects, gone })
    }

    /// The folder `folder` asks for on this server's computer, for the project `name`: where
    /// it is, and where its files come from.
    pub(crate) async fn make_folder(
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
                        Sentence::TheFullPath { path },
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
                    .map_err(|error| {
                        ApiError::new(
                            code::INTERNAL,
                            Sentence::FolderNotMade {
                                reason: error.to_string(),
                            },
                        )
                    })?;
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
            return Err(only_here(Sentence::FoldersOnlyHere));
        }
        let workspace = self.inner.harness.workspace();
        let path = match path.or_else(|| workspace.info().home.clone()) {
            Some(path) if Path::new(&path).is_absolute() => path,
            Some(path) => {
                return Err(ApiError::new(
                    code::INVALID_PARAMS,
                    Sentence::AFullPath { path },
                ));
            }
            None => {
                return Err(ApiError::new(code::INVALID_PARAMS, Sentence::NoHome));
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
            Sentence::NoFolderAt {
                path: path.to_owned(),
            },
        ),
        ComputerError::Refused(message) => ApiError::new(code::INVALID_PARAMS, message.clone()),
        _ => ApiError::new(
            code::INTERNAL,
            Sentence::FolderNotOpened {
                path: path.to_owned(),
                reason: error.message().to_owned(),
            },
        ),
    }
}

/// A clone that failed, with what git said.
fn not_cloned(url: &str, error: &ComputerError) -> ApiError {
    ApiError::new(
        code::HOST,
        Sentence::CloneFailed {
            url: url.to_owned(),
            reason: error.message().to_owned(),
        },
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
            Sentence::NotARepository {
                url: url.to_owned(),
            },
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

/// A folder's name for the project `name`: its letters and digits, with `-` for projects and
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
    use glave_ports::testing::FAKE_SECRET;

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
        let signed_in = format!("https://ada:{FAKE_SECRET}@github.com/acme/app");
        for bad in [
            "git@github.com:acme/app.git",
            "http://github.com/acme/app",
            &signed_in,
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
