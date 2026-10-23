//! The person's controls over memory (`specs/memory.md`, "Seeing, editing and forgetting"):
//! `memory/list`, `save`, `edit`, `move`, `forget` (with the chat a memory was saved in, when
//! it is the caller's own and asked for) and `restore`, in the project a request names or the
//! caller's Chats when it names none. The caller decides what may be seen and changed: a
//! project's members may change its memory, and only the person their own; a memory names the
//! chat it was saved in only to the person it was started for. And what a memory call's result
//! kept by reference shows the caller, read again as they may read memory.

use std::collections::BTreeMap;

use glave_event::{Body, CallId, RawEvent, ThreadId, decode};
use glave_harness::Change;
use glave_ports::memories::{Access, Control, MemoryContext, Remembered, SHOWN};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider, ProjectId, Scope, Standing};

use crate::protocol::{ApiError, Reply, Told, code};
use crate::server::Server;
use crate::words::Sentence;

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Answers a `memory/*` request for `caller`: `control` on the memory of `project` (their
    /// Chats when it names none) and the caller's own, each memory as they are shown it.
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
        let memories = memory.control(&access, control).await?;
        let mut shown = Vec::with_capacity(memories.len());
        for mut memory in memories {
            if let Some(source) = memory.source
                && !self.theirs(caller, source.thread).await
            {
                memory.source = None;
            }
            shown.push(memory);
        }
        Ok(Reply::Memories {
            memories: shown,
            thread_kept: None,
        })
    }

    /// Whether `thread` is a chat started for `caller`, in a project they work in, and not
    /// deleted: the only chat a memory names to them, and the only one that goes with it.
    async fn theirs(&self, caller: &Caller, thread: ThreadId) -> bool {
        let (Ok(person), Ok(scope)) =
            (self.person_of(caller), self.authorize(caller, thread).await)
        else {
            return false;
        };
        let Ok(Some(view)) = self.inner.harness.log().thread_view(&scope, thread).await else {
            return false;
        };
        let whose = view
            .for_
            .and_then(|whose| whose.id)
            .or_else(|| self.person());
        view.standing != Standing::Deleted && whose == Some(person)
    }

    /// Answers `memory/forget` for `caller`, as [`Server::remember`] does, then, with
    /// `delete_thread`, deletes the chat the memory was saved in as `thread/delete` does, when
    /// it is the caller's own; any other is left as it is, and the answer says so.
    pub(crate) async fn forget(
        &self,
        caller: &Caller,
        project: Option<ProjectId>,
        control: Control,
        delete_thread: bool,
    ) -> Result<Reply, ApiError> {
        let reply = self.remember(caller, project, control).await?;
        let Reply::Memories { memories, .. } = reply else {
            return Ok(reply);
        };
        if !delete_thread {
            return Ok(Reply::Memories {
                memories,
                thread_kept: None,
            });
        }
        let mut deleted = false;
        for thread in memories.iter().filter_map(saved_in) {
            match self.change(caller, thread, Change::Delete).await {
                Ok(_) => deleted = true,
                Err(gone) if gone.code == code::NO_THREAD => {}
                Err(refused) => return Err(refused),
            }
        }
        Ok(Reply::Memories {
            memories,
            thread_kept: (!deleted).then(|| Told::from(Sentence::ThreadKept)),
        })
    }

    /// By call, the words each memory call's result among `events` of `thread` in `scope`
    /// shows `caller` now: read again by reference, as the caller may read memory, so a memory
    /// forgotten since, or one not theirs to read, is named alone. None without memory, and
    /// none for a result that cannot be read again: its stored words stand. Only a result that
    /// may hold a reference is decoded, and memory is read only when one does.
    pub(crate) async fn results(
        &self,
        caller: &Caller,
        scope: Scope,
        thread: ThreadId,
        events: &[RawEvent],
    ) -> BTreeMap<CallId, String> {
        let Some(memory) = self.inner.harness.memory() else {
            return BTreeMap::new();
        };
        let named = format!("\"{SHOWN}\"");
        let (calls, shown): (Vec<CallId>, Vec<serde_json::Value>) = (events.iter())
            .filter(|raw| {
                raw.kind.as_str() == "tool.call.completed" && raw.data.get().contains(&named)
            })
            .filter_map(|raw| match decode(raw.clone()).ok()?.body {
                Body::ToolCallCompleted(done) if !done.is_error => {
                    Some((done.call_id, done.structured?.get(SHOWN)?.clone()))
                }
                _ => None,
            })
            .unzip();
        if calls.is_empty() {
            return BTreeMap::new();
        }
        let on = MemoryContext {
            person: caller.person.or_else(|| self.person()),
            agent: None,
            scope,
            thread,
            seq: None,
            untrusted_read: false,
        };
        let read = memory.reread(&on, &shown).await.unwrap_or_default();
        (calls.into_iter().zip(read))
            .filter_map(|(call, words)| Some((call, words?)))
            .collect()
    }
}

/// The chat `memory` was saved in, as its caller is shown it.
fn saved_in(memory: &Remembered) -> Option<ThreadId> {
    memory.source.map(|source| source.thread)
}

/// What `memory/forget` forgets: one memory, or with `all`, every memory of a scope. Only one
/// memory's thread is deleted with it.
pub(crate) fn forget(
    memory: Option<glave_event::MemoryId>,
    all: bool,
    scope: Option<glave_event::MemoryScope>,
    delete_thread: bool,
) -> Result<Control, ApiError> {
    match (memory, all, scope, delete_thread) {
        (Some(memory), false, None, _) => Ok(Control::Forget { memory }),
        (None, true, Some(scope), false) => Ok(Control::ForgetAll { scope }),
        _ => Err(ApiError::new(code::INVALID_PARAMS, Sentence::ForgetWhat)),
    }
}
