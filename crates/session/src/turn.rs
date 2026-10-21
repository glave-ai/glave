//! Starting a turn and following it as it runs: its notifications in order, then how it ended.
//! A turn started with an idempotency key can be followed again by a retry with the same key.
//! While one runs, a message is queued to it instead (`steer.rs`).

use std::future::pending;
use std::sync::Arc;

use glave_event::ThreadId;
use glave_harness::{Cancel, HarnessError, Live, Said, Thread, TurnSummary};
use glave_ports::{BlobStore, BoxFuture, Caller, EventLog, ModelProvider};
use tokio::sync::watch;

use crate::held::{Finish, Use, busy, finished};
use crate::hub::Hub;
use crate::keyed::Done;
use crate::modes::Choice;
use crate::protocol::{ApiError, IdempotencyKey, Notification, TurnResult, Update, code};
use crate::server::{Ended, Inner, Server};
use crate::steer::{Message, Queue, Sent};
use crate::watch::{Watch, Watched};
use crate::words::Sentence;

/// One thing a running turn sends.
#[derive(Debug, Clone)]
pub enum TurnItem {
    Note(Arc<Notification>),
    /// A client of a server elsewhere fell behind what the server sent it, and missed this many
    /// notifications its own buffer could not hold. The stored events are still in the log:
    /// read them with `thread/events` after the last `seq` seen. A turn followed in the
    /// server's own process never misses one: what its watch misses, it reads from the log.
    Missed(u64),
    /// How the turn ended. Always the last item.
    Ended(Result<TurnResult, ApiError>),
}

/// A turn's notifications, then its ending: a watch of its thread from where the turn began
/// (what it records, its live updates and its subagents'), which ends with how the turn ended.
pub struct TurnStream {
    /// The model the turn, or the goal's first turn, goes to.
    pub(crate) model: String,
    thread: ThreadId,
    /// `None` when the turn had already ended before anyone followed it.
    watch: Option<Box<Watch>>,
    done: Done,
    /// How the turn ended, once it has, while what it sent before is still to come.
    ending: Option<Result<TurnResult, ApiError>>,
    ended: bool,
}

/// What came first while a turn ran: something its watch sent, or how it ended.
enum Came {
    Watched(Option<Watched>),
    Ending(Result<TurnResult, ApiError>),
}

impl TurnStream {
    pub(crate) fn new(model: String, thread: ThreadId, watch: Option<Watch>, done: Done) -> Self {
        Self {
            model,
            thread,
            watch: watch.map(Box::new),
            done,
            ending: None,
            ended: false,
        }
    }

    /// The thread the turn runs in.
    pub fn thread(&self) -> ThreadId {
        self.thread
    }

    /// The next item, or `None` after the ending.
    pub async fn next(&mut self) -> Option<TurnItem> {
        if self.ended {
            return None;
        }
        while self.ending.is_none() {
            let Self { watch, done, .. } = self;
            let watched = async {
                match watch {
                    Some(watch) => watch.next().await,
                    None => pending().await,
                }
            };
            // What the watch has is taken before the ending.
            let came = tokio::select! {
                biased;
                item = watched => Came::Watched(item),
                ended = ending(done) => Came::Ending(ended),
            };
            match came {
                Came::Watched(Some(Watched::Note(note))) => return Some(TurnItem::Note(note)),
                // The thread was deleted, or its log could not be read: the ending still comes.
                Came::Watched(_) => self.watch = None,
                Came::Ending(ended) => self.ending = Some(ended),
            }
        }
        // Everything the turn sent before it ended comes before how it ended.
        if let Some(watch) = &mut self.watch {
            match watch.rest().await {
                Some(Watched::Note(note)) => return Some(TurnItem::Note(note)),
                Some(Watched::Ended(_)) | None => self.watch = None,
            }
        }
        self.ended = true;
        self.ending.take().map(TurnItem::Ended)
    }

    /// Follows the turn to its end, ignoring its notifications.
    pub async fn finish(&mut self) -> Result<TurnResult, ApiError> {
        while let Some(item) = self.next().await {
            if let TurnItem::Ended(ended) = item {
                return ended;
            }
        }
        Err(ApiError::new(code::INTERNAL, Sentence::TurnEnded))
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

/// How the turn ended, as the API reports it: a turn that stopped at a wait has not ended, and
/// goes on, under work of its own, once answered (`answers.rs`).
async fn ending(done: &mut Done) -> Result<TurnResult, ApiError> {
    loop {
        let Some(ended) = finished(done).await else {
            return Err(ApiError::new(code::INTERNAL, Sentence::TurnStopped));
        };
        match ended.as_ref() {
            Ok(summary) => return Ok(TurnResult::from(summary.clone())),
            Err(HarnessError::Waits(_)) => {
                if done.changed().await.is_err() {
                    return Err(ApiError::new(code::NOT_AVAILABLE, Sentence::TurnUnfollowed));
                }
            }
            Err(error) => return Err(ApiError::from(error)),
        }
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
            files,
            choice,
            queue,
            unattended,
        } = message;
        let scope = self.authorize(caller, thread).await?;
        let model = self.model_of(thread, choice.model.as_deref()).await?;
        if let Some(effort) = &choice.effort {
            self.offered_effort(&model, effort)?;
        }
        self.check_approval(thread, choice.approval).await?;
        let files = self.attach(&scope, thread, files, &model).await?;
        let (finish, done) = watch::channel(None);
        if let Some(key) = key
            && let Some(started) = self.inner.keyed.claim(caller.person, thread, key, &done)
        {
            let running = started.borrow().is_none();
            let watch = if running {
                Some(self.watch_from_now(scope, thread).await?)
            } else {
                None
            };
            return Ok(Sent::Started(TurnStream::new(
                model, thread, watch, started,
            )));
        }
        if queue {
            // Read in, so a turn that waits for an answer takes the message for when it goes on.
            self.load(thread).await?;
        }
        let sent = loop {
            if queue {
                match self.queue(caller, thread, prompt, &files, &choice) {
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
            let watch = match self.watch_from_now(scope, thread).await {
                Ok(watch) => watch,
                Err(failed) => break Err(failed),
            };
            let said = Said {
                text: prompt,
                files: &files,
            };
            let work = turn(thread, said, caller, &choice, unattended, finish.clone());
            let started = self
                .occupy_for(thread, finish.clone(), Use::Steered, work)
                .await;
            match started {
                Ok(()) => {
                    let stream = TurnStream::new(model, thread, Some(watch), done);
                    break Ok(Sent::Started(stream));
                }
                // Started by someone else meanwhile: queued to it instead.
                Err(running)
                    if queue && running.code == code::TURN_RUNNING && self.is_busy(thread) => {}
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
/// then the turn on what was `said`, then a turn for each message sent too late for the one
/// before it; answered by no one throughout when `unattended`. A turn that stops at a wait
/// keeps `finish`, where it sends how it ends, for the work that goes on with it.
fn turn<P, L, B>(
    thread: ThreadId,
    said: Said<'_>,
    caller: &Caller,
    choice: &Choice,
    unattended: bool,
    finish: Finish<TurnSummary>,
) -> TurnWork<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    let (prompt, files) = (said.text.to_owned(), said.files.to_vec());
    let (caller, choice) = (caller.clone(), choice.clone());
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
                .run_turn_with_files(
                    &mut held,
                    Said {
                        text: &prompt,
                        files: &files,
                    },
                    &caller,
                    &cancel,
                    &mut publish,
                )
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
            if matches!(ran, Err(HarnessError::Waits(_))) {
                inner.followed.keep(thread, finish);
            }
            (held, ran)
        })
    })
}
