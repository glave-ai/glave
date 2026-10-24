//! The person's controls over memory (`specs/memory.md`, "Seeing, editing and forgetting"):
//! `memory/list`, `save`, `edit`, `move`, `forget` and `restore`, in the project a request names
//! or the caller's Chats when it names none. The caller decides what may be seen and changed:
//! memory is the person's own, in their projects as in their Chats, and a memory names the chat
//! it was saved in only to the person it was started for.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use glave_event::ThreadId;
use glave_ports::memories::{Access, Control};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider, ProjectId, Standing};

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
    /// Chats when it names none) and the caller's own, each memory as they are shown it: its
    /// `source` names only a chat started for them that is still there, so no one learns of
    /// another's.
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
            person: self.person_of(caller)?,
            scope,
        };
        let mut memories = memory.control(&access, control).await?;
        // Each chat looked up once: one chat often holds several memories.
        let mut theirs = BTreeMap::new();
        for memory in &mut memories {
            let Some(thread) = memory.source.map(|source| source.thread) else {
                continue;
            };
            if let Entry::Vacant(unknown) = theirs.entry(thread) {
                unknown.insert(self.theirs(caller, thread).await);
            }
            if theirs.get(&thread) != Some(&true) {
                memory.source = None;
            }
        }
        Ok(Reply::Memories { memories })
    }

    /// Whether `thread` is a chat started for `caller`, in a project they work in, and not
    /// deleted: the only chat a memory names to them.
    async fn theirs(&self, caller: &Caller, thread: ThreadId) -> bool {
        let (Ok(person), Ok(scope)) =
            (self.person_of(caller), self.authorize(caller, thread).await)
        else {
            return false;
        };
        let Ok(Some(view)) = self.inner.harness.log().thread_view(&scope, thread).await else {
            return false;
        };
        let whose = view.whose().or_else(|| self.person());
        view.standing != Standing::Deleted && whose == Some(person)
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
