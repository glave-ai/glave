//! Steering a running turn (`specs/session.md`, "Steering a turn"): a message sent while a turn
//! or a goal's turns run is queued to that work, and the model reads it at its next call. The
//! choice is made under the threads' lock, the same lock the work's end takes to make the
//! thread idle, so a message lands either in the running work or in a turn of its own after
//! it, never in between.

use glave_event::{EventId, Part, ThreadId};
use glave_harness::{Inbox, NotQueued, Queued, Said};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider};
use tokio::sync::watch;

use crate::choice::Choice;
use crate::held::{Slot, busy};
use crate::protocol::{ApiError, AttachedFile, QueuedMessage, code};
use crate::server::Server;
use crate::words::Sentence;

/// A message the person sends a thread with `turn/start`.
pub(crate) struct Message<'a> {
    pub(crate) prompt: &'a str,
    /// The files the person added to it (`turn/start`'s `files`).
    pub(crate) files: &'a [AttachedFile],
    /// The model and preset it chooses for the turn it starts.
    pub(crate) choice: Choice,
    /// Whether it may be queued to a turn already running: a task's own turn may not.
    pub(crate) queue: bool,
    /// Whether no one answers the turn it starts.
    pub(crate) unattended: bool,
}

/// What sending a message did, for whoever follows the turns they start: it started a turn,
/// followed with `T` (a [`TurnStream`](crate::TurnStream) here, a
/// [`RemoteTurn`](crate::client::RemoteTurn) from a server elsewhere), or it was queued to the
/// turn already running in its thread, which that does not follow.
pub enum Sent<T> {
    Started(T),
    Queued(QueuedMessage),
}

impl<T> Sent<T> {
    /// The same answer, a turn it started followed with what `follow` makes of it.
    pub fn map<U>(self, follow: impl FnOnce(T) -> U) -> Sent<U> {
        match self {
            Self::Started(turn) => Sent::Started(follow(turn)),
            Self::Queued(queued) => Sent::Queued(queued),
        }
    }
}

/// What queueing a message found.
pub(crate) enum Queue {
    Queued(QueuedMessage),
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
    /// messages. A message chooses no model or preset: those are for a turn to start
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
        let said = Said {
            text: prompt,
            files,
        };
        let threads = self.lock();
        // A turn that waits for an answer takes messages too, for when it goes on: kept in
        // memory, as a running turn's are.
        if let Some(Slot::Idle { thread: held, .. }) = threads.get(&thread)
            && held.waits()
        {
            return self.queue_to(held.inbox(), None, caller, said, choice);
        }
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
        self.queue_to(inbox, Some(finished), caller, said, choice)
    }

    /// Queues what `caller` `said` to `inbox`, the inbox of a turn that runs (whose end
    /// `finished` tells) or waits.
    fn queue_to(
        &self,
        inbox: &Inbox,
        finished: Option<&watch::Receiver<bool>>,
        caller: &Caller,
        said: Said<'_>,
        choice: &Choice,
    ) -> Result<Queue, ApiError> {
        if choice != &Choice::default() {
            return Err(ApiError::new(
                code::INVALID_PARAMS,
                Sentence::NoChoiceWhileRunning,
            ));
        }
        let harness = &self.inner.harness;
        let id = EventId::new(harness.clock().now(), harness.entropy().bytes()?)?;
        let queued = Queued {
            id,
            text: said.text.to_owned(),
            files: said.files.to_vec(),
            caller: caller.clone(),
        };
        match (inbox.push(queued), finished) {
            (Ok(()), _) => Ok(Queue::Queued(QueuedMessage { id })),
            (Err(NotQueued::Ending), Some(finished)) => Ok(Queue::Ending(finished.clone())),
            (Err(NotQueued::Ending), None) => Ok(Queue::Idle),
            (Err(full @ NotQueued::Full), _) => {
                Err(ApiError::new(code::QUEUE_FULL, full.to_string()))
            }
        }
    }
}
