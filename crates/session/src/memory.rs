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
    /// Chats when it names none) and the caller's own.
    pub(crate) async fn remember(
        &self,
        caller: &Caller,
        project: Option<ProjectId>,
        control: Control,
    ) -> Result<Reply, ApiError> {
        Ok(Reply::Memories {
            memories: self.controlled(caller, project, control).await?,
            thread_kept: None,
        })
    }

    /// The memories `control` lists or changed, each as `caller` is shown it: its `source`
    /// names only a chat started for them that is still there, so no one learns of another's.
    async fn controlled(
        &self,
        caller: &Caller,
        project: Option<ProjectId>,
        control: Control,
    ) -> Result<Vec<Remembered>, ApiError> {
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
        let mut memories = memory.control(&access, control).await?;
        let mut theirs = BTreeMap::new();
        for memory in &mut memories {
            let Some(thread) = memory.source.map(|source| source.thread) else {
                continue;
            };
            let shown = if let Some(known) = theirs.get(&thread).copied() {
                known
            } else {
                let known = self.theirs(caller, thread).await;
                theirs.insert(thread, known);
                known
            };
            if !shown {
                memory.source = None;
            }
        }
        Ok(memories)
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
        let whose = view.whose().or_else(|| self.person());
        view.standing != Standing::Deleted && whose == Some(person)
    }

    /// Answers `memory/forget` for `caller` as [`Server::remember`] does, then, with
    /// `delete_thread`, deletes the chat the memory was saved in as `thread/delete` does, when
    /// it is the caller's own. When no chat goes, the memory is still forgotten, and the answer
    /// says why in a sentence.
    pub(crate) async fn forget(
        &self,
        caller: &Caller,
        project: Option<ProjectId>,
        control: Control,
        delete_thread: bool,
    ) -> Result<Reply, ApiError> {
        let memories = self.controlled(caller, project, control).await?;
        let saved_in = memories.iter().filter_map(|memory| memory.source);
        let mut kept = delete_thread.then(|| Told::from(Sentence::ThreadKept));
        for thread in saved_in
            .filter(|_| delete_thread)
            .map(|source| source.thread)
        {
            kept = match self.change(caller, thread, Change::Delete).await {
                Ok(_) => None,
                Err(gone) if gone.code == code::NO_THREAD => kept,
                // Refused, such as while a turn runs there: the memory is forgotten all the same.
                Err(refused) => Some(refused.message),
            };
        }
        Ok(Reply::Memories {
            memories,
            thread_kept: kept,
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

/// What `memory/forget` forgets: one memory, or with `all`, every memory of a scope. Only one
/// memory's thread is deleted with it.
pub(crate) fn forget_control(
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
