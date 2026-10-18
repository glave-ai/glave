//! Spaces over the session API, and who may do what in them (`specs/session.md`, "Spaces and
//! who may do what"). A server serves many spaces of one organization: its own, any other its
//! callers work in, and each caller's Chats, where requests that name none act. Every request
//! that acts on a thread, or
//! on the server's own space as a whole, is checked against the caller: they must own the
//! thread's space or be one of its members, or it is refused with `-32012` before anything is
//! read or recorded.

use std::collections::HashMap;

use glave_event::{At, Principal, PrincipalId, ThreadId};
use glave_harness::Caller;
use glave_ports::{
    BlobStore, EventLog, ModelProvider, Scope, Space, SpaceError, SpaceId, SpaceKind,
};

use crate::held::Slot;
use crate::protocol::{ApiError, NewFolder, Reply, Request, code};
use crate::server::{Server, no_thread};

/// Threads whose space a server remembers; past this, it forgets them all and asks again.
const PLACES: usize = 4_096;
/// The most spaces whose tasks and schedules one person sees.
const MOST_SPACES: usize = 1_000;

/// What a personal space is called until its person renames it.
pub const PERSONAL_NAME: &str = "Chats";

/// The scopes of threads a server has looked up. A thread's space never changes.
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

pub(crate) fn failed(error: &SpaceError) -> ApiError {
    match error {
        SpaceError::NotFound(space) => no_space(*space),
        SpaceError::Conflict(_) | SpaceError::Invalid(_) => {
            ApiError::new(code::INVALID_PARAMS, error.to_string())
        }
        SpaceError::Storage(_) => ApiError::new(code::INTERNAL, error.to_string()),
    }
}

