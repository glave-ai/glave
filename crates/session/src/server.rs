//! The session server: threads and turns behind the methods of `specs/session.md`. It keeps a
//! thread in memory while it is used, and rebuilds any thread from the log on demand, so a
//! fresh server answers for every thread the log holds.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use glave_event::{Seq, ThreadId};
use glave_harness::{
    Clock, Entropy, Harness, HarnessError, Live, Parts, Settings, TurnSummary, Workspace,
};
use glave_ports::{BlobStore, EventLog, ModelProvider, Scope, ToolSource};
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;

use crate::approvals::{Answering, Approvals};
use crate::held::Slot;
use crate::hub::{Hub, Observed};
use crate::protocol::{
    ApiError, ErrorData, Notification, PROTOCOL, Reply, Request, ServerInfo, code,
};
use crate::turn::TurnStream;

/// The most events or threads one page returns.
const MAX_PAGE: usize = 5_000;

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
    pub(crate) hub: Arc<Hub>,
    pub(crate) approvals: Approvals,
    pub(crate) threads: Mutex<HashMap<ThreadId, Slot>>,
    pub(crate) tasks: Mutex<Vec<JoinHandle<()>>>,
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
                harness: Harness::new(parts, settings).with_tools(wiring.tools),
                scope,
                hub,
                approvals,
                threads: Mutex::new(HashMap::new()),
                tasks: Mutex::new(Vec::new()),
            }),
        }
    }

    /// Answers one request. `turn/start` answers when the turn has ended; to follow it as it
    /// runs, use [`Server::start_turn`].
    pub async fn request(&self, request: Request) -> Result<Reply, ApiError> {
        match request {
            Request::Discover {} => Ok(Reply::Server(ServerInfo {
                name: "glave".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
                protocols: vec![PROTOCOL.to_owned()],
            })),
            Request::ThreadStart {} => self.start_thread().await,
            Request::ThreadList { limit } => self.list(limit).await,
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
            Request::TurnStart { thread, prompt } => {
                let mut stream = self.start_turn(thread, &prompt).await?;
                stream.finish().await.map(|turn| Reply::Turn { turn })
            }
            Request::TurnCancel { thread } => self.cancel(thread).await,
            Request::ThreadCompact { thread } => self.compact(thread).await,
            Request::ApprovalDecide {
                thread,
                approval,
                decision,
            } => {
                self.inner.approvals.decide(thread, &approval, decision)?;
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
        }
    }

    /// Everything `thread` publishes from now on: its events as they are stored, and progress.
    pub fn watch(&self, thread: ThreadId) -> broadcast::Receiver<Arc<Notification>> {
        self.inner.hub.watch(thread)
    }

    /// Starts a turn and streams it: its notifications, then how it ended.
    pub async fn start_turn(&self, thread: ThreadId, prompt: &str) -> Result<TurnStream, ApiError> {
        let notes = self.watch(thread);
        let prompt = prompt.to_owned();
        let done = self
            .occupy(thread, move |inner, mut held, cancel| async move {
                let hub = Arc::clone(&inner.hub);
                let mut publish = |live| {
                    // A subagent's progress goes under its own thread, and so to its watchers
                    // and the parent's.
                    let (from, live) = match live {
                        Live::Child { thread, update } => (thread, *update),
                        live => (thread, live),
                    };
                    let update = Notification::Live {
                        thread: from,
                        update: crate::protocol::Update::from(live),
                    };
                    hub.publish(from, update);
                };
                let ran = inner
                    .harness
                    .run_turn(&mut held, &prompt, &cancel, &mut publish)
                    .await;
                (held, ran)
            })
            .await?;
        Ok(TurnStream::new(notes, done))
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

    async fn start_thread(&self) -> Result<Reply, ApiError> {
        let thread = self
            .inner
            .harness
            .start_thread(self.inner.scope)
            .await
            .map_err(|error| api(&error))?;
        let id = thread.id;
        self.keep(id, thread);
        Ok(Reply::Thread {
            thread: self.summary(id),
        })
    }

    async fn list(&self, limit: Option<usize>) -> Result<Reply, ApiError> {
        let limit = limit.unwrap_or(20).clamp(1, MAX_PAGE);
        let ids = self
            .inner
            .harness
            .log()
            .threads(&self.inner.scope, limit)
            .await
            .map_err(|err| ApiError::new(code::INTERNAL, err.to_string()))?;
        Ok(Reply::Threads {
            threads: ids.into_iter().map(|id| self.summary(id)).collect(),
        })
    }

    async fn events(
        &self,
        thread: ThreadId,
        after: Option<Seq>,
        limit: Option<usize>,
    ) -> Result<Reply, ApiError> {
        let limit = limit.unwrap_or(500).clamp(1, MAX_PAGE);
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
        let done = self
            .occupy(thread, |inner, mut held, cancel| async move {
                let compacted = inner.harness.compact(&mut held, &cancel, &mut |_| {}).await;
                (held, compacted)
            })
            .await?;
        let compacted = done
            .await
            .map_err(|_| ApiError::new(code::INTERNAL, "the compaction stopped"))?
            .map_err(|error| api(&error))?;
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
        HarnessError::Cancelled => ApiError::new(code::CANCELLED, "cancelled"),
        _ => ApiError::new(code::INTERNAL, error.to_string()),
    }
}

pub(crate) fn no_thread(thread: ThreadId) -> ApiError {
    ApiError::new(code::NO_THREAD, format!("no thread {thread} in this space"))
}

/// A finished turn's summary, for callers that only need the result.
pub(crate) type Ended = Result<TurnSummary, HarnessError>;
