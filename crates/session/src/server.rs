//! The session server: threads and turns behind the methods of `specs/session.md`. It keeps a
//! thread in memory while it is used, and rebuilds any thread from the log on demand, so a
//! fresh server answers for every thread the log holds.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use glave_event::{Seq, ThreadId};
use glave_harness::{
    Caller, Change, Clock, Entropy, Harness, HarnessError, Hooks, Parts, Place, ReleasedWorker,
    Settings, TurnSummary, Workers, Workspace,
};
use glave_ports::{BlobStore, Environment, EventLog, ModelProvider, Scope, ToolSource};
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;

use crate::approvals::{Answering, Approvals};
use crate::held::{Slot, finished};
use crate::hub::{Hub, Observed};
use crate::keyed::Keyed;
use crate::protocol::{
    ApiError, ErrorData, Notification, PROTOCOL, Reply, Request, ServerInfo, code,
};

/// The most events or threads one page returns.
pub(crate) const MAX_PAGE: usize = 5_000;

/// What a server is built from: the harness's parts, less the approver, which is the API.
pub struct Wiring<P, L, B> {
    pub provider: P,
    pub log: L,
    pub blobs: B,
    pub clock: Box<dyn Clock>,
    pub entropy: Box<dyn Entropy>,
    pub workspace: Workspace,
    /// Tools beside the built-in ones, such as MCP servers'.
    pub tools: Vec<Arc<dyn ToolSource>>,
    /// Whether a new thread works in a worktree of its own unless `thread/start` says; off,
    /// it works in the workspace itself.
    pub worktree: bool,
    /// Commands run at points of every turn.
    pub hooks: Hooks,
    /// When set, every new thread works on a worker of its own (`--on cloud`) unless
    /// `thread/start` asks for a worktree; none by default.
    pub workers: Option<OnWorkers>,
}

/// Threads on workers: where workers come from, and the environment each is made from.
#[derive(Debug, Clone)]
pub struct OnWorkers {
    pub workers: Workers,
    pub environment: Environment,
}

/// A session server for one workspace and one scope. Clones share it.
pub struct Server<P, L, B> {
    pub(crate) inner: Arc<Inner<P, L, B>>,
}

impl<P, L, B> Clone for Server<P, L, B> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

pub(crate) type Glave<P, L, B> = Harness<P, Observed<L>, B, Approvals>;

pub(crate) struct Inner<P, L, B> {
    pub(crate) harness: Glave<P, L, B>,
    pub(crate) scope: Scope,
    /// Whether new threads get a worktree when `thread/start` does not say.
    pub(crate) worktree: bool,
    /// The environment new threads' workers are made from, when they work on workers.
    pub(crate) worker: Option<Environment>,
    pub(crate) hub: Arc<Hub>,
    pub(crate) approvals: Approvals,
    pub(crate) threads: Mutex<HashMap<ThreadId, Slot>>,
    pub(crate) tasks: Mutex<Vec<JoinHandle<()>>>,
    /// Turns started with an idempotency key, for retries.
    pub(crate) keyed: Keyed,
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    pub fn new(
        wiring: Wiring<P, L, B>,
        settings: Settings,
        scope: Scope,
        answering: Answering,
    ) -> Self {
        let hub = Arc::new(Hub::default());
        let approvals = Approvals::new(answering);
        let parts = Parts {
            provider: wiring.provider,
            log: Observed::new(wiring.log, Arc::clone(&hub)),
            blobs: wiring.blobs,
            approver: approvals.clone(),
            clock: wiring.clock,
            entropy: wiring.entropy,
            workspace: wiring.workspace,
        };
        Self {
            inner: Arc::new(Inner {
                harness: match wiring.workers.clone() {
                    Some(on) => Harness::new(parts, settings).with_workers(on.workers),
                    None => Harness::new(parts, settings),
                }
                .with_tools(wiring.tools)
                .with_hooks(wiring.hooks),
                worker: wiring.workers.map(|on| on.environment),
                scope,
                worktree: wiring.worktree,
                hub,
                approvals,
                threads: Mutex::new(HashMap::new()),
                tasks: Mutex::new(Vec::new()),
                keyed: Keyed::default(),
            }),
        }
    }

