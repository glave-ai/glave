//! The session server: threads and turns behind the methods of `specs/session.md`. It keeps a
//! thread in memory while it is used, and rebuilds any thread from the log on demand, so a
//! fresh server answers for every thread the log holds, in every space of its organization,
//! to the callers who work in that space.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use glave_event::{Body, PrincipalId, Seq, ThreadId, decode};
use glave_harness::{
    Caller, Clock, Entropy, Harness, HarnessError, Hooks, Memory, Parts, Place, Projects,
    ReleasedWorker, Settings, TurnSummary, Workers, Workspace,
};
use glave_ports::{
    BlobStore, ComputerError, Environment, EventLog, ModelProvider, Scope, SpaceId, Spaces,
    ToolSource,
};
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;

use crate::approvals::{Answering, Approvals};
use crate::credentials::GitHosts;
use crate::held::{Slot, finished};
use crate::hub::{Hub, Observed};
use crate::keyed::Keyed;
use crate::ledger::{Ledgers, is_ledger};
use crate::modes::Choice;
use crate::protocol::{ApiError, ApprovalMode, ErrorData, ModelInfo, Notification, Reply, code};
use crate::providers::Providers;
use crate::spaces::Places;
use crate::tasks::{TaskLimits, Tasks};
use crate::trust::{Asks, ProjectTrust};

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
    /// Whether a new thread in a project with a folder works in a worktree of its own unless
    /// `thread/start` says; off, it works in the folder itself.
    pub worktree: bool,
    /// Commands run at points of every turn.
    pub hooks: Hooks,
    /// When set, every new thread in the server's own space works on a worker of its own
    /// (`--on cloud`) unless `thread/start` asks for a worktree; none by default.
    pub workers: Option<OnWorkers>,
    /// What one person may run at once and keep scheduled.
    pub tasks: TaskLimits,
    /// What threads remember from one to the next (`specs/memory.md`); none when memory is off.
    pub memory: Option<Arc<dyn Memory>>,
    /// The spaces of the server's organization: who owns each and who works in it.
    pub spaces: Arc<dyn Spaces>,
    /// The models threads may choose besides the settings' own, the default (`model/list`).
    /// The provider must serve each by its id: the gateway's choices.
    pub models: Vec<ModelInfo>,
    /// The approval modes a thread may choose besides the settings' own: those the policy
    /// leaves, where auto runs behind a wall (`specs/approvals.md`).
    pub modes: Vec<glave_harness::ApprovalMode>,
    /// The model accounts the person on this computer connects, which the models follow;
    /// none where whoever runs the server sets up its models.
    pub providers: Option<Arc<dyn Providers>>,
    /// The git hosts the person on this computer connects; none where whoever runs the
    /// server sets up what its threads sign in to.
    pub git_hosts: Option<Arc<dyn GitHosts>>,
    /// Where a thread in a folder of its own reads the instructions it opens with (its
    /// project's AGENTS.md and what its folder brings that stays off), and what its project
    /// turns on at each turn. None: every thread opens with the settings' instructions, with
    /// the server's tools and hooks alone.
    pub projects: Option<Arc<dyn Projects>>,
    /// Where what a project asks to be trusted is read, and the person's yes kept; none where
    /// no one may trust a project from a client.
    pub trust: Option<Arc<dyn ProjectTrust>>,
}

/// Threads on workers: where workers come from, and the environment each is made from.
#[derive(Debug, Clone)]
pub struct OnWorkers {
    pub workers: Workers,
    pub environment: Environment,
}

