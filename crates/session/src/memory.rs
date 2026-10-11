//! The person's controls over memory (`specs/memory.md`, "Seeing, editing and forgetting"):
//! `memory/list`, `save`, `edit`, `move`, `forget` and `restore`. The caller and the server's
//! space decide what may be seen and changed, never the parameters: a space's members may change
//! its memory, and only the person their own.

use glave_harness::{Access, Caller, Control, MemoryError};
use glave_ports::{BlobStore, EventLog, ModelProvider};

use crate::protocol::{ApiError, Reply, Request, code};
use crate::server::Server;

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Answers a `memory/*` request for `caller`.
    pub(crate) async fn memory_request(
        &self,
        caller: &Caller,
        request: Request,
    ) -> Result<Reply, ApiError> {
        let memory = self
            .inner
            .memory
            .as_ref()
            .ok_or_else(|| ApiError::new(code::MEMORY_OFF, "memory is off for this server"))?;
        let control = match request {
            Request::MemoryList { scope } => Control::List { scope },
            Request::MemorySave { text, scope } => Control::Save { text, scope },
            Request::MemoryEdit { memory, text } => Control::Edit { memory, text },
            Request::MemoryMove { memory, scope } => Control::Move { memory, scope },
            Request::MemoryForget {
                memory: Some(memory),
                all: false,
                scope: None,
            } => Control::Forget { memory },
            Request::MemoryForget {
                memory: None,
                all: true,
                scope: Some(scope),
            } => Control::ForgetAll { scope },
            Request::MemoryForget { .. } => {
                return Err(ApiError::new(
                    code::INVALID_PARAMS,
                    "memory/forget takes `memory`, or `all: true` with a `scope`",
                ));
            }
            Request::MemoryRestore { memory } => Control::Restore { memory },
            _ => return Err(ApiError::new(code::INTERNAL, "not a memory request")),
        };
        let access = Access {
            person: caller.person,
            scope: self.inner.scope,
        };
        match memory.control(&access, control).await {
            Ok(memories) => Ok(Reply::Memories { memories }),
            Err(error) => Err(refused(&error)),
        }
    }
}

fn refused(error: &MemoryError) -> ApiError {
    let code = match error {
        MemoryError::NotFound(_) => code::NO_MEMORY,
        MemoryError::Refused(_) => code::NOT_ALLOWED,
        MemoryError::Invalid(_) => code::INVALID_PARAMS,
        MemoryError::Off(_) => code::MEMORY_OFF,
        MemoryError::Storage(_) => code::INTERNAL,
    };
    ApiError::new(code, error.to_string())
}
