//! Spaces over the session API, and who may do what in them (`specs/session.md`, "Spaces and
//! who may do what"). A server serves many spaces of one organization: its own (the folder it
//! was started in) and any other its callers work in. Every request that acts on a thread, or
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
use crate::protocol::{ApiError, Reply, Request, code};
use crate::server::{Server, no_thread};

/// Threads whose space a server remembers; past this, it forgets them all and asks again.
const PLACES: usize = 4_096;

/// What a personal space is called until its person renames it.
pub const PERSONAL_NAME: &str = "Inbox";

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

fn failed(error: &SpaceError) -> ApiError {
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
        let now = At::from(self.inner.clock.now());
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
            Ok(()) => Ok(space),
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
        let now = self.inner.clock.now();
        let id = ThreadId::new(now, self.inner.entropy.bytes())
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

    /// The space a request without one acts in: `asked`, when the caller works in it; else
    /// the server's own, when they work in it; else their personal space.
    pub(crate) async fn space_for(
        &self,
        caller: &Caller,
        asked: Option<SpaceId>,
    ) -> Result<Space, ApiError> {
        if let Some(space) = asked {
            return self.member_of(caller, space).await;
        }
        let home = self.home_space().await?;
        if home.admits_person(self.person_of(caller)?) {
            return Ok(home);
        }
        self.personal_space(caller).await
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
    /// for the server's tasks, schedules and memory, in the server's own space.
    pub(crate) async fn allow(&self, caller: &Caller, request: &Request) -> Result<(), ApiError> {
        if let Some(thread) = request.thread() {
            self.authorize(caller, thread).await?;
        } else if request.in_home() {
            self.member_of(caller, self.inner.home.space).await?;
        }
        Ok(())
    }

    /// Answers `space/*` for `caller`.
    pub(crate) async fn space_request(
        &self,
        caller: &Caller,
        request: Request,
    ) -> Result<Reply, ApiError> {
        let org = self.inner.home.org;
        match request {
            Request::SpaceList { limit } => {
                // The server's own space is listed for those who work in it, record or not.
                self.home_space().await?;
                let person = Principal::person(self.person_of(caller)?);
                let limit = limit.unwrap_or(100).clamp(1, 1_000);
                let spaces = self
                    .inner
                    .spaces
                    .list(org, &person, limit)
                    .await
                    .map_err(|e| failed(&e))?;
                Ok(Reply::Spaces { spaces })
            }
            Request::SpaceGet { space } => Ok(Reply::Space {
                space: self.member_of(caller, space).await?,
            }),
            Request::SpaceCreate { name, members } => {
                let now = self.inner.clock.now();
                let id = ThreadId::new(now, self.inner.entropy.bytes())
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
                self.inner
                    .spaces
                    .create(org, &space)
                    .await
                    .map_err(|e| failed(&e))?;
                Ok(Reply::Space { space })
            }
            Request::SpaceRename { space, name } => {
                let mut found = self.member_of(caller, space).await?;
                if found.owner != Principal::person(self.person_of(caller)?) {
                    return Err(ApiError::new(
                        code::NOT_ALLOWED,
                        format!("only its owner renames space {space}"),
                    ));
                }
                found.name = name;
                found.updated_at = At::from(self.inner.clock.now());
                self.inner
                    .spaces
                    .update(org, &found)
                    .await
                    .map_err(|e| failed(&e))?;
                Ok(Reply::Space { space: found })
            }
            _ => Err(ApiError::new(code::INTERNAL, "not a space request")),
        }
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
