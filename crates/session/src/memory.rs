//! The person's controls over memory (`specs/memory.md`, "Seeing, editing and forgetting"):
//! `memory/list`, `save`, `edit`, `move`, `forget` and `restore`, in the project a request names
//! or the caller's Chats when it names none. The caller decides what may be seen and changed: a project's members may
//! change its memory, and only the person their own.

use glave_ports::memories::{Access, Control};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider, ProjectId};

use crate::protocol::{ApiError, Reply, code};
use crate::server::Server;
use crate::words::Sentence;

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Answers a `memory/*` request for `caller`: `control` on the memory of `project` (their
    /// Chats when it names none) and the caller's own.
    pub(crate) async fn remember(
        &self,
        caller: &Caller,
        project: Option<ProjectId>,
        control: Control,
    ) -> Result<Reply, ApiError> {
        let memory = self
            .inner
            .harness
            .memory()
            .ok_or_else(|| ApiError::new(code::MEMORY_OFF, Sentence::MemoryOff))?;
        let (_, scope) = self.project_for(caller, project).await?;
        let access = Access {
            person: caller.person,
            scope,
        };
        match memory.control(&access, control).await {
            Ok(memories) => Ok(Reply::Memories { memories }),
            Err(error) => Err(error.into()),
        }
    }
}

/// What `memory/forget` forgets: one memory, or with `all`, every memory of a scope.
pub(crate) fn forget(
    memory: Option<glave_event::MemoryId>,
    all: bool,
    scope: Option<glave_event::MemoryScope>,
) -> Result<Control, ApiError> {
    match (memory, all, scope) {
        (Some(memory), false, None) => Ok(Control::Forget { memory }),
        (None, true, Some(scope)) => Ok(Control::ForgetAll { scope }),
        _ => Err(ApiError::new(code::INVALID_PARAMS, Sentence::ForgetWhat)),
    }
}
