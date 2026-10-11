//! Starting a turn and following it as it runs: its notifications in order, then how it ended.
//! A turn started with an idempotency key can be followed again by a retry with the same key.

use std::future::{pending, poll_fn};
use std::pin::pin;
use std::sync::Arc;
use std::task::Poll;

use glave_event::ThreadId;
use glave_harness::{Caller, DEFAULT_BUDGET, Goal, Live};
use glave_ports::{BlobStore, EventLog, ModelProvider};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, watch};

use crate::held::finished;
use crate::hub::Hub;
use crate::keyed::Done;
use crate::protocol::{ApiError, IdempotencyKey, Notification, Request, TurnResult, Update, code};
use crate::server::{Server, api};

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
    /// `None` when the turn had already ended before anyone followed it.
    notes: Option<broadcast::Receiver<Arc<Notification>>>,
    done: Done,
    ended: bool,
}

impl TurnStream {
    pub(crate) fn new(notes: Option<broadcast::Receiver<Arc<Notification>>>, done: Done) -> Self {
        Self {
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
            Err(error) => Err(api(error)),
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
    /// Starts `request` and streams it when its method streams (`turn/start`, `goal/start`);
    /// hands any other request back, to answer with [`Server::request_as`].
    pub async fn start_streaming(
        &self,
        caller: &Caller,
        request: Request,
    ) -> Result<Result<TurnStream, ApiError>, Box<Request>> {
        match request {
            Request::TurnStart {
                thread,
                prompt,
                meta,
                background: false,
                budget,
                model,
            } => {
                if budget.is_some() {
                    return Ok(Err(crate::tasks::requests::unenforced_budget()));
                }
                let key = meta.idempotency_key.as_ref();
                Ok(self
                    .start_turn_with(caller, thread, &prompt, key, model)
                    .await)
            }
            Request::GoalStart {
                thread,
                objective,
                budget,
                background: false,
            } => {
                let goal = objective.map(|objective| Goal {
                    objective,
                    token_budget: budget.unwrap_or(DEFAULT_BUDGET),
                });
                Ok(self.start_goal_as(caller, thread, goal).await)
            }
            other => Err(Box::new(other)),
        }
    }

    /// Starts a turn for the local person and streams it: its notifications, then how it ended.
    pub async fn start_turn(&self, thread: ThreadId, prompt: &str) -> Result<TurnStream, ApiError> {
        self.start_turn_as(&Caller::default(), thread, prompt, None)
            .await
    }

    /// Starts a turn for `caller` and streams it. With `key`, a turn already started in
    /// `thread` with the same key is followed instead of starting another.
    pub async fn start_turn_as(
        &self,
        caller: &Caller,
        thread: ThreadId,
        prompt: &str,
        key: Option<&IdempotencyKey>,
    ) -> Result<TurnStream, ApiError> {
        self.start_turn_with(caller, thread, prompt, key, None)
            .await
    }

    /// Like [`Server::start_turn_as`], first choosing `model` for the thread, when given:
    /// this turn and the thread's later ones go to it. The model the turn would go to must be
    /// one this server runs (`-32021`).
    pub(crate) async fn start_turn_with(
        &self,
        caller: &Caller,
        thread: ThreadId,
        prompt: &str,
        key: Option<&IdempotencyKey>,
        model: Option<String>,
    ) -> Result<TurnStream, ApiError> {
        self.authorize(caller, thread).await?;
        self.model_of(thread, model.as_deref()).await?;
        let (finish, done) = watch::channel(None);
        if let Some(key) = key
            && let Some(started) = self.inner.keyed.claim(thread, key, &done)
        {
            let notes = started.borrow().is_none().then(|| self.watch(thread));
            return Ok(TurnStream::new(notes, started));
        }
        let notes = self.watch(thread);
        let prompt = prompt.to_owned();
        let caller = caller.clone();
        let started = self
            .occupy(thread, finish, move |inner, mut held, cancel| async move {
                let hub = Arc::clone(&inner.hub);
                let mut publish = |live| publish(&hub, thread, live);
                if let Some(model) = &model
                    && let Err(error) = inner.harness.choose_model(&mut held, &caller, model).await
                {
                    return (held, Err(error));
                }
                let ran = inner
                    .harness
                    .run_turn_as(&mut held, &prompt, &caller, &cancel, &mut publish)
                    .await;
                (held, ran)
            })
            .await;
        if let Err(failed) = started {
            if let Some(key) = key {
                self.inner.keyed.release(thread, key, &done);
            }
            return Err(failed);
        }
        Ok(TurnStream::new(Some(notes), done))
    }
}