    /// Answers one request from the local person. `turn/start` answers when the turn has
    /// ended; to follow it as it runs, use [`Server::start_turn`].
    pub async fn request(&self, request: Request) -> Result<Reply, ApiError> {
        self.request_as(&Caller::default(), request).await
    }

    /// Answers one request from `caller`, the person the transport verified.
    pub async fn request_as(&self, caller: &Caller, request: Request) -> Result<Reply, ApiError> {
        match request {
            Request::Discover {} => Ok(Reply::Server(ServerInfo {
                name: "glave".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
                protocols: vec![PROTOCOL.to_owned()],
            })),
            Request::ThreadStart { worktree } => self.start_thread(caller, worktree).await,
            Request::ThreadList { limit, show } => self.list(limit, show.unwrap_or_default()).await,
            Request::ThreadArchive { thread } => self.change(caller, thread, Change::Archive).await,
            Request::ThreadUnarchive { thread } => {
                self.change(caller, thread, Change::Unarchive).await
            }
            Request::ThreadDelete { thread } => self.change(caller, thread, Change::Delete).await,
            Request::ThreadGet { thread } => {
                self.load(thread).await?;
                Ok(Reply::Thread {
                    thread: self.summary(thread),
                })
            }
            Request::ThreadEvents {
                thread,
                after,
                limit,
            } => self.events(thread, after, limit).await,
            streams @ (Request::TurnStart { .. } | Request::GoalStart { .. }) => {
                match self.start_streaming(caller, streams).await {
                    Ok(started) => started?.finish().await.map(|turn| Reply::Turn { turn }),
                    Err(_) => Err(ApiError::new(code::INTERNAL, "not a streaming method")),
                }
            }
            Request::TurnCancel { thread } => self.cancel(thread).await,
            Request::ThreadCompact { thread } => self.compact(thread).await,
            Request::ApprovalDecide {
                thread,
                approval,
                decision,
            } => {
                let by = caller.person;
                self.inner
                    .approvals
                    .decide(thread, &approval, decision, by)?;
                Ok(Reply::Done {})
            }
            Request::QuestionAnswer {
                thread,
                question,
                answers,
            } => {
                self.inner.approvals.respond(thread, &question, answers)?;
                Ok(Reply::Done {})
            }
            Request::GoalStop { thread } => self.stop_goal(caller, thread).await,
            Request::GoalGet { thread } => Ok(Reply::Goal {
                goal: self.goal(thread).await?,
            }),
        }
    }

    /// Everything `thread` publishes from now on: its events as they are stored, and progress.
    pub fn watch(&self, thread: ThreadId) -> broadcast::Receiver<Arc<Notification>> {
        self.inner.hub.watch(thread)
    }

    /// Releases the worker of every thread held idle that works on one (call after
    /// [`Server::shutdown`], when no turn runs): what each left behind, or why it could not be
    /// released.
    pub async fn release_workers(&self) -> Vec<(ThreadId, Result<ReleasedWorker, HarnessError>)> {
        let idle: Vec<(ThreadId, Box<glave_harness::Thread>)> = {
            let mut threads = self.lock();
            let ids: Vec<ThreadId> = threads
                .iter()
                .filter(|(_, slot)| matches!(slot, Slot::Idle(_)))
                .map(|(id, _)| *id)
                .collect();
            ids.into_iter()
                .filter_map(|id| match threads.remove(&id) {
                    Some(Slot::Idle(thread)) => Some((id, thread)),
                    _ => None,
                })
                .collect()
        };
        let mut released = Vec::new();
        for (id, mut thread) in idle {
            match self.inner.harness.release_worker(&mut thread).await {
                Ok(Some(left)) => released.push((id, Ok(left))),
                Ok(None) => {}
                Err(error) => released.push((id, Err(error))),
            }
            self.lock().insert(id, Slot::Idle(thread));
        }
        released
    }

