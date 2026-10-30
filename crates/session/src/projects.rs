//! Projects over the session API, and who may do what in them (`specs/session.md`, "Projects and
//! who may do what"). A server serves many projects, in the organization each request works in
//! ("Who is calling"), with none of its own: those its callers work in, and each caller's Chats,
//! where requests that name none act. Every
//! request that acts on a thread, or on a project, is checked against the caller: they must own
//! the project, or it is answered before anything is read or recorded exactly as a thread or
//! project that does not exist is, so whether it exists never leaks.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use glave_event::{At, Principal, PrincipalId, ThreadId};
use glave_ports::{
    BlobStore, Caller, EventLog, Located, ModelProvider, OrgId, Project, ProjectId, ProjectKind,
    Scope,
};

use crate::held::Slot;
use crate::protocol::{ApiError, NewFolder, Reply, Request, code};
use crate::server::{Server, no_thread};
use crate::words::Sentence;

/// Threads whose project a server remembers; past this, it forgets them all and asks again.
const PLACES: usize = 4_096;
/// The most projects whose tasks and schedules one person sees.
const MOST_PROJECTS: usize = 1_000;

/// What a personal project is called until its person renames it.
pub const PERSONAL_NAME: &str = "Chats";

/// Where the threads a server has looked up are, their organization and project, which never
/// change; and how many requests in flight hold each, which keeps it while they do.
#[derive(Debug, Default)]
pub(crate) struct Places(HashMap<ThreadId, (Located, usize)>);

impl Places {
    fn get(&self, thread: ThreadId) -> Option<Located> {
        self.0.get(&thread).map(|(located, _)| *located)
    }

    /// Remembers `thread` is at `located`, forgetting every place no request holds once
    /// [`PLACES`] are kept.
    pub(crate) fn insert(&mut self, thread: ThreadId, located: Located) {
        if self.0.len() >= PLACES {
            self.0.retain(|_, (_, held)| *held > 0);
        }
        self.0.entry(thread).or_insert((located, 0)).0 = located;
    }
}

/// A thread's place held for a request in flight: `scope_of` finds it until this is dropped,
/// however many other threads are looked up meanwhile.
pub(crate) struct Held {
    places: Arc<Mutex<Places>>,
    thread: ThreadId,
}

impl Drop for Held {
    fn drop(&mut self) {
        let mut places = self.places.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, held)) = places.0.get_mut(&self.thread) {
            *held = held.saturating_sub(1);
        }
    }
}

