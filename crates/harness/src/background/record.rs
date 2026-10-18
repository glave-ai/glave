//! The record itself: every command the thread kept in the background, numbered in the order
//! they started, and the changes still to be recorded.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use glave_event::{At, CallId, CommandEnded, CommandEnding, CommandStarted};
use glave_ports::computer::{CommandId, Computers, Ending, WorkspaceId};

/// A thread's commands in the background. Clones of a thread share it.
#[derive(Default)]
pub struct Commands {
    inner: Mutex<Inner>,
}

impl std::fmt::Debug for Commands {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Commands").finish_non_exhaustive()
    }
}

#[derive(Default)]
struct Inner {
    all: Vec<Entry>,
    /// The tool call running now, which a command it starts is recorded under.
    call: Option<CallId>,
    unrecorded: Vec<Change>,
}

/// One command the thread kept in the background.
#[derive(Clone)]
pub(crate) struct Entry {
    /// How the model names it: 1 for the thread's first.
    pub(crate) number: u32,
    pub(crate) id: CommandId,
    pub(crate) workspace: WorkspaceId,
    pub(crate) computers: Arc<dyn Computers>,
    pub(crate) script: String,
    /// When it was recorded as started; unknown until then.
    pub(crate) started: Option<At>,
    /// How it ended, once glave knows.
    pub(crate) ended: Option<CommandEnding>,
}

/// A command a thread keeps running in the background, as a client lists it: what it runs,
/// since when, and a sentence saying so (`command/list`).
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct RunningCommand {
    /// Which command, for `command/stop`.
    pub id: CommandId,
    /// What it runs, as the agent wrote it.
    pub command: String,
    /// When it started, once the thread has recorded it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<At>,
    /// What it is doing, in plain words.
    pub summary: String,
}

/// The sentence a command running in the background is listed with.
pub(crate) const RUNNING: &str =
    "Still running: the agent started it, and it runs until it ends or someone stops it.";

/// A change waiting to be recorded.
pub(crate) enum Change {
    Started(u32, CommandStarted),
    Ended(CommandEnded),
}

impl Commands {
    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Names `call` as the tool call running now: what it starts is recorded under it.
    pub(crate) fn calling(&self, call: CallId) {
        self.inner().call = Some(call);
    }

    /// Keeps `id`, started (or `moved` there at its timeout) by the call running now, and
    /// answers its number.
    pub(crate) fn add(
        &self,
        id: CommandId,
        workspace: WorkspaceId,
        computers: Arc<dyn Computers>,
        script: String,
        moved: bool,
    ) -> u32 {
        let mut inner = self.inner();
        let number = next_number(&inner.all);
        if let Some(call_id) = inner.call.clone() {
            let started = CommandStarted {
                command: id.as_uuid(),
                workspace: workspace.as_uuid(),
                call_id,
                script: script.clone(),
                moved,
            };
            inner.unrecorded.push(Change::Started(number, started));
        }
        inner.all.push(Entry {
            number,
            id,
            workspace,
            computers,
            script,
            started: None,
            ended: None,
        });
        number
    }

    /// Keeps a command the log says was started, with nothing to record.
    pub(super) fn restore(
        &self,
        id: CommandId,
        workspace: WorkspaceId,
        computers: Arc<dyn Computers>,
        script: String,
        started: At,
    ) {
        let mut inner = self.inner();
        let number = next_number(&inner.all);
        inner.all.push(Entry {
            number,
            id,
            workspace,
            computers,
            script,
            started: Some(started),
            ended: None,
        });
    }

    /// Notes how a command the log says ended did, with nothing to record.
    pub(super) fn restore_end(&self, id: CommandId, ending: &CommandEnding) {
        let mut inner = self.inner();
        if let Some(entry) = inner.all.iter_mut().find(|entry| entry.id == id) {
            entry.ended = Some(ending.clone());
        }
    }

    /// Notes when the command numbered `number` was recorded as started.
    pub(crate) fn started_at(&self, number: u32, at: At) {
        let mut inner = self.inner();
        if let Some(entry) = inner.all.iter_mut().find(|entry| entry.number == number) {
            entry.started = Some(at);
        }
    }

    /// Notes that `id` ended so, after `millis` when known, unless it is known to have ended.
    pub(crate) fn end(&self, id: CommandId, ending: CommandEnding, millis: Option<u64>) {
        let mut inner = self.inner();
        let Some(entry) = inner
            .all
            .iter_mut()
            .find(|entry| entry.id == id && entry.ended.is_none())
        else {
            return;
        };
        entry.ended = Some(ending.clone());
        inner.unrecorded.push(Change::Ended(CommandEnded {
            command: id.as_uuid(),
            ending,
            millis,
        }));
    }

    /// The command the model calls `number`.
    pub(crate) fn numbered(&self, number: u32) -> Option<Entry> {
        let inner = self.inner();
        inner
            .all
            .iter()
            .find(|entry| entry.number == number)
            .cloned()
    }

    /// The commands not known to have ended, oldest first.
    pub(crate) fn running(&self) -> Vec<Entry> {
        let inner = self.inner();
        inner
            .all
            .iter()
            .filter(|entry| entry.ended.is_none())
            .cloned()
            .collect()
    }

    pub(crate) fn any_running(&self) -> bool {
        self.inner().all.iter().any(|entry| entry.ended.is_none())
    }

    /// The changes waiting to be recorded, taken in order.
    pub(crate) fn unrecorded(&self) -> Vec<Change> {
        std::mem::take(&mut self.inner().unrecorded)
    }
}

/// The number after the last one given.
fn next_number(all: &[Entry]) -> u32 {
    all.last().map_or(1, |entry| entry.number.saturating_add(1))
}

/// How the thread records a command's ending as its computer gave it.
pub(crate) fn ending_of(ending: Ending) -> CommandEnding {
    match ending {
        Ending::Exited { code } => CommandEnding::Exited { code },
        Ending::Signalled => CommandEnding::Signalled,
        Ending::Idle { .. } => CommandEnding::Idle,
        Ending::TimedOut { .. } => CommandEnding::TimedOut,
        // Stopped, though not by this thread: someone or something else asked its computer.
        Ending::Cancelled => CommandEnding::Stopped {
            by: Some(glave_event::Principal::glave()),
        },
        // Only a run moves to the background; a command in the background never answers it.
        Ending::Backgrounded { .. } => CommandEnding::Gone,
    }
}
