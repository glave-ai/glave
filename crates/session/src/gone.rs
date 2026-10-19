//! A project whose folder isn't there anymore (`specs/session.md`, "Projects and who may do
//! what"). The server notices when the project is listed and before anything would run in it,
//! and says so as a fact with its sentence: `project/list`'s `gone`, and `-32029` for what would
//! run there, so nothing starts and nothing is made in the folder's place. The person on this
//! computer mends it: `project/folder/set` gives the project its folder's new place, where its
//! threads work from then on; `project/remove` archives its threads and lists it no more.

use glave_event::{Principal, ProjectRemoved, ThreadId};
use glave_harness::{Change, change_standing};
use glave_ports::{
    BlobStore, Caller, EventLog, FolderKey, ModelProvider, Project, ProjectError, ProjectId,
    ProjectKind, Scope, Standing, ThreadQuery,
};

use crate::folders::only_here;
use crate::held::Slot;
use crate::protocol::{ApiError, GoneFolder, NewFolder, Reply, code};
use crate::providers::local;
use crate::server::{MAX_PAGE, Server};

/// What the person is told about a project whose folder isn't there anymore.
const GONE: &str = "This project's folder isn't there anymore.";

/// Refused: the project's folder isn't there anymore, so nothing runs in it.
pub(crate) fn folder_gone() -> ApiError {
    ApiError::new(code::NO_FOLDER, GONE)
}