pub(crate) fn no_project(project: ProjectId) -> ApiError {
    ApiError::new(
        code::NO_PROJECT,
        Sentence::NoProject {
            project: project.to_string(),
        },
    )
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// The organization `caller`'s request works in: the one the transport established for
    /// them, else the server's own (`specs/session.md`, "Who is calling").
    pub(crate) fn org_of(&self, caller: &Caller) -> OrgId {
        caller.org.unwrap_or(self.inner.own)
    }

    /// The person `caller` acts as: the one the transport verified, or the local person.
    pub(crate) fn person_of(&self, caller: &Caller) -> Result<PrincipalId, ApiError> {
        caller
            .person
            .or_else(|| self.inner.harness.person())
            .ok_or_else(|| ApiError::new(code::INTERNAL, Sentence::NoPerson))
    }

    /// The project `project` of `org`.
    pub(crate) async fn project(
        &self,
        org: OrgId,
        project: ProjectId,
    ) -> Result<Project, ApiError> {
        self.inner
            .projects
            .get(org, project)
            .await?
            .ok_or_else(|| no_project(project))
    }

    /// The one check of who may use a project's scope: `project`, read from the organization
    /// the request works in, and the scope `caller` may read and write it with, when they own it;
    /// otherwise "no project", as for one that does not exist (`specs/invariants.md`, 17).
    pub(crate) async fn authorized(
        &self,
        caller: &Caller,
        project: ProjectId,
    ) -> Result<(Project, Scope), ApiError> {
        let org = self.org_of(caller);
        let found = self.project(org, project).await?;
        let person = self.person_of(caller)?;
        let scope = Scope::of_owner(org, &found, person).ok_or_else(|| no_project(project))?;
        Ok((found, scope))
    }

    /// The project `project`, when `caller` owns it; see [`Server::authorized`].
    pub(crate) async fn owned(
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
            created_at: At::from(now),
            updated_at: At::from(now),
        };
        let org = self.org_of(caller);
        let kept = self.inner.projects.personal(org, &new).await?;
        let person = self.person_of(caller)?;
        let scope = Scope::of_owner(org, &kept, person).ok_or_else(|| no_project(kept.id))?;
        Ok((kept, scope))
    }

    /// The project a request acts in, and the caller's scope in it: `asked`, when the caller
    /// owns it; with none, their personal project, their Chats, whatever folder the server
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

    /// The projects `caller` owns, in the organization the request works in: those whose tasks
    /// and schedules they see.
    pub(crate) async fn projects_of(&self, caller: &Caller) -> Result<Vec<Located>, ApiError> {
        let org = self.org_of(caller);
        let person = Principal::person(self.person_of(caller)?);
        let projects = self
            .inner
            .projects
            .list(org, &person, MOST_PROJECTS)
            .await?;
        let projects = projects.into_iter();
        Ok(projects
            .map(|project| Located {
                org,
                project: project.id,
            })
            .collect())
    }

    /// Where `thread` is, remembered or held, whichever organization holds it.
    fn remembered(&self, thread: ThreadId) -> Option<Located> {
        if let Some(located) = self.places().get(thread) {
            return Some(located);
        }
        match self.lock().get(&thread)? {
            Slot::Idle { thread: held, .. } => Some(held.scope.located()),
            Slot::Busy { at, .. } => Some(*at),
        }
    }

    /// Where `thread` is in `org`: remembered, or found in the log. Where it is, not who may
    /// read it; a thread of another organization is not found.
    async fn located(&self, org: OrgId, thread: ThreadId) -> Result<Located, ApiError> {
        if glave_ports::stream::Reserved::of(thread).is_some() {
            return Err(no_thread(thread));
        }
        if let Some(located) = self.remembered(thread).filter(|found| found.org == org) {
            return Ok(located);
        }
        let located = self
            .inner
            .harness
            .log()
            .locate(org, thread)
            .await?
            .ok_or_else(|| no_thread(thread))?;
        self.places().insert(thread, located);
        Ok(located)
    }

    /// The scope `thread` is stored in, for work on it after the request was let through by
    /// [`Server::allow`], which checked the caller against the thread's project and remembered
    /// where it is, or for glave's own work on a thread this server started or holds (tasks,
    /// schedules, titles, recovery). A thread it knows nothing of is not found: where it is
    /// comes only from a caller's organization.
    pub(crate) fn scope_of(&self, thread: ThreadId) -> Result<Scope, ApiError> {
        let located = self.remembered(thread).ok_or_else(|| no_thread(thread))?;
        Ok(Scope::glaves_own(located.org, located.project))
    }

    /// The scope of `thread`, when `caller` owns its project; otherwise "no such thread", as
    /// for one that does not exist.
    pub(crate) async fn authorize(
        &self,
        caller: &Caller,
        thread: ThreadId,
    ) -> Result<Scope, ApiError> {
        let located = self.located(self.org_of(caller), thread).await?;
        let (_, scope) = self
            .authorized(caller, located.project)
            .await
            .map_err(|refused| match refused.code {
                code::NO_PROJECT => no_thread(thread),
                _ => refused,
            })?;
        Ok(scope)
    }

    /// Refuses `request` unless `caller` may make it: for an admin method, their role
    /// permits it; for a thread, they own its project, whose place is held while the request
    /// runs; for a project's memory, the project it names (with none, their own Chats).
    pub(crate) async fn allow(
        &self,
        caller: &Caller,
        request: &Request,
    ) -> Result<Option<Held>, ApiError> {
        if let Some(needed) = request.needs() {
            self.permitted(caller, needed).await?;
        } else if let Some(thread) = request.thread() {
            let scope = self.authorize(caller, thread).await?;
            return Ok(Some(self.hold(thread, scope)));
        } else if let Some(project) = request.memory_project() {
            self.owned(caller, project).await?;
        }
        Ok(None)
    }

    /// Remembers that `thread` is in `scope`, and keeps it while the [`Held`] lives.
    pub(crate) fn hold(&self, thread: ThreadId, scope: Scope) -> Held {
        let mut places = self.places();
        places.insert(thread, scope.located());
        if let Some((_, held)) = places.0.get_mut(&thread) {
            *held = held.saturating_add(1);
        }
        Held {
            places: Arc::clone(&self.inner.places),
            thread,
        }
    }

    /// Answers `project/list`: the projects `caller` owns but removed projects, and which of
    /// them have no folder here anymore.
    pub(crate) async fn list_projects(
        &self,
        caller: &Caller,
        limit: Option<usize>,
    ) -> Result<Reply, ApiError> {
        let person = Principal::person(self.person_of(caller)?);
        let limit = limit.unwrap_or(100).clamp(1, 1_000);
        let projects = self
            .inner
            .projects
            .list(self.org_of(caller), &person, limit)
            .await?;
        let (projects, gone) = self.listed(self.org_of(caller), projects).await?;
        Ok(Reply::Projects { projects, gone })
    }

    /// Answers `project/create`: a project owned by `caller`, with its folder when it is
    /// given one.
    pub(crate) async fn create_project(
        &self,
        caller: &Caller,
        name: String,
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
            created_at: At::from(now),
            updated_at: At::from(now),
        };
        if let Some(folder) = folder {
            return self.create_with_folder(caller, project, folder).await;
        }
        let project = self
            .inner
            .projects
            .create(self.org_of(caller), &project)
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
        let (project, scope) = self.authorized(caller, project).await?;
        let folder = self.folder_of(scope.located()).await?;
        Ok(Reply::Project { project, folder })
    }

    /// Answers `project/rename`: the last rename wins.
    pub(crate) async fn rename_project(
        &self,
        caller: &Caller,
        project: ProjectId,
        name: String,
    ) -> Result<Reply, ApiError> {
        self.owned(caller, project).await?;
        let at = At::from(self.inner.harness.clock().now());
        let renamed = self
            .inner
            .projects
            .rename(self.org_of(caller), project, &name, at)
            .await?;
        let folder = self
            .folder_of(Located {
                org: self.org_of(caller),
                project,
            })
            .await?;
        Ok(Reply::Project {
            project: renamed,
            folder,
        })
    }

    fn places(&self) -> std::sync::MutexGuard<'_, Places> {
        self.inner
            .places
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Remembers that `thread` is in `scope`.
    pub(crate) fn place(&self, thread: ThreadId, scope: Scope) {
        self.places().insert(thread, scope.located());
    }
}
