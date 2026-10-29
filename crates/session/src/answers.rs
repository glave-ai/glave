//! Answers to what a turn waits for (`specs/one-server.md`, "An answer continues the turn"): a
//! turn that waits holds nothing in memory, so `approval/decide` and `question/answer` check the
//! log, record the answer there exactly as the turn would, and the turn goes on from the log
//! (`Harness::go_on`), in this server, under the lease of the thread whose turn it is: the
//! waiting thread's own, or its parent's for a subagent's wait. Stopping a waiting turn goes on
//! with it stopped, which declines what it waits for. The client that started the turn keeps
//! following it: how it ends goes to the turn's stream, kept while the turn waits.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use glave_event::{CallId, ParentCall, ThreadId, TurnResult};
use glave_harness::{Decision, HarnessError};
use glave_ports::{
    BlobStore, BoxFuture, CallKind, Caller, EventLog, ModelProvider, Scope, Standing, ThreadQuery,
    ThreadView, WaitKind,
};
use tokio::sync::{oneshot, watch};

use crate::held::{Finish, Slot, Use, finished};
use crate::protocol::{Answer, ApiError, Reply, Status, code};
use crate::server::{Inner, Server};
use crate::turn::publish;
use crate::words::Sentence;

/// Waiting turns whose ending someone follows, kept at once; past this, a new one's follower
/// learns how it ends from the thread's events alone.
const FOLLOWED: usize = 4_096;

/// Where each waiting turn sends how it ends: to whoever started or retried it, kept while the
/// turn waits, since the work that ran it has ended.
#[derive(Default)]
pub(crate) struct Followed(Mutex<HashMap<ThreadId, Finish<TurnResult>>>);

impl Followed {
    /// Keeps `finish`, where `thread`'s turn sends how it ends, while the turn waits.
    pub(crate) fn keep(&self, thread: ThreadId, finish: Finish<TurnResult>) {
        let mut kept = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        kept.retain(|_, finish| finish.receiver_count() > 0);
        if kept.len() < FOLLOWED || kept.contains_key(&thread) {
            kept.insert(thread, finish);
        }
    }

    /// Where `thread`'s turn sends how it ends, now that it goes on: the one kept for whoever
    /// follows it, else a new one.
    fn take(&self, thread: ThreadId) -> Finish<TurnResult> {
        let kept = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&thread);
        kept.unwrap_or_else(|| watch::channel(None).0)
    }
}

/// Records an answer in the log, before the turn goes on.
type Record<P, L, B> =
    Box<dyn FnOnce(Arc<Inner<P, L, B>>) -> BoxFuture<'static, Result<(), HarnessError>> + Send>;

/// Where the work that records an answer says whether it was.
type Told = oneshot::Sender<Result<(), HarnessError>>;

/// Work that goes on with a waiting turn.
type GoingOn<P, L, B> = Box<
    dyn FnOnce(
            Arc<Inner<P, L, B>>,
            glave_harness::Thread,
            glave_harness::Cancel,
        )
            -> BoxFuture<'static, (glave_harness::Thread, Result<TurnResult, HarnessError>)>
        + Send,