/// Whether `error` says the project's folder isn't there anymore.
pub(crate) fn is_gone(error: &ApiError) -> bool {
    error.code == code::NO_FOLDER
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Of `projects`, those `project/list` shows (a removed project is not one), and the projects
    /// among them whose folder isn't there anymore.
    pub(crate) async fn listed(
        &self,
        projects: Vec<Project>,
    ) -> Result<(Vec<Project>, Vec<GoneFolder>), ApiError> {
        let (mut shown, mut gone) = (Vec::with_capacity(projects.len()), Vec::new());
        for project in projects {
            if project.kind == ProjectKind::Project {
                if self.removed(project.id).await? {
                    continue;
                }
                if self
                    .folder_found(project.id)
                    .await
                    .is_err_and(|e| is_gone(&e))
                {
                    gone.push(GoneFolder {
                        project: project.id,
                        message: GONE.to_owned(),
                    });
                }
            }
            shown.push(project);
        }
        Ok((shown, gone))
    }

    /// Answers `project/folder/set`: the folder at `path` on this computer becomes the folder of
    /// the project `project`, whose own isn't there anymore, and leads to it. Threads held idle
    /// in it are let go of, so each works where the folder is now the next time it is opened.
    pub(crate) async fn move_folder(
        &self,
        caller: &Caller,
        project: ProjectId,
        path: String,
    ) -> Result<Reply, ApiError> {
        let found = self
            .owned_here(caller, project, "gives a project its folder")
            .await?;
        let was = match self.folder_found(project).await {
            Err(error) if is_gone(&error) => self.folder_of(project).await?,
            Err(error) => return Err(error),
            Ok(_) => {
                return Err(ApiError::new(
                    code::INVALID_PARAMS,
                    "this project's folder is still where it was",
                ));
            }
        };
        let (root, source) = self
            .make_folder(&found.name, NewFolder::Path { path })
            .await?;
        let mut keys = vec![FolderKey::Path(root.clone())];
        if let glave_event::FolderSource::Repository { url } = &source {
            keys.push(FolderKey::Remote(url.clone()));
        }
        // Its old place goes last: known, it leads to the project, so the new keys join it.
        if let Some(was) = was {
            keys.push(FolderKey::Path(was.checkout.path));
        }
        match self
            .inner
            .projects
            .folder(self.inner.home.org(), &keys, &found)
            .await
        {
            Ok(led) if led.id != project => {
                return Err(ApiError::new(
                    code::NOT_ALLOWED,
                    "that folder is already another project's",
                ));
            }
            // No key was known: the project keeps its folder in its ledger alone.
            Ok(_) | Err(ProjectError::Conflict(_)) => {}
            Err(error) => return Err(crate::projects::failed(&error)),
        }
        let person = Principal::person(self.person_of(caller)?);
        let folder = self.set_folder(project, source, root, Some(person)).await?;
        self.let_go_in(project);
        Ok(Reply::Project {
            project: found,
            folder: Some(folder),
        })
    }

    /// Answers `project/remove`: every thread of the project `project` goes to the archive, their
    /// history kept, then the project is recorded removed. Refused while a turn runs in it.
    pub(crate) async fn remove_project(
        &self,
        caller: &Caller,
        project: ProjectId,
    ) -> Result<Reply, ApiError> {
        self.owned_here(caller, project, "removes a project")
            .await?;
        let threads = self.active_threads(project).await?;
        if threads
            .iter()
            .any(|thread| matches!(self.lock().get(thread), Some(Slot::Busy { .. })))
        {
            return Err(ApiError::new(
                code::TURN_RUNNING,
                "something is running in this project: stop it first, then remove the project",
            ));
        }
        self.let_go_in(project);
        // Glave's own: archiving the threads of a project its owner removes.
        let scope = Scope::glaves_own(self.inner.home.org(), project);
        for thread in threads {
            let now = self.inner.harness.clock().now();
            let random = self.inner.harness.entropy().bytes()?;
            let log = self.inner.harness.log();
            change_standing(log, &scope, thread, now, random, Change::Archive)
                .await
                .map_err(|error| crate::server::api(&error))?;
        }
        let by = Some(Principal::person(self.person_of(caller)?));
        self.write_project_ledger(project, |current| {
            Ok((!current.removed).then(|| ProjectRemoved { by: by.clone() }))
        })
        .await?;
        Ok(Reply::Done {})
    }

    /// Archives, unarchives or deletes `thread`, whose project's folder isn't there anymore,
    /// straight in the log, as the person on this computer: it cannot be opened to change it
    /// as a held thread is. Anyone else is told the folder is gone.
    pub(crate) async fn change_gone(
        &self,
        caller: &Caller,
        thread: ThreadId,
        change: Change,
    ) -> Result<Reply, ApiError> {
        if !local(caller) {
            return Err(folder_gone());
        }
        let scope = self.scope_of(thread).await?;
        let now = self.inner.harness.clock().now();
        let random = self.inner.harness.entropy().bytes()?;
        change_standing(
            self.inner.harness.log(),
            &scope,
            thread,
            now,
            random,
            change,
        )
        .await
        .map_err(|error| crate::server::api(&error))?;
        Ok(Reply::Done {})
    }

    /// The project `project`, for `caller` on this computer, who owns it; refused otherwise, as
    /// only the person on this computer `does` what this is.
    async fn owned_here(
        &self,
        caller: &Caller,
        project: ProjectId,
        does: &str,
    ) -> Result<Project, ApiError> {
        if !local(caller) {
            return Err(only_here(&format!(
                "only the person on the computer glave runs on {does}"
            )));
        }
        let found = self.member_of(caller, project).await?;
        if found.kind != ProjectKind::Project {
            return Err(ApiError::new(
                code::INVALID_PARAMS,
                "only a project has a folder, and can be removed",
            ));
        }
        if found.owner != Principal::person(self.person_of(caller)?) {
            return Err(ApiError::new(
                code::NOT_ALLOWED,
                format!("only its owner {does}"),
            ));
        }
        Ok(found)
    }

    /// Every thread of `project` that is neither archived nor deleted.
    async fn active_threads(&self, project: ProjectId) -> Result<Vec<ThreadId>, ApiError> {
        // Glave's own: listing a project's threads, for the server to act on.
        let scope = Scope::glaves_own(self.inner.home.org(), project);
        let (mut threads, mut after) = (Vec::new(), None);
        loop {
            let query = ThreadQuery {
                standing: Standing::Active,
                limit: MAX_PAGE,
                after,
            };
            let views = self
                .inner
                .harness
                .log()
                .thread_views(&scope, &query)
                .await
                .map_err(|error| ApiError::new(code::INTERNAL, error.to_string()))?;
            let full = views.len() == MAX_PAGE;
            after = views.last().map(glave_ports::ThreadView::cursor);
            threads.extend(
                views
                    .into_iter()
                    .map(|view| view.thread)
                    .filter(|thread| !crate::ledger::is_ledger(*thread)),
            );
            if !full {
                return Ok(threads);
            }
        }
    }

    /// Lets go of every thread of `project` held idle, so each is read again from the log, and
    /// opened where its project's folder is now, the next time it is used.
    fn let_go_in(&self, project: ProjectId) {
        self.lock().retain(|_, slot| match slot {
            Slot::Idle { thread: held, .. } => held.scope.project() != project,
            Slot::Busy { .. } => true,
        });
    }
}