fn no_space(space: SpaceId) -> ApiError {
    ApiError::new(code::NO_SPACE, format!("no space {space} here"))
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

    /// The server's own space: the folder it serves. Kept as the local person's project when
    /// the store has no record of it yet.
    pub async fn home_space(&self) -> Result<Space, ApiError> {
        let home = self.inner.home;
        if let Some(space) = self
            .inner
            .spaces
            .get(home.org, home.space)
            .await
            .map_err(|e| failed(&e))?
        {
            return Ok(space);
        }
        let now = At::from(self.inner.harness.clock().now());
        let space = Space {
            id: home.space,
            kind: SpaceKind::Project,
            name: "this folder".to_owned(),
            owner: Principal::person(self.person_of(&Caller::default())?),
            members: Vec::new(),
            created_at: now,
            updated_at: now,
        };
        match self.inner.spaces.create(home.org, &space).await {
            Ok(kept) => Ok(kept),
            // Another process kept it first.
            Err(SpaceError::Conflict(_)) => self
                .inner
                .spaces
                .get(home.org, home.space)
                .await
                .map_err(|e| failed(&e))?
                .ok_or_else(|| no_space(home.space)),
            Err(error) => Err(failed(&error)),
        }
    }

    /// The space `space` of this server's organization.
    async fn space(&self, space: SpaceId) -> Result<Space, ApiError> {
        if space == self.inner.home.space {
            return self.home_space().await;
        }
        self.inner
            .spaces
            .get(self.inner.home.org, space)
            .await
            .map_err(|e| failed(&e))?
            .ok_or_else(|| no_space(space))
    }

    /// The space `space`, when `caller` works in it; refused with `-32012` otherwise.
    pub(crate) async fn member_of(
        &self,
        caller: &Caller,
        space: SpaceId,
    ) -> Result<Space, ApiError> {
        let found = self.space(space).await?;
        if found.admits_person(self.person_of(caller)?) {
            Ok(found)
        } else {
            Err(not_allowed(&format!("space {space}")))
        }
    }

    /// The caller's personal space, made the first time.
    pub(crate) async fn personal_space(&self, caller: &Caller) -> Result<Space, ApiError> {
        let now = self.inner.harness.clock().now();
        let id = ThreadId::new(now, self.inner.harness.entropy().bytes())
            .map_err(|e| ApiError::new(code::INTERNAL, e.to_string()))?;
        let new = Space {
            id: SpaceId::from_uuid(id.as_uuid()),
            kind: SpaceKind::Personal,
            name: PERSONAL_NAME.to_owned(),
            owner: Principal::person(self.person_of(caller)?),
            members: Vec::new(),
            created_at: At::from(now),
            updated_at: At::from(now),
        };
        self.inner
            .spaces
            .personal(self.inner.home.org, &new)
            .await
            .map_err(|e| failed(&e))
    }

    /// The space a request acts in: `asked`, when the caller works in it; with none, their
    /// personal space, their Chats, whatever folder the server started in.
    pub(crate) async fn space_for(
        &self,
        caller: &Caller,
        asked: Option<SpaceId>,
    ) -> Result<Space, ApiError> {
        match asked {
            Some(space) => self.member_of(caller, space).await,
            None => self.personal_space(caller).await,
        }
    }

    /// The spaces `caller` works in, the server's own among them when they do: those whose
    /// tasks and schedules they see.
    pub(crate) async fn spaces_of(&self, caller: &Caller) -> Result<Vec<SpaceId>, ApiError> {
        // The server's own space is kept first, so it is listed for those who work in it.
        self.home_space().await?;
        let person = Principal::person(self.person_of(caller)?);
        let spaces = self
            .inner
            .spaces
            .list(self.inner.home.org, &person, MOST_SPACES)
            .await
            .map_err(|e| failed(&e))?;
        Ok(spaces.into_iter().map(|space| space.id).collect())
    }

    /// The scope `thread` is stored in: remembered, or found in the log.
    pub(crate) async fn scope_of(&self, thread: ThreadId) -> Result<Scope, ApiError> {
        if crate::ledger::is_ledger(thread) {
            return Err(no_thread(thread));
        }
        if let Some(scope) = self.places().get(thread) {
            return Ok(scope);
        }
        if let Some(Slot::Idle(held)) = self.lock().get(&thread) {
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

    /// The scope of `thread`, when `caller` works in its space; refused with `-32012`
    /// otherwise, and with "no such thread" when there is none.
    pub(crate) async fn authorize(
        &self,
        caller: &Caller,
        thread: ThreadId,
    ) -> Result<Scope, ApiError> {
        let scope = self.scope_of(thread).await?;
        self.member_of(caller, scope.space)
            .await
            .map_err(|refused| match refused.code {
                code::NOT_ALLOWED => not_allowed(&format!("the space of thread {thread}")),
                _ => refused,
            })?;
        Ok(scope)
    }

    /// Refuses `request` unless `caller` may make it: for a thread, they work in its space;
    /// for a space's memory, in that space, the server's own when it names none.
    pub(crate) async fn allow(&self, caller: &Caller, request: &Request) -> Result<(), ApiError> {
        if let Some(thread) = request.thread() {
            self.authorize(caller, thread).await?;
        } else if let Some(space) = request.memory_space(self.inner.home.space) {
            self.member_of(caller, space).await?;
        }
        Ok(())
    }

    /// Answers `space/list`: the spaces `caller` works in but removed projects, and which of
    /// them have no folder here anymore.
    pub(crate) async fn list_spaces(
        &self,
        caller: &Caller,
        limit: Option<usize>,
    ) -> Result<Reply, ApiError> {
        // The server's own space is listed for those who work in it, record or not.
        self.home_space().await?;
        let person = Principal::person(self.person_of(caller)?);
        let limit = limit.unwrap_or(100).clamp(1, 1_000);
        let spaces = self
            .inner
            .spaces
            .list(self.inner.home.org, &person, limit)
            .await
            .map_err(|e| failed(&e))?;
        let (spaces, gone) = self.listed(spaces).await?;
        Ok(Reply::Spaces { spaces, gone })
    }

    /// Answers `space/create`: a project space owned by `caller`, with its folder when it is
    /// given one.
    pub(crate) async fn create_space(
        &self,
        caller: &Caller,
        name: String,
        members: Vec<Principal>,
        folder: Option<NewFolder>,
    ) -> Result<Reply, ApiError> {
        Space::check_name(&name).map_err(|e| failed(&e))?;
        let now = self.inner.harness.clock().now();
        let id = ThreadId::new(now, self.inner.harness.entropy().bytes())
            .map_err(|e| ApiError::new(code::INTERNAL, e.to_string()))?;
        let space = Space {
            id: SpaceId::from_uuid(id.as_uuid()),
            kind: SpaceKind::Project,
            name,
            owner: Principal::person(self.person_of(caller)?),
            members,
            created_at: At::from(now),
            updated_at: At::from(now),
        };
        if let Some(folder) = folder {
            return self.create_project(caller, space, folder).await;
        }
        let space = self
            .inner
            .spaces
            .create(self.inner.home.org, &space)
            .await
            .map_err(|e| failed(&e))?;
        Ok(Reply::Space {
            space,
            folder: None,
        })
    }

    /// Answers `space/get`: the space, and its folder when it has one.
    pub(crate) async fn get_space(
        &self,
        caller: &Caller,
        space: SpaceId,
    ) -> Result<Reply, ApiError> {
        let space = self.member_of(caller, space).await?;
        let folder = self.folder_of(space.id).await?;
        Ok(Reply::Space { space, folder })
    }

    /// Answers `space/rename`: for its owner only.
    pub(crate) async fn rename_space(
        &self,
        caller: &Caller,
        space: SpaceId,
        name: String,
    ) -> Result<Reply, ApiError> {
        let mut found = self.member_of(caller, space).await?;
        if found.owner != Principal::person(self.person_of(caller)?) {
            return Err(ApiError::new(
                code::NOT_ALLOWED,
                format!("only its owner renames space {space}"),
            ));
        }
        found.name = name;
        found.updated_at = At::from(self.inner.harness.clock().now());
        self.inner
            .spaces
            .update(self.inner.home.org, &found)
            .await
            .map_err(|e| failed(&e))?;
        let folder = self.folder_of(space).await?;
        Ok(Reply::Space {
            space: found,
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