>;

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Answers `approval/decide` for `caller`: the decision is recorded for the approval
    /// `call` in `thread` waits for, and the turn goes on.
    pub(crate) async fn decide(
        &self,
        caller: &Caller,
        thread: ThreadId,
        call: CallId,
        answer: Answer,
    ) -> Result<Reply, ApiError> {
        let scope = self.authorize(caller, thread).await?;
        let decision = match answer {
            Answer::AllowOnce => Decision::AllowOnce,
            Answer::AllowForThread => Decision::AllowForThread,
            Answer::Deny => Decision::Deny,
        };
        let person = caller.person;
        let record: Record<P, L, B> = Box::new(move |inner| {
            Box::pin(async move {
                let harness = &inner.harness;
                harness.decide(scope, thread, &call, decision, person).await
            })
        });
        self.answered(caller, scope, thread, record).await
    }

    /// Answers `question/answer` for `caller`: `answers` are recorded as the result of the
    /// questions `call` in `thread` waits for, and the turn goes on.
    pub(crate) async fn answer_questions(
        &self,
        caller: &Caller,
        thread: ThreadId,
        call: CallId,
        answers: Vec<String>,
    ) -> Result<Reply, ApiError> {
        let scope = self.authorize(caller, thread).await?;
        let record: Record<P, L, B> = Box::new(move |inner| {
            Box::pin(async move {
                let harness = &inner.harness;
                harness.answer(scope, thread, &call, &answers).await
            })
        });
        self.answered(caller, scope, thread, record).await
    }

    /// Records an answer with `record` for `thread`'s wait, under the lease of the thread whose
    /// turn waits, and goes on with that turn. Where this server's work holds that thread, its
    /// lease is held already: the answer is recorded at once, and the turn goes on once the
    /// work stops.
    async fn answered(
        &self,
        caller: &Caller,
        scope: Scope,
        thread: ThreadId,
        record: Record<P, L, B>,
    ) -> Result<Reply, ApiError> {
        let root = self.root_of(scope, thread).await?;
        // A subagent's wait goes on in its parent's turn, in the same project, which this
        // server may not have looked up: held until the answer is in.
        let _root = self.hold(root, scope);
        if self.is_busy(root) {
            return self.record_now(caller, root, record).await;
        }
        let (told, recorded) = oneshot::channel();
        let finish = self.inner.followed.take(root);
        let work = going_on(
            root,
            Some((record, told)),
            caller.clone(),
            finish.clone(),
            false,
        );
        match self
            .occupy_for(root, finish.clone(), Use::Answer, work)
            .await
        {
            Ok(()) => recorded
                .await
                .map_err(|_| ApiError::new(code::INTERNAL, Sentence::AnswerNotRecorded))?
                .map(|()| Reply::Done {})
                .map_err(|error| ApiError::from(&error)),
            Err(refused) => {
                self.inner.followed.keep(root, finish);
                Err(refused)
            }
        }
    }

    /// Records an answer with `record` while this server's work holds `root`, and goes on
    /// with `root`'s turn once that work stops.
    async fn record_now(
        &self,
        caller: &Caller,
        root: ThreadId,
        record: Record<P, L, B>,
    ) -> Result<Reply, ApiError> {
        record(Arc::clone(&self.inner))
            .await
            .map_err(|error| ApiError::from(&error))?;
        self.go_on_when_free(root, caller.clone());
        Ok(Reply::Done {})
    }

    /// Goes on with `root`'s turn once the work this server runs there stops: the turn stopped
    /// at the wait that was just answered, or saw its answer and went on already.
    fn go_on_when_free(&self, root: ThreadId, caller: Caller) {
        let server = self.clone();
        let task = tokio::spawn(async move {
            loop {
                let running = match server.lock().get(&root) {
                    Some(Slot::Busy { finished, .. }) => Some(finished.clone()),
                    _ => None,
                };
                if let Some(mut running) = running {
                    let _ = running.wait_for(|done| *done).await;
                }
                let finish = server.inner.followed.take(root);
                let work = going_on(root, None, caller.clone(), finish.clone(), false);
                match server
                    .occupy_for(root, finish.clone(), Use::Answer, work)
                    .await
                {
                    Ok(()) => break,
                    Err(refused) => {
                        server.inner.followed.keep(root, finish);
                        if refused.code != code::TURN_RUNNING || !server.is_busy(root) {
                            tracing::warn!(error = %refused.message, "an answered turn could not go on yet");
                            break;
                        }
                    }
                }
            }
        });
        self.own(task);
    }

    /// Stops `thread`'s turn where it waits: it goes on stopped, declining what it waits for,
    /// and ends cancelled. Answers once it has ended.
    pub(crate) async fn stop_waiting(&self, thread: ThreadId) -> Result<Reply, ApiError> {
        let scope = self.scope_of(thread)?;
        let view = self.inner.harness.log().thread_view(&scope, thread).await?;
        let waits = match &view {
            Some(view) => self.view_waits(&scope, view).await?,
            None => false,
        };
        if !waits {
            return Err(ApiError::new(
                code::NO_TURN,
                Sentence::NoTurn {
                    thread: thread.to_string(),
                },
            ));
        }
        let finish = self.inner.followed.take(thread);
        let mut done = finish.subscribe();
        let work = going_on(thread, None, Caller::default(), finish.clone(), true);
        self.occupy_for(thread, finish, Use::Answer, work).await?;
        let _ = finished(&mut done).await;
        Ok(Reply::Done {})
    }

    /// The thread whose turn `thread`'s wait belongs to: its parent's for a subagent's.
    async fn root_of(&self, scope: Scope, thread: ThreadId) -> Result<ThreadId, ApiError> {
        let view = self.inner.harness.log().thread_view(&scope, thread).await?;
        Ok(view.and_then(|view| view.parent).unwrap_or(thread))
    }

    /// Whether work of this server holds `thread` now.
    pub(crate) fn is_busy(&self, thread: ThreadId) -> bool {
        matches!(self.lock().get(&thread), Some(Slot::Busy { .. }))
    }

    /// What `view`'s thread is doing now: waiting for the person, its own wait or a
    /// subagent's, as its view and its subagents' say, whichever server runs it; running, here
    /// or elsewhere; or idle.
    pub(crate) async fn status_of(&self, scope: &Scope, view: &ThreadView) -> Status {
        // A view that cannot be read says nothing of a wait.
        if self.view_waits(scope, view).await.unwrap_or(false) {
            return Status::Waiting;
        }
        if self.is_busy(view.thread) || view.turn.is_some() {
            Status::Running
        } else {
            Status::Idle
        }
    }

    /// Whether the turn open in `view` waits for the person: at its own approval or questions,
    /// or at subagents' work while one of them waits at theirs.
    pub(crate) async fn view_waits(
        &self,
        scope: &Scope,
        view: &ThreadView,
    ) -> Result<bool, ApiError> {
        let Some(turn) = &view.turn else {
            return Ok(false);
        };
        match turn.wait.as_ref().map(|wait| wait.kind) {
            Some(WaitKind::Approval | WaitKind::Question) => Ok(true),
            None => Ok(false),
            Some(WaitKind::Agent) => {
                let agents = turn
                    .unfinished
                    .iter()
                    .take_while(|call| call.kind == CallKind::Agent && call.asked.is_none());
                for call in agents {
                    let child = self.child_view(scope, view.thread, &call.call).await?;
                    let waits = child
                        .and_then(|child| child.turn)
                        .and_then(|turn| turn.wait)
                        .is_some_and(|wait| wait.kind != WaitKind::Agent);
                    if waits {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
        }
    }

    /// The view of the subagent's thread `call` in `parent` started, if any.
    async fn child_view(
        &self,
        scope: &Scope,
        parent: ThreadId,
        call: &CallId,
    ) -> Result<Option<ThreadView>, ApiError> {
        let query = ThreadQuery {
            standing: Standing::Active,
            limit: 1,
            after: None,
            open: false,
            parent: Some(ParentCall {
                thread: parent,
                call_id: call.clone(),
            }),
        };
        let found = self.inner.harness.log().thread_views(scope, &query).await?;
        Ok(found.into_iter().next())
    }
}

/// Work that goes on with `thread`'s waiting turn for `caller`, sending how it ends on
/// `finish`: first recording an answer, when `first` gives one and where to tell whether it
/// was; stopped from the start when `stop` says so. A turn that waits again keeps `finish` for
/// its followers.
fn going_on<P, L, B>(
    thread: ThreadId,
    first: Option<(Record<P, L, B>, Told)>,
    caller: Caller,
    finish: Finish<TurnResult>,
    stop: bool,
) -> GoingOn<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    Box::new(move |inner, mut held, cancel| {
        Box::pin(async move {
            if let Some((record, told)) = first {
                let recorded = record(Arc::clone(&inner)).await;
                let refused = recorded.is_err();
                let _ = told.send(recorded);
                if refused {
                    inner.followed.keep(thread, finish);
                    return (held, Err(HarnessError::Waits(thread)));
                }
            }
            if stop {
                cancel.cancel();
            }
            adopt_children(&inner, &held).await;
            let hub = Arc::clone(&inner.hub);
            let mut publish = |live| publish(&hub, thread, live);
            let harness = &inner.harness;
            let ran = harness
                .go_on(&mut held, &caller, &cancel, &mut publish)
                .await;
            // A first exchange that ended only now gets its title now.
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

/// Lets the subagents of `held`'s open turn, found through the threads view, send what they do
/// to whoever follows `held` too, as a subagent started here does from its start.
async fn adopt_children<P, L, B>(inner: &Inner<P, L, B>, held: &glave_harness::Thread)
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
{
    let log = inner.harness.log();
    let Ok(Some(view)) = log.thread_view(&held.scope, held.id).await else {
        return;
    };
    let Some(turn) = view.turn else {
        return;
    };
    for call in turn
        .unfinished
        .iter()
        .filter(|call| call.kind == CallKind::Agent)
    {
        let query = ThreadQuery {
            standing: Standing::Active,
            limit: 1,
            after: None,
            open: false,
            parent: Some(ParentCall {
                thread: held.id,
                call_id: call.call.clone(),
            }),
        };
        if let Ok(found) = log.thread_views(&held.scope, &query).await {
            for child in found {
                inner.hub.adopt(child.thread, held.id);
            }
        }
    }
}
