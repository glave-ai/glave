//! The session server: threads and turns behind the methods of `specs/session.md`. It keeps a
//! thread in memory while it is used, and rebuilds any thread from the log on demand, so a
//! fresh server answers for every thread the log holds, in every space of its organization,
//! to the callers who work in that space.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use glave_event::{Body, PrincipalId, Seq, ThreadId, decode};
use glave_harness::{
    Harness, HarnessError, Hooks, Parts, Place, Projects, ReleasedWorker, Settings, TurnSummary,
    Workers, Workspace,
};
use glave_identity::signin::Browsers;
use glave_ports::computer::{ComputerError, Environment};
use glave_ports::memories::Memory;
use glave_ports::{
    BlobStore, Caller, Clock, Entropy, EventLog, ModelProvider, Scope, SpaceId, Spaces, ToolSource,
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
use crate::reread::Rereads;
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
    /// `web_fetch`, which a project's own settings replace for its threads; none when it is
    /// off.
    pub web: Option<Arc<dyn ToolSource>>,
    /// Tools beside the built-in ones and `web_fetch`, such as skills.
    pub tools: Vec<Arc<dyn ToolSource>>,
    /// The person's own tools, such as their MCP servers': for their threads alone, never a
    /// thread of anyone signed in from elsewhere.
    pub own: Vec<Arc<dyn ToolSource>>,
    /// Whether a new thread in a project with a folder works in a worktree of its own unless
    /// `thread/start` or its project says; off, it works in the folder itself.
    pub worktree: bool,
    /// Commands run at points of every turn of the person the server works for.
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
    /// What reads the host's own settings again at the start of each turn, which the models
    /// and the person's own tools follow; none where they are read only at the start.
    pub rereads: Option<Arc<dyn Rereads>>,
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
    /// The browsers signed in on this computer, by links' one-time codes; none where no
    /// browser is signed in that way, as over stdio.
    pub browsers: Option<Arc<Browsers>>,
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
    /// say: the person's choice, which follows their settings.
    pub(crate) worktree: AtomicBool,
    /// The environment new threads' workers are made from, when they work on workers.
    pub(crate) worker: Option<Environment>,
    /// The models threads may be sent to, the default first; none until one is set up.
    pub(crate) models: RwLock<Vec<ModelInfo>>,
    /// Where model accounts are kept, when the person here may connect them.
    pub(crate) providers: Option<Arc<dyn Providers>>,
    /// What reads the host's own settings again, when it has any.
    pub(crate) rereads: Option<Arc<dyn Rereads>>,
    /// Held while model accounts or the host's settings change and the models follow.
    pub(crate) changing_models: tokio::sync::Mutex<()>,
    /// Where git hosts are kept, when the person here may connect them.
    pub(crate) git_hosts: Option<Arc<dyn GitHosts>>,
    /// The modes a thread may choose, cautious first, as the policy now leaves them.
    pub(crate) modes: RwLock<Vec<ApprovalMode>>,
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
    /// The browsers signed in on this computer, when it signs them in.
    pub(crate) browsers: Option<Arc<Browsers>>,
    /// What the threads opened or started here did with their projects' asks.
    pub(crate) asks: Mutex<Asks>,
    /// Set once the server stops: it starts no new work (`stopping.rs`).
    pub(crate) stopping: AtomicBool,
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
                .with_web(wiring.web)
                .with_tools(wiring.tools)
                .with_own_tools(wiring.own)
                .with_hooks(wiring.hooks)
                .with_memory_if(wiring.memory)
                .with_projects(wiring.projects),
                worker: wiring.workers.map(|on| on.environment),
                models: RwLock::new(catalog),
                providers: wiring.providers,
                rereads: wiring.rereads,
                changing_models: tokio::sync::Mutex::new(()),
                git_hosts: wiring.git_hosts,
                modes: RwLock::new(choosable),
                person,
                limits: wiring.tasks,
                ledgers: Mutex::new(Ledgers::new()),
                running: Mutex::new(Tasks::new()),
                home,
                spaces: wiring.spaces,
                places: Mutex::new(Places::default()),
                worktree: AtomicBool::new(wiring.worktree),
                hub,
                approvals,
                threads: Mutex::new(HashMap::new()),
                tasks: Mutex::new(Vec::new()),
                keyed: Keyed::default(),
                trust: wiring.trust,
                asks: Mutex::new(Asks::default()),
                browsers: wiring.browsers,
                stopping: AtomicBool::new(false),
            }),
        }
    }

    /// The person threads are for unless someone else starts them.
    pub fn person(&self) -> Option<PrincipalId> {
        self.inner.harness.person()
    }

    /// Whether what `caller` starts is the person's the server works for, whose trust turns
    /// on what a project brings.
    pub(crate) fn is_theirs(&self, caller: &Caller) -> bool {
        caller
            .person
            .is_none_or(|person| Some(person) == self.person())
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
        // What the person chose meanwhile applies to the new thread.
        self.reread().await;
        if let Some(model) = &choice.model {
            self.known_model(model)?;
        }
        if let Some(mode) = choice.approval {
            self.allowed_mode(mode)?;
        }
        let home = space == self.inner.home.space;
        let own = self.inner.worktree.load(Ordering::Relaxed);
        let folder = self.folder_found(space).await?;
        // The project's own choice, for the person here, is nearer than the server's.
        let copies = match &folder {
            Some(folder) if self.is_theirs(caller) => self
                .inner
                .harness
                .settled(std::path::Path::new(folder))
                .await
                .worktree
                .unwrap_or(own),
            _ => own,
        };
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
            (Some(folder), None, _) if background || copies => Place::Worktree(folder),
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
                Some(Slot::Busy {
                    cancel, finished, ..
                }) => {
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
