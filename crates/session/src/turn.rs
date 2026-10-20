//! Starting a turn and following it as it runs: its notifications in order, then how it ended.
//! A turn started with an idempotency key can be followed again by a retry with the same key.
//! While one runs, a message is queued to it instead (`steer.rs`).

use std::future::{pending, poll_fn};
use std::pin::pin;
use std::sync::Arc;
use std::task::Poll;

use glave_event::ThreadId;
use glave_harness::{Cancel, Live, Thread};
use glave_ports::{BlobStore, BoxFuture, Caller, EventLog, ModelProvider};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, watch};

use crate::held::{Use, busy, finished};
use crate::hub::Hub;
use crate::keyed::Done;
use crate::modes::Choice;
use crate::protocol::{ApiError, IdempotencyKey, Notification, TurnResult, Update, code};
use crate::server::{Ended, Inner, Server};
use crate::steer::{Message, Queue, Sent};

/// One thing a running turn sends.
#[derive(Debug, Clone)]
pub enum TurnItem {
    Note(Arc<Notification>),
    /// The follower fell behind and missed this many notifications. The stored events are
    /// still in the log: read them with `thread/events` after the last `seq` seen.
    Missed(u64),
    /// How the turn ended. Always the last item.
    Ended(Result<TurnResult, ApiError>),
}

/// A turn's notifications, then its ending.
pub struct TurnStream {
    /// The model the turn, or the goal's first turn, goes to.
    pub(crate) model: String,
    /// `None` when the turn had already ended before anyone followed it.
    notes: Option<broadcast::Receiver<Arc<Notification>>>,
    done: Done,
    ended: bool,
}

impl TurnStream {
    pub(crate) fn new(
        model: String,
        notes: Option<broadcast::Receiver<Arc<Notification>>>,
        done: Done,
    ) -> Self {
        Self {
            model,
            notes,
            done,
            ended: false,
        }
    }

    /// The next item, or `None` after the ending.
    pub async fn next(&mut self) -> Option<TurnItem> {
        if self.ended {
            return None;
        }
        let Self { notes, done, .. } = self;
        let note = async {
            match notes {
                Some(notes) => notes.recv().await,
                None => pending().await,
            }
        };
        let mut note = pin!(note);
        let mut ending = pin!(ending(done));
        let item = poll_fn(|cx| {
            // Everything sent before the turn ended is in the channel by then, so notes are
            // always taken first and the ending comes last.
            match note.as_mut().poll(cx) {
                Poll::Ready(Ok(note)) => return Poll::Ready(TurnItem::Note(note)),
                Poll::Ready(Err(RecvError::Lagged(missed))) => {
                    return Poll::Ready(TurnItem::Missed(missed));
                }
                Poll::Ready(Err(RecvError::Closed)) | Poll::Pending => {}
            }
            ending.as_mut().poll(cx).map(TurnItem::Ended)
        })
        .await;
        self.ended = matches!(item, TurnItem::Ended(_));
        Some(item)
    }

    /// Follows the turn to its end, ignoring its notifications.
    pub async fn finish(&mut self) -> Result<TurnResult, ApiError> {
        while let Some(item) = self.next().await {
            if let TurnItem::Ended(ended) = item {
                return ended;
            }
        }
        Err(ApiError::new(code::INTERNAL, "the turn already ended"))
    }
}

/// Sends progress in `thread` to whoever follows it. A subagent's progress goes under its own
/// thread, and so to its watchers and the parent's.
pub(crate) fn publish(hub: &Hub, thread: ThreadId, live: Live) {
    let (from, live) = match live {
        Live::Child { thread, update } => (thread, *update),
        live => (thread, live),
    };
    let update = Notification::Live {
        thread: from,
        update: Update::from(live),
    };
    hub.publish(from, update);
}

