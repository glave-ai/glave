//! Projects over the session API, and who may do what in them (`specs/session.md`, "Projects and
//! who may do what"). A server serves many projects of one organization: its own, any other its
//! callers work in, and each caller's Chats, where requests that name none act. Every request
//! that acts on a thread, or
//! on the server's own project as a whole, is checked against the caller: they must own the
//! thread's project or be one of its members, or it is refused with `-32012` before anything is
//! read or recorded.

use std::collections::HashMap;

use glave_event::{At, Principal, PrincipalId, ThreadId};
use glave_ports::{
    BlobStore, Caller, EventLog, ModelProvider, Project, ProjectError, ProjectId, ProjectKind,
    Scope,
};

use crate::held::Slot;
use crate::protocol::{ApiError, NewFolder, Reply, Request, code};
use crate::server::{Server, no_thread};

/// Threads whose project a server remembers; past this, it forgets them all and asks again.
const PLACES: usize = 4_096;
/// The most projects whose tasks and schedules one person sees.
const MOST_PROJECTS: usize = 1_000;

/// What a personal project is called until its person renames it.
pub const PERSONAL_NAME: &str = "Chats";

/// The scopes of threads a server has looked up. A thread's project never changes.
#[derive(Debug, Default)]
pub(crate) struct Places(HashMap<ThreadId, Scope>);

impl Places {
    fn get(&self, thread: ThreadId) -> Option<Scope> {
        self.0.get(&thread).copied()
    }

    pub(crate) fn insert(&mut self, thread: ThreadId, scope: Scope) {
        if self.0.len() >= PLACES {
            self.0.clear();
        }
        self.0.insert(thread, scope);
    }
}

pub(crate) fn failed(error: &ProjectError) -> ApiError {
    match error {
        ProjectError::NotFound(project) => no_project(*project),
        ProjectError::Conflict(_) | ProjectError::Invalid(_) => {
            ApiError::new(code::INVALID_PARAMS, error.to_string())
        }
        ProjectError::Storage(_) => ApiError::new(code::INTERNAL, error.to_string()),
    }
}

fn no_project(project: ProjectId) -> ApiError {
    ApiError::new(code::NO_PROJECT, format!("no project {project} here"))
}