/// A session server for the spaces of one organization, its own first, whose threads work in
/// their projects' folders on the computer its workspace is on. Clones share it.
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
    /// The server's own space: where a request that names none acts, and whose memory this
    /// is.
    pub(crate) home: Scope,
    pub(crate) spaces: Arc<dyn Spaces>,
    /// The spaces of threads looked up.
    pub(crate) places: Mutex<Places>,
    /// Whether new threads in a project's folder get a worktree when `thread/start` does not
    /// say.
    pub(crate) worktree: bool,
    /// The environment new threads' workers are made from, when they work on workers.
    pub(crate) worker: Option<Environment>,
    /// The models threads may be sent to, the default first; none until one is set up.
    pub(crate) models: RwLock<Vec<ModelInfo>>,
    /// Where model accounts are kept, when the person here may connect them.
    pub(crate) providers: Option<Arc<dyn Providers>>,
    /// Held while model accounts change and the models follow.
    pub(crate) changing_models: tokio::sync::Mutex<()>,
    /// Where git hosts are kept, when the person here may connect them.
    pub(crate) git_hosts: Option<Arc<dyn GitHosts>>,
    /// The modes a thread may choose, cautious first.
    pub(crate) modes: Vec<ApprovalMode>,
    /// The space's own person, whom the log leaves implied.
    pub(crate) person: Option<PrincipalId>,
    pub(crate) limits: TaskLimits,
    /// What each space's task ledger says, as far as this server has read it.
    pub(crate) ledgers: Mutex<Ledgers>,
    /// Tasks running in this server.
    pub(crate) running: Mutex<Tasks>,
    pub(crate) hub: Arc<Hub>,
    pub(crate) approvals: Approvals,
    pub(crate) threads: Mutex<HashMap<ThreadId, Slot>>,
    pub(crate) tasks: Mutex<Vec<JoinHandle<()>>>,
    /// Turns started with an idempotency key, for retries.
    pub(crate) keyed: Keyed,
    /// Where what a project asks to be trusted is read, and the person's yes kept.
    pub(crate) trust: Option<Arc<dyn ProjectTrust>>,
    /// What the threads opened or started here did with their projects' asks.
    pub(crate) asks: Mutex<Asks>,
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// A server whose own space is `home`, its threads working on the computer
    /// `wiring.workspace` is on: in their projects' folders, or scratch folders.
    pub fn new(
        wiring: Wiring<P, L, B>,
        settings: Settings,
        home: Scope,
        answering: Answering,
    ) -> Self {
        let hub = Arc::new(Hub::default());
        let approvals = Approvals::new(answering, Arc::clone(&hub));
        let person = settings.person;
        let catalog = crate::models::catalog(&settings.model, wiring.models);
        let choosable = crate::modes::choosable(settings.approval, &wiring.modes);
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
                .with_hooks(wiring.hooks)
                .with_memory_if(wiring.memory)
                .with_projects(wiring.projects),
                worker: wiring.workers.map(|on| on.environment),
                models: RwLock::new(catalog),
                providers: wiring.providers,
                changing_models: tokio::sync::Mutex::new(()),
                git_hosts: wiring.git_hosts,
                modes: choosable,
                person,
                limits: wiring.tasks,
                ledgers: Mutex::new(Ledgers::new()),
                running: Mutex::new(Tasks::new()),
                home,
                spaces: wiring.spaces,
                places: Mutex::new(Places::default()),
                worktree: wiring.worktree,
                hub,
                approvals,
                threads: Mutex::new(HashMap::new()),
                tasks: Mutex::new(Vec::new()),
                keyed: Keyed::default(),
                trust: wiring.trust,
                asks: Mutex::new(Asks::default()),
            }),
        }
    }

    /// The person threads are for unless someone else starts them.
    pub fn person(&self) -> Option<PrincipalId> {
        self.inner.harness.person()
    }

    /// The spaces this server serves, for a host to manage.
    pub fn spaces(&self) -> &Arc<dyn Spaces> {
        &self.inner.spaces
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

    /// Starts a thread for `caller` in `space`. In a project with a folder, the thread works
    /// in a worktree of it where `worktree` says (the server's choice when it does not), else
    /// in the folder itself; a task's thread (`background`) always works on a copy, never in
    /// the folder. In the server's own space, every thread works on a worker instead when the
    /// server puts them there, which clones the repository the server's workers are set up
    /// with. With no folder (a person's Chats, or a project without one), the thread gets a
    /// scratch folder of its own. With a model or an approval mode in `choice`, its turns go
    /// to that model, or ask that much.
    pub(crate) async fn new_thread(
        &self,
        caller: &Caller,
        space: SpaceId,
        worktree: Option<bool>,
        background: bool,
        choice: &Choice,
    ) -> Result<ThreadId, ApiError> {
        if let Some(model) = &choice.model {
            self.known_model(model)?;
        }
        if let Some(mode) = choice.approval {
            self.allowed_mode(mode)?;
        }
        let home = space == self.inner.home.space;
        let folder = self.folder_here(space).await?;
        let place = match (folder, worktree, &self.inner.worker) {
            (None, Some(true), _) => {
                return Err(ApiError::new(
                    code::WORKSPACE,
                    "there is no project folder here to make a worktree of: threads here work \
                     in a scratch folder of their own",
                ));
            }
            (None, _, _) => Place::Scratch,
            (Some(_), Some(false), _) if background => return Err(crate::tasks::in_place()),
            (Some(folder), Some(true), _) => Place::Worktree(folder),
            (Some(_), None, Some(environment)) if home => Place::Worker(environment.clone()),
            (Some(folder), None, _) if background || self.inner.worktree => Place::Worktree(folder),
            (Some(folder), _, _) => Place::Folder(folder),
        };
        let copy = background && matches!(place, Place::Worktree(_));
        let scope = self.scope_in(space);
        let mut thread = self
            .inner
            .harness
            .start_thread_in(scope, caller, place)
            .await
            .map_err(|error| match error {
                HarnessError::Workspace(ComputerError::Refused(_)) if copy => {
                    crate::tasks::no_copy()
                }
                error => api(&error),
            })?;
        crate::modes::record(&self.inner.harness, &mut thread, caller, choice)
            .await
            .map_err(|error| api(&error))?;
        let id = thread.id;
        self.place(id, scope);
        self.keep(id, thread);
        Ok(id)
    }

    /// The latest event of type `kind` in `thread`, decoded: how a choice the thread made is
    /// read back from the log, by any server of the deployment.
    pub(crate) async fn latest(
        &self,
        scope: &Scope,
        thread: ThreadId,
        kind: &str,
    ) -> Result<Option<Body>, ApiError> {
        let failed =
            |error: &dyn std::fmt::Display| ApiError::new(code::INTERNAL, error.to_string());
        let latest = self
            .inner
            .harness
            .log()
            .latest_of(scope, thread, &[kind])
            .await
            .map_err(|error| failed(&error))?;
        latest
            .map(|raw| {
                decode(raw)
                    .map(|event| event.body)
                    .map_err(|error| failed(&error))
            })
            .transpose()
    }

    pub(crate) async fn events(
        &self,
        thread: ThreadId,
        after: Option<Seq>,
        limit: Option<usize>,
    ) -> Result<Reply, ApiError> {
        let limit = limit.unwrap_or(500).clamp(1, MAX_PAGE);
        if is_ledger(thread) {
            return Err(no_thread(thread));
        }
        self.ensure_not_deleted(thread).await?;
        let scope = self.scope_of(thread).await?;
        let events = self
            .inner
            .harness
            .log()
            .read(&scope, thread, after, limit)
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

    pub(crate) async fn cancel(&self, thread: ThreadId) -> Result<Reply, ApiError> {
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

    pub(crate) async fn compact(&self, thread: ThreadId) -> Result<Reply, ApiError> {
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
        HarnessError::Invalid(_) => ApiError::new(code::INVALID_PARAMS, error.to_string()),
        _ => ApiError::new(code::INTERNAL, error.to_string()),
    }
}

pub(crate) fn no_thread(thread: ThreadId) -> ApiError {
    ApiError::new(code::NO_THREAD, format!("no thread {thread} in this space"))
}

/// A finished turn's summary, for callers that only need the result.
pub(crate) type Ended = Result<TurnSummary, HarnessError>;
