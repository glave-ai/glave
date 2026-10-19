//! Projects over the session API, and who may do what in them (`specs/session.md`, "Projects and
//! who may do what"). A server serves many projects of one organization: its own, any other its
//! callers work in, and each caller's Chats, where requests that name none act. Every request
//! that acts on a thread, or
//! on the server's own project as a whole, is checked against the caller: they must own the
//! thread's project or be one of its members, or it is answered before anything is read or
//! recorded exactly as a thread or project that does not exist is, so whether it exists never
//! leaks.

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

/// The projects of threads a server has looked up, in its organization. A thread's project
/// never changes.
#[derive(Debug, Default)]
pub(crate) struct Places(HashMap<ThreadId, ProjectId>);

impl Places {
    fn get(&self, thread: ThreadId) -> Option<ProjectId> {
        self.0.get(&thread).copied()
    }

    pub(crate) fn insert(&mut self, thread: ThreadId, project: ProjectId) {
        if self.0.len() >= PLACES {
            self.0.clear();
        }
        self.0.insert(thread, project);
    }
}

pub(crate) fn no_project(project: ProjectId) -> ApiError {
    ApiError::new(code::NO_PROJECT, format!("no project {project} here"))
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
        if let Some(project) = self.inner.projects.get(home.org(), home.project()).await? {
            return Ok(project);
        }
        let now = At::from(self.inner.harness.clock().now());
        let project = Project {
            id: home.project(),
            kind: ProjectKind::Project,
            name: "this folder".to_owned(),
            owner: Principal::person(self.person_of(&Caller::default())?),
            members: Vec::new(),
            created_at: now,
            updated_at: now,
        };
        match self.inner.projects.create(home.org(), &project).await {
            Ok(kept) => Ok(kept),
            // Another process kept it first.
            Err(ProjectError::Conflict(_)) => self
                .inner
                .projects
                .get(home.org(), home.project())
                .await?
                .ok_or_else(|| no_project(home.project())),
            Err(error) => Err(error.into()),
        }
    }

    /// The project `project` of this server's organization.
    async fn project(&self, project: ProjectId) -> Result<Project, ApiError> {
        if project == self.inner.home.project() {
            return self.home_project().await;
        }
        self.inner
            .projects
            .get(self.inner.home.org(), project)
            .await?
            .ok_or_else(|| no_project(project))
    }

    /// The one check of who may use a project's scope: `project`, read from this server's
    /// organization, and the scope `caller` may read and write it with, when they own it or are
    /// one of its members; otherwise "no project", as for one that does not exist
    /// (`specs/invariants.md`, 17).
    pub(crate) async fn authorized(
        &self,
        caller: &Caller,
        project: ProjectId,
    ) -> Result<(Project, Scope), ApiError> {
        let found = self.project(project).await?;
        let person = self.person_of(caller)?;
        let scope = Scope::of_member(self.inner.home.org(), &found, person)
            .ok_or_else(|| no_project(project))?;
        Ok((found, scope))
    }

    /// The project `project`, when `caller` works in it; see [`Server::authorized`].
    pub(crate) async fn member_of(
        &self,
        caller: &Caller,
        project: ProjectId,
    ) -> Result<Project, ApiError> {
        Ok(self.authorized(caller, project).await?.0)
    }

    /// The caller's personal project, made the first time, and its scope.
    pub(crate) async fn personal_project(
        &self,
        caller: &Caller,
    ) -> Result<(Project, Scope), ApiError> {
        let now = self.inner.harness.clock().now();
        let id = ThreadId::new(now, self.inner.harness.entropy().bytes()?)?;
        let new = Project {
            id: ProjectId::from_uuid(id.as_uuid()),
            kind: ProjectKind::Personal,
            name: PERSONAL_NAME.to_owned(),
            owner: Principal::person(self.person_of(caller)?),
            members: Vec::new(),
            created_at: At::from(now),
            updated_at: At::from(now),
        };
        let kept = self
            .inner
            .projects
            .personal(self.inner.home.org(), &new)
            .await?;
        let person = self.person_of(caller)?;
        let scope = Scope::of_member(self.inner.home.org(), &kept, person)
            .ok_or_else(|| no_project(kept.id))?;
        Ok((kept, scope))
    }

    /// The project a request acts in, and the caller's scope in it: `asked`, when the caller
    /// works in it; with none, their personal project, their Chats, whatever folder the server
    /// started in.
    pub(crate) async fn project_for(
        &self,
        caller: &Caller,
        asked: Option<ProjectId>,
    ) -> Result<(Project, Scope), ApiError> {
        match asked {
            Some(project) => self.authorized(caller, project).await,
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
            .list(self.inner.home.org(), &person, MOST_PROJECTS)
            .await?;
        Ok(projects.into_iter().map(|project| project.id).collect())
    }

    /// The project `thread` is in, in this server's organization: remembered, or found in the
    /// log. Where it is, not who may read it.
    async fn located(&self, thread: ThreadId) -> Result<ProjectId, ApiError> {
        if glave_ports::stream::Reserved::of(thread).is_some() {
            return Err(no_thread(thread));
        }
        if let Some(project) = self.places().get(thread) {
            return Ok(project);
        }
        if let Some(Slot::Idle { thread: held, .. }) = self.lock().get(&thread) {
            return Ok(held.scope.project());
        }
        let located = self
            .inner
            .harness
            .log()
            .locate(thread)
            .await?
            .filter(|located| located.org == self.inner.home.org())
            .ok_or_else(|| no_thread(thread))?;
        self.places().insert(thread, located.project);
        Ok(located.project)
    }

    /// The scope `thread` is stored in, for work on it after the request was let through by
    /// [`Server::allow`], which checked the caller against the thread's project, or for glave's
    /// own work on it (tasks, schedules, titles, recovery).
    pub(crate) async fn scope_of(&self, thread: ThreadId) -> Result<Scope, ApiError> {
        let project = self.located(thread).await?;
        Ok(Scope::glaves_own(self.inner.home.org(), project))
    }

    /// The scope of `thread`, when `caller` works in its project; otherwise "no such thread",
    /// as for one that does not exist.
    pub(crate) async fn authorize(
        &self,
        caller: &Caller,
        thread: ThreadId,
    ) -> Result<Scope, ApiError> {
        let project = self.located(thread).await?;
        let (_, scope) = self
            .authorized(caller, project)
            .await
            .map_err(|refused| match refused.code {
                code::NO_PROJECT => no_thread(thread),
                _ => refused,
            })?;
        Ok(scope)
    }

    /// Refuses `request` unless `caller` may make it: for a thread, they work in its project;
    /// for a project's memory, in that project, the server's own when it names none.
    pub(crate) async fn allow(&self, caller: &Caller, request: &Request) -> Result<(), ApiError> {
        if let Some(thread) = request.thread() {
            self.authorize(caller, thread).await?;
        } else if let Some(project) = request.memory_project(self.inner.home.project()) {
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
            .list(self.inner.home.org(), &person, limit)
            .await?;
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
        Project::check_name(&name)?;
        let now = self.inner.harness.clock().now();
        let id = ThreadId::new(now, self.inner.harness.entropy().bytes()?)?;
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
            .create(self.inner.home.org(), &project)
            .await?;
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
            .update(self.inner.home.org(), &found)
            .await?;
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
        self.places().insert(thread, scope.project());
    }
}