/// How the turn ended, as the API reports it.
async fn ending(done: &mut Done) -> Result<TurnResult, ApiError> {
    match finished(done).await {
        Some(ended) => match ended.as_ref() {
            Ok(summary) => Ok(TurnResult::from(summary.clone())),
            Err(error) => Err(ApiError::from(error)),
        },
        None => Err(ApiError::new(code::INTERNAL, "the turn stopped")),
    }
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Sends `caller`'s `message` to `thread`: it starts a turn, streamed, first choosing for
    /// the thread the model and approval mode the message names, when it does: this turn and
    /// the thread's later ones go to it and ask that much. While a turn runs there, a message
    /// that may be queued is queued to it instead, for the model's next call. With `key`, a
    /// turn `caller` already started in `thread` with the same key is followed instead of starting
    /// another. The model the turn would go to must be one this server runs (`-32021`), and its
    /// mode one the policy leaves (`-32012`).
    pub(crate) async fn start_turn_with(
        &self,
        caller: &Caller,
        thread: ThreadId,
        message: Message<'_>,
        key: Option<&IdempotencyKey>,
    ) -> Result<Sent, ApiError> {
        let Message {
            prompt,
            choice,
            queue,
            unattended,
        } = message;
        self.authorize(caller, thread).await?;
        let model = self.model_of(thread, choice.model.as_deref()).await?;
        if let Some(effort) = &choice.effort {
            self.offered_effort(&model, effort)?;
        }
        self.check_approval(thread, choice.approval).await?;
        let (finish, done) = watch::channel(None);
        if let Some(key) = key
            && let Some(started) = self.inner.keyed.claim(caller.person, thread, key, &done)
        {
            let notes = started.borrow().is_none().then(|| self.watch(thread));
            return Ok(Sent::Started(TurnStream::new(model, notes, started)));
        }
        let sent = loop {
            if queue {
                match self.queue(caller, thread, prompt, &choice) {
                    Ok(Queue::Queued(id)) => break Ok(Sent::Queued(id)),
                    // Started again once it has ended; never, if its work went without ending.
                    Ok(Queue::Ending(mut ending)) => match ending.wait_for(|ended| *ended).await {
                        Ok(_) => continue,
                        Err(_) => break Err(busy(thread)),
                    },
                    Ok(Queue::Idle) => {}
                    Err(refused) => break Err(refused),
                }
            }
            let notes = self.watch(thread);
            let work = turn(thread, prompt, caller, &choice, unattended);
            let started = self
                .occupy_for(thread, finish.clone(), Use::Steered, work)
                .await;
            match started {
                Ok(()) => break Ok(Sent::Started(TurnStream::new(model, Some(notes), done))),
                // Started by someone else meanwhile: queued to it instead.
                Err(running) if queue && running.code == code::TURN_RUNNING => {}
                Err(failed) => break Err(failed),
            }
        };
        // The key names no turn when none started.
        if !matches!(sent, Ok(Sent::Started(_)))
            && let Some(key) = key
        {
            self.inner
                .keyed
                .release(caller.person, thread, key, &finish.subscribe());
        }
        sent
    }
}

/// Work on a held thread that ends as a turn does.
type TurnWork<P, L, B> = Box<
    dyn FnOnce(Arc<Inner<P, L, B>>, Thread, Cancel) -> BoxFuture<'static, (Thread, Ended)> + Send,
>;

/// A turn's work on a held thread: the model and approval mode `choice` names chosen first,
/// then the turn, then a turn for each message sent too late for the one before it; answered
/// by no one throughout when `unattended`.
fn turn<P, L, B>(
    thread: ThreadId,
    prompt: &str,
    caller: &Caller,
    choice: &Choice,
    unattended: bool,
) -> TurnWork<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    let (prompt, caller, choice) = (prompt.to_owned(), caller.clone(), choice.clone());
    Box::new(move |inner, mut held, cancel| {
        Box::pin(async move {
            let _no_one = unattended.then(|| inner.approvals.unattended_while(thread));
            let hub = Arc::clone(&inner.hub);
            let mut publish = |live| publish(&hub, thread, live);
            let chose = crate::modes::record(&inner.harness, &mut held, &caller, &choice);
            if let Err(error) = chose.await {
                return (held, Err(error));
            }
            let harness = &inner.harness;
            let ran = harness
                .run_turn_as(&mut held, &prompt, &caller, &cancel, &mut publish)
                .await;
            // After the first exchange, a model titles the thread without holding it.
            if ran.is_ok()
                && let Some(case) = harness.title_case(&held)
            {
                let inner = Arc::clone(&inner);
                Server { inner }.title_later(thread, case);
            }
            let ran = harness
                .run_queued(&mut held, ran, &cancel, &mut publish)
                .await;
            (held, ran)
        })
    })
}