fn not_allowed(what: &str) -> ApiError {
    ApiError::new(code::NOT_ALLOWED, format!("you do not work in {what}"))
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// The person `caller` acts as: the one the transport verified, or the local person.
    pub(crate) fn person_of(&self, caller: &Caller) -> Result<PrincipalId, ApiError> {
        caller
            .person
            .or_else(|| self.inner.harness.person())
            .ok_or_else(|| ApiError::new(code::INTERNAL, "this server has no person"))
    }

    /// The server's own project: the folder it serves. Kept as the local person's project when
    /// the store has no record of it yet.
    pub async fn home_project(&self) -> Result<Project, ApiError> {
        let home = self.inner.home;
        if let Some(project) = self
            .inner
            .projects
            .get(home.org, home.project)
            .await
            .map_err(|e| failed(&e))?
        {
            return Ok(project);
        }
        let now = At::from(self.inner.harness.clock().now());
        let project = Project {
            id: home.project,
            kind: ProjectKind::Project,
            name: "this folder".to_owned(),
            owner: Principal::person(self.person_of(&Caller::default())?),
            members: Vec::new(),
            created_at: now,
            updated_at: now,
        };
        match self.inner.projects.create(home.org, &project).await {
            Ok(kept) => Ok(kept),
            // Another process kept it first.
            Err(ProjectError::Conflict(_)) => self
                .inner
                .projects
                .get(home.org, home.project)
                .await
                .map_err(|e| failed(&e))?
                .ok_or_else(|| no_project(home.project)),
            Err(error) => Err(failed(&error)),
        }
    }

    /// The project `project` of this server's organization.
    async fn project(&self, project: ProjectId) -> Result<Project, ApiError> {
        if project == self.inner.home.project {
            return self.home_project().await;
        }
        self.inner
            .projects
            .get(self.inner.home.org, project)
            .await
            .map_err(|e| failed(&e))?
            .ok_or_else(|| no_project(project))
    }

    /// The project `project`, when `caller` works in it; refused with `-32012` otherwise.
    pub(crate) async fn member_of(
        &self,
        caller: &Caller,
        project: ProjectId,
    ) -> Result<Project, ApiError> {
        let found = self.project(project).await?;
        if found.admits_person(self.person_of(caller)?) {
            Ok(found)
        } else {
            Err(not_allowed(&format!("project {project}")))
        }
    }

    /// The caller's personal project, made the first time.
    pub(crate) async fn personal_project(&self, caller: &Caller) -> Result<Project, ApiError> {
        let now = self.inner.harness.clock().now();
        let id = ThreadId::new(now, self.inner.harness.entropy().bytes()?)
            .map_err(|e| ApiError::new(code::INTERNAL, e.to_string()))?;
        let new = Project {
            id: ProjectId::from_uuid(id.as_uuid()),
            kind: ProjectKind::Personal,
            name: PERSONAL_NAME.to_owned(),
            owner: Principal::person(self.person_of(caller)?),
            members: Vec::new(),
            created_at: At::from(now),
            updated_at: At::from(now),
        };
        self.inner
            .projects
            .personal(self.inner.home.org, &new)
            .await
            .map_err(|e| failed(&e))
    }

    /// The project a request acts in: `asked`, when the caller works in it; with none, their
    /// personal project, their Chats, whatever folder the server started in.
    pub(crate) async fn project_for(
        &self,
        caller: &Caller,
        asked: Option<ProjectId>,
    ) -> Result<Project, ApiError> {
        match asked {
            Some(project) => self.member_of(caller, project).await,
            None => self.personal_project(caller).await,
        }
    }

    /// The projects `caller` works in, the server's own among them when they do: those whose
    /// tasks and schedules they see.
    pub(crate) async fn projects_of(&self, caller: &Caller) -> Result<Vec<ProjectId>, ApiError> {
        // The server's own project is kept first, so it is listed for those who work in it.
        self.home_project().await?;
        let person = Principal::person(self.person_of(caller)?);
        let projects = self
            .inner
            .projects
            .list(self.inner.home.org, &person, MOST_PROJECTS)
            .await
            .map_err(|e| failed(&e))?;
        Ok(projects.into_iter().map(|project| project.id).collect())
    }

    /// The scope `thread` is stored in: remembered, or found in the log.
    pub(crate) async fn scope_of(&self, thread: ThreadId) -> Result<Scope, ApiError> {
        if crate::ledger::is_ledger(thread) {
            return Err(no_thread(thread));
        }
        if let Some(scope) = self.places().get(thread) {
            return Ok(scope);
        }
        if let Some(Slot::Idle { thread: held, .. }) = self.lock().get(&thread) {
            return Ok(held.scope);
        }
        let scope = self
            .inner
            .harness
            .log()
            .locate(thread)
            .await
            .map_err(|e| ApiError::new(code::INTERNAL, e.to_string()))?
            .filter(|scope| scope.org == self.inner.home.org)
            .ok_or_else(|| no_thread(thread))?;
        self.places().insert(thread, scope);
        Ok(scope)
    }

    /// The scope of `thread`, when `caller` works in its project; refused with `-32012`
    /// otherwise, and with "no such thread" when there is none.
    pub(crate) async fn authorize(
        &self,
        caller: &Caller,
        thread: ThreadId,
    ) -> Result<Scope, ApiError> {
        let scope = self.scope_of(thread).await?;
        self.member_of(caller, scope.project)
            .await
            .map_err(|refused| match refused.code {
                code::NOT_ALLOWED => not_allowed(&format!("the project of thread {thread}")),
                _ => refused,
            })?;
        Ok(scope)
    }

    /// Refuses `request` unless `caller` may make it: for a thread, they work in its project;
    /// for a project's memory, in that project, the server's own when it names none.
    pub(crate) async fn allow(&self, caller: &Caller, request: &Request) -> Result<(), ApiError> {
        if let Some(thread) = request.thread() {
            self.authorize(caller, thread).await?;
        } else if let Some(project) = request.memory_project(self.inner.home.project) {
            self.member_of(caller, project).await?;
        }
        Ok(())
    }

    /// Answers `project/list`: the projects `caller` works in but removed projects, and which of
    /// them have no folder here anymore.
    pub(crate) async fn list_projects(
        &self,
        caller: &Caller,
        limit: Option<usize>,
    ) -> Result<Reply, ApiError> {
        // The server's own project is listed for those who work in it, record or not.
        self.home_project().await?;
        let person = Principal::person(self.person_of(caller)?);
        let limit = limit.unwrap_or(100).clamp(1, 1_000);
        let projects = self
            .inner
            .projects
            .list(self.inner.home.org, &person, limit)
            .await
            .map_err(|e| failed(&e))?;
        let (projects, gone) = self.listed(projects).await?;
        Ok(Reply::Projects { projects, gone })
    }

    /// Answers `project/create`: a project owned by `caller`, with its folder when it is
    /// given one.
    pub(crate) async fn create_project(
        &self,
        caller: &Caller,
        name: String,
        members: Vec<Principal>,
        folder: Option<NewFolder>,
    ) -> Result<Reply, ApiError> {
        Project::check_name(&name).map_err(|e| failed(&e))?;
        let now = self.inner.harness.clock().now();
        let id = ThreadId::new(now, self.inner.harness.entropy().bytes()?)
            .map_err(|e| ApiError::new(code::INTERNAL, e.to_string()))?;
        let project = Project {
            id: ProjectId::from_uuid(id.as_uuid()),
            kind: ProjectKind::Project,
            name,
            owner: Principal::person(self.person_of(caller)?),
            members,
            created_at: At::from(now),
            updated_at: At::from(now),
        };
        if let Some(folder) = folder {
            return self.create_with_folder(caller, project, folder).await;
        }
        let project = self
            .inner
            .projects
            .create(self.inner.home.org, &project)
            .await
            .map_err(|e| failed(&e))?;
        Ok(Reply::Project {
            project,
            folder: None,
        })
    }

    /// Answers `project/get`: the project, and its folder when it has one.
    pub(crate) async fn get_project(
        &self,
        caller: &Caller,
        project: ProjectId,
    ) -> Result<Reply, ApiError> {
        let project = self.member_of(caller, project).await?;
        let folder = self.folder_of(project.id).await?;
        Ok(Reply::Project { project, folder })
    }

    /// Answers `project/rename`: for its owner only.
    pub(crate) async fn rename_project(
        &self,
        caller: &Caller,
        project: ProjectId,
        name: String,
    ) -> Result<Reply, ApiError> {
        let mut found = self.member_of(caller, project).await?;
        if found.owner != Principal::person(self.person_of(caller)?) {
            return Err(ApiError::new(
                code::NOT_ALLOWED,
                format!("only its owner renames project {project}"),
            ));
        }
        found.name = name;
        found.updated_at = At::from(self.inner.harness.clock().now());
        self.inner
            .projects
            .update(self.inner.home.org, &found)
            .await
            .map_err(|e| failed(&e))?;
        let folder = self.folder_of(project).await?;
        Ok(Reply::Project {
            project: found,
            folder,
        })
    }

    fn places(&self) -> std::sync::MutexGuard<'_, Places> {
        self.inner
            .places
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Remembers that `thread` is in `scope`.
    pub(crate) fn place(&self, thread: ThreadId, scope: Scope) {
        self.places().insert(thread, scope);
    }
}