    /// Cancels every running turn and waits for each to end. Call before dropping the server.
    pub async fn shutdown(&self) {
        let finished: Vec<watch::Receiver<bool>> = {
            let threads = self
                .inner
                .threads
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            threads
                .values()
                .filter_map(|slot| match slot {
                    Slot::Busy { cancel, finished } => {
                        cancel.cancel();
                        Some(finished.clone())
                    }
                    Slot::Idle(_) => None,
                })
                .collect()
        };
        for mut done in finished {
            let _ = done.wait_for(|done| *done).await;
        }
        let tasks: Vec<JoinHandle<()>> = std::mem::take(
            &mut *self
                .inner
                .tasks
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
        for task in tasks {
            let _ = task.await;
        }
    }

    async fn start_thread(
        &self,
        caller: &Caller,
        worktree: Option<bool>,
    ) -> Result<Reply, ApiError> {
        let place = match (worktree, &self.inner.worker) {
            (Some(true), _) => Place::Worktree,
            (None, Some(environment)) => Place::Worker(environment.clone()),
            (None, None) if self.inner.worktree => Place::Worktree,
            _ => Place::Here,
        };
        let thread = self
            .inner
            .harness
            .start_thread_in(self.inner.scope, caller, place)
            .await
            .map_err(|error| api(&error))?;
        let id = thread.id;
        self.keep(id, thread);
        Ok(Reply::Thread {
            thread: self.summary(id),
        })
    }

    async fn events(
        &self,
        thread: ThreadId,
        after: Option<Seq>,
        limit: Option<usize>,
    ) -> Result<Reply, ApiError> {
        let limit = limit.unwrap_or(500).clamp(1, MAX_PAGE);
        self.ensure_not_deleted(thread).await?;
        let events = self
            .inner
            .harness
            .log()
            .read(&self.inner.scope, thread, after, limit)
            .await
            .map_err(|err| ApiError::new(code::INTERNAL, err.to_string()))?;
        if events.is_empty() && after.is_none() {
            return Err(no_thread(thread));
        }
        let next = (events.len() == limit)
            .then(|| events.last().map(|event| event.seq))
            .flatten();
        Ok(Reply::Events { events, next })
    }

    async fn cancel(&self, thread: ThreadId) -> Result<Reply, ApiError> {
        let mut finished = {
            let threads = self
                .inner
                .threads
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            match threads.get(&thread) {
                Some(Slot::Busy { cancel, finished }) => {
                    cancel.cancel();
                    finished.clone()
                }
                _ => {
                    return Err(ApiError::new(
                        code::NO_TURN,
                        format!("no turn is running in thread {thread}"),
                    ));
                }
            }
        };
        let _ = finished.wait_for(|done| *done).await;
        Ok(Reply::Done {})
    }

    async fn compact(&self, thread: ThreadId) -> Result<Reply, ApiError> {
        let (finish, mut done) = watch::channel(None);
        self.occupy(thread, finish, |inner, mut held, cancel| async move {
            let compacted = inner.harness.compact(&mut held, &cancel, &mut |_| {}).await;
            (held, compacted)
        })
        .await?;
        let ended = finished(&mut done)
            .await
            .ok_or_else(|| ApiError::new(code::INTERNAL, "the compaction stopped"))?;
        let compacted = ended.as_ref().as_ref().map_err(api)?;
        Ok(Reply::Compacted {
            before_tokens: compacted.before_tokens,
            after_tokens: compacted.after_tokens,
        })
    }
}

/// A harness failure as the API reports it.
pub(crate) fn api(error: &HarnessError) -> ApiError {
    match error {
        HarnessError::Provider(failed) => ApiError {
            code: code::PROVIDER,
            message: error.to_string(),
            data: Some(ErrorData {
                failure: failed.failure(),
            }),
        },
        HarnessError::NotFound(thread) => no_thread(*thread),
        HarnessError::Archived(_) => ApiError::new(code::ARCHIVED, error.to_string()),
        HarnessError::Cancelled => ApiError::new(code::CANCELLED, "cancelled"),
        HarnessError::NoGoal(_) => ApiError::new(code::NO_GOAL, error.to_string()),
        HarnessError::Workspace(_) => ApiError::new(code::WORKSPACE, error.to_string()),
        HarnessError::PromptBlocked(_) => ApiError::new(code::PROMPT_BLOCKED, error.to_string()),
        _ => ApiError::new(code::INTERNAL, error.to_string()),
    }
}

pub(crate) fn no_thread(thread: ThreadId) -> ApiError {
    ApiError::new(code::NO_THREAD, format!("no thread {thread} in this space"))
}

/// A finished turn's summary, for callers that only need the result.
pub(crate) type Ended = Result<TurnSummary, HarnessError>;
