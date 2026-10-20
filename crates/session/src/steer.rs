//! Steering a running turn (`specs/session.md`, "Steering a turn"): a message sent while a turn
//! or a goal's turns run is queued to that work, and the model reads it at its next call. The
//! choice is made under the threads' lock, the same lock the work's end takes to make the
//! thread idle, so a message lands either in the running work or in a turn of its own after
//! it, never in between.

use glave_event::{EventId, Part, ThreadId};
use glave_harness::{NotQueued, Queued};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider};
use tokio::sync::watch;

use crate::held::{Slot, busy};
use crate::modes::Choice;
use crate::protocol::{ApiError, AttachedFile, code};
use crate::server::Server;
use crate::turn::TurnStream;

/// A message the person sends a thread with `turn/start`.
pub(crate) struct Message<'a> {
    pub(crate) prompt: &'a str,
    /// The files the person added to it (`turn/start`'s `files`).
    pub(crate) files: &'a [AttachedFile],
    /// The model and approval mode it chooses for the turn it starts.
    pub(crate) choice: Choice,
    /// Whether it may be queued to a turn already running: a task's own turn may not.
    pub(crate) queue: bool,
    /// Whether no one answers the turn it starts.
    pub(crate) unattended: bool,
}

/// What sending a message did.
pub(crate) enum Sent {
    /// It started a turn, followed here.
    Started(TurnStream),
    /// It was queued to the turn running in its thread, under this id (`QueuedMessage`).
    Queued(EventId),
}

/// What queueing a message found.
pub(crate) enum Queue {
    Queued(EventId),
    /// Nothing runs in the thread: the message starts a turn.
    Idle,
    /// The running work was stopped, or has made its last call and is ending: the message
    /// starts a turn once it has ended (`finished`).
    Ending(watch::Receiver<bool>),
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Queues `caller`'s `prompt` to the work running in `thread`, when that work takes
    /// messages. A message chooses no model or approval mode: those are for a turn to start
    /// with. Refused when as many messages wait as a turn takes (`-32028`), or while work that
    /// takes none runs (`-32002`).
    pub(crate) fn queue(
        &self,
        caller: &Caller,
        thread: ThreadId,
        prompt: &str,
        files: &[Part],
        choice: &Choice,
    ) -> Result<Queue, ApiError> {
        let threads = self.lock();
        let Some(Slot::Busy {
            cancel,
            finished,
            inbox,
            ..
        }) = threads.get(&thread)
        else {
            return Ok(Queue::Idle);
        };
        let Some(inbox) = inbox else {
            return Err(busy(thread));
        };
        // A stopped turn reads nothing more: what is sent after Stop starts the next turn.
        if cancel.is_cancelled() {
            return Ok(Queue::Ending(finished.clone()));
        }
        if choice != &Choice::default() {
            return Err(ApiError::new(
                code::INVALID_PARAMS,
                "a model, reasoning level or approval mode is chosen for a turn to start with: \
                 while a turn runs, send the message without one, or choose once the turn has \
                 ended",
            ));
        }
        let harness = &self.inner.harness;
        let id = EventId::new(harness.clock().now(), harness.entropy().bytes()?)?;
        let queued = Queued {
            id,
            text: prompt.to_owned(),
            files: files.to_vec(),
            caller: caller.clone(),
        };
        match inbox.push(queued) {
            Ok(()) => Ok(Queue::Queued(id)),
            Err(NotQueued::Ending) => Ok(Queue::Ending(finished.clone())),
            Err(full @ NotQueued::Full) => Err(ApiError::new(code::QUEUE_FULL, full.to_string())),
        }
    }
}
