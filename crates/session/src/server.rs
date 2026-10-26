//! The session server: threads and turns behind the methods of `specs/session.md`. It keeps a
//! thread in memory while it is used, and rebuilds any thread from the log on demand, so a
//! fresh server answers for every thread the log holds, in every project of its organization,
//! to the callers who work in that project.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, RwLock};

use glave_event::{Body, PrincipalId, RawEvent, Seq, ThreadId, decode};
use glave_harness::{
    Harness, HarnessError, Parts, Place, ProjectFolders, ReleasedWorker, Settings, TurnSummary,
    Workers, Workspace,
};
use glave_identity::signin::Browsers;
use glave_ports::computer::{ComputerError, Environment};
use glave_ports::host::PublicOrigin;
use glave_ports::memories::Memory;
use glave_ports::settings::Hooks;
use glave_ports::{
    BlobStore, Caller, Clock, Entropy, EventLog, Holder, InMemoryLeases, Lease, Leases,
    ModelProvider, OrgId, OsEntropy, ProjectId, Projects, Scope, SystemClock, ToolSource,
};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::answers::Followed;
use crate::approvals::{Answering, Approvals};
use crate::awake::{Awake, KeepAwake};
use crate::choice::Choice;
use crate::connections::Connections;
use crate::credentials::GitHosts;
use crate::held::{Slot, finished};
use crate::hub::{Hub, Observed};
use crate::keyed::Keyed;
use crate::ledger::Ledgers;
use crate::projects::Places;
use crate::protocol::{ApiError, ModelInfo, Reply, code};
use crate::providers::Providers;
use crate::reread::Rereads;
use crate::standing::Worktrees;
use crate::tasks::{TaskLimits, Tasks};
use crate::trust::{Asks, ProjectTrust};
use crate::words::Sentence;
use glave_ports::stream::Reserved;

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
    /// When set, every new thread in the project whose settings chose workers works on a
    /// worker of its own (`on = "cloud"`) unless `thread/start` asks for a worktree; none by
    /// default.
    pub workers: Option<OnWorkers>,
    /// What one person may run at once and keep scheduled.
    pub tasks: TaskLimits,
    /// What threads remember from one to the next (`specs/memory.md`); none when memory is off.
    pub memory: Option<Arc<dyn Memory>>,
    /// The projects of the server's organization, and who owns each.
    pub projects: Arc<dyn Projects>,
    /// The models threads may choose besides the settings' own, the default (`model/list`).
    /// The provider must serve each by its id: the gateway's choices.
    pub models: Vec<ModelInfo>,
    /// The model accounts the person on this computer connects, which the models follow;
    /// none where whoever runs the server sets up its models.
    pub providers: Option<Arc<dyn Providers>>,
    /// What reads the host's own settings again at the start of each turn, which the models
    /// and the person's own tools follow; none where they are read only at the start.
    pub rereads: Option<Arc<dyn Rereads>>,
    /// The git hosts the person on this computer connects; none where whoever runs the
    /// server sets up what its threads sign in to.
    pub git_hosts: Option<Arc<dyn GitHosts>>,
    /// The connections the person on this computer signs in to; none where whoever runs the server
    /// sets up what its threads sign in to.
    pub connections: Option<Arc<dyn Connections>>,
    /// Where a thread in a folder of its own reads the instructions it opens with (its
    /// project's AGENTS.md and what its folder brings that stays off), and what its project
    /// turns on at each turn. None: every thread opens with the settings' instructions, with
    /// the server's tools and hooks alone.
    pub project_folders: Option<Arc<dyn ProjectFolders>>,
    /// Where what a project asks to be trusted is read, and the person's yes kept; none where
    /// no one may trust a project from a client.
    pub trust: Option<Arc<dyn ProjectTrust>>,
    /// The browsers signed in on this computer, by links' one-time codes; none where no
    /// browser is signed in that way, as over stdio.
    pub browsers: Option<Arc<Browsers>>,
    /// Where people reach the server from elsewhere, through what serves HTTPS for it; none
    /// where it answers only on its own computer.
    pub origin: Option<PublicOrigin>,
    /// The leases work on a thread runs under, kept in the store beside its log, so servers on
    /// one store never run one thread at once; none keeps them in this server's memory, for a
    /// server alone on its store.
    pub leases: Option<Arc<dyn Leases>>,
    /// What keeps the computer from sleeping on its own while work runs: only on a person's
    /// own computer, never on a server, which does not sleep.
    pub awake: Option<Arc<dyn KeepAwake>>,
    /// What lets go of a thread's worktrees on the host's computer once it is archived or
    /// deleted; none where the host keeps none.
    pub worktrees: Option<Arc<dyn Worktrees>>,
}

impl<P, L, B> Wiring<P, L, B> {
    /// A server's wiring with what it cannot do without: its models, its store's log, blobs
    /// and projects, and the workspace on the computer its threads work on. Everything else
    /// starts off or empty: the system clock and the operating system's randomness; no
    /// `web_fetch`, other tools, hooks, workers, memory, model accounts, git hosts, connections,
    /// rereading, project folders, trust, browsers or worktrees to let go of; no limit on tasks;
    /// only the settings' own model and mode; threads in their project's folder rather than a
    /// worktree; reached only on its own computer, which it does not keep awake; leases in its
    /// own memory. An embedder names only what it changes: `Wiring { hooks, ..Wiring::new(...) }`.
    pub fn new(
        provider: P,
        log: L,
        blobs: B,
        workspace: Workspace,
        projects: Arc<dyn Projects>,
    ) -> Self {
        Self {
            provider,
            log,
            blobs,
            clock: Box::new(SystemClock),
            entropy: Box::new(OsEntropy),
            workspace,
            web: None,
            tools: Vec::new(),
            own: Vec::new(),
            worktree: false,
            hooks: Hooks::default(),
            workers: None,
            tasks: TaskLimits::default(),
            memory: None,
            projects,
            models: Vec::new(),
            providers: None,
            rereads: None,
            git_hosts: None,
            connections: None,
            project_folders: None,
            trust: None,
            browsers: None,
            origin: None,
            leases: None,
            awake: None,
            worktrees: None,
        }
    }
}

/// Threads on workers: where workers come from, the environment each is made from, and the
/// project whose settings chose them, whose repository they clone.
#[derive(Debug, Clone)]
pub struct OnWorkers {
    pub workers: Workers,
    pub environment: Environment,
    pub project: ProjectId,
}

/// A session server for the projects of one organization, its own first, whose threads work in
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
    /// The organization whose projects this server serves.
    pub(crate) org: OrgId,
    pub(crate) projects: Arc<dyn Projects>,
    /// The projects of threads looked up.
    pub(crate) places: Mutex<Places>,
    /// Whether new threads in a project's folder get a worktree when `thread/start` does not
    /// say: the person's choice, which follows their settings.
    pub(crate) worktree: AtomicBool,
    /// The environment new threads' workers are made from, and the project whose threads go
    /// to them, when threads work on workers.
    pub(crate) worker: Option<(ProjectId, Environment)>,
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
    /// Where sign-ins to connections are kept, when the person here may sign in.
    pub(crate) connections: Option<Arc<dyn Connections>>,
    /// Once it listens over HTTP, its address on this computer when it listens on loopback:
    /// where a browser here comes back to from signing in to a remote server.
    pub(crate) reached: OnceLock<Option<String>>,
    /// The project's own person, whom the log leaves implied.
    pub(crate) person: Option<PrincipalId>,
    pub(crate) limits: TaskLimits,
    /// What each project's task ledger says, as far as this server has read it.
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
    /// Where people reach the server from elsewhere, if they do.
    pub(crate) origin: Option<PublicOrigin>,
    /// What the threads opened or started here did with their projects' asks.
    pub(crate) asks: Mutex<Asks>,
    /// Set once the server stops: it starts no new work (`stopping.rs`).
    pub(crate) stopping: AtomicBool,
    /// The leases work on a thread runs under (`leased.rs`).
    pub(crate) leases: Arc<dyn Leases>,
    /// Who holds this server's leases, made the first time it takes one.
    pub(crate) holder: OnceLock<Holder>,
    /// The organization's scheduler lease, while this server holds it (`schedule.rs`).
    pub(crate) scheduling: Mutex<Option<Lease>>,
    /// Where each waiting turn sends how it ends, for whoever follows it (`answers.rs`).
    pub(crate) followed: Followed,
    /// The computer held awake while work runs, where it is.
    pub(crate) awake: Option<Awake>,
    /// What lets go of a thread's worktrees once it is put away, where the host keeps them.
    pub(crate) worktrees: Option<Arc<dyn Worktrees>>,
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// A server for the projects of `org`, its threads working on the computer
    /// `wiring.workspace` is on: in their projects' folders, or scratch folders. It has no
    /// project of its own: a request that names none acts in the caller's Chats.
    pub fn new(
        wiring: Wiring<P, L, B>,
        settings: Settings,
        org: OrgId,
        answering: Answering,
    ) -> Self {
        let hub = Arc::new(Hub::default());
        let approvals = Approvals::new(answering, Arc::clone(&hub));
        let person = settings.person;
        let catalog = crate::models::catalog(&settings.model, wiring.models);
        let clock: Arc<dyn Clock> = Arc::from(wiring.clock);
        let leases = wiring
            .leases
            .unwrap_or_else(|| Arc::new(InMemoryLeases::new(Arc::clone(&clock))));
        let parts = Parts {
            provider: wiring.provider,
            log: Observed::new(wiring.log, Arc::clone(&hub)),
            blobs: wiring.blobs,
            approver: approvals.clone(),
            clock: Box::new(clock),
            entropy: wiring.entropy,
            workspace: wiring.workspace,
        };
        let harness = match wiring.workers.clone() {
            Some(on) => Harness::new(parts, settings).with_workers(on.workers),
            None => Harness::new(parts, settings),
        }
        .with_web(wiring.web)
        .with_tools(wiring.tools)
        .with_own_tools(wiring.own)
        .with_hooks(wiring.hooks)
        .with_memory_if(wiring.memory)
        .with_project_folders(wiring.project_folders);
        harness.set_without_images(crate::models::without_images(&catalog));
        Self {
            inner: Arc::new(Inner {
                harness,
                worker: wiring.workers.map(|on| (on.project, on.environment)),
                models: RwLock::new(catalog),
                providers: wiring.providers,
                rereads: wiring.rereads,
                changing_models: tokio::sync::Mutex::new(()),
                git_hosts: wiring.git_hosts,
                connections: wiring.connections,
                reached: OnceLock::new(),
                person,
                limits: wiring.tasks,
                ledgers: Mutex::new(Ledgers::new()),
                running: Mutex::new(Tasks::new()),
                org,
                projects: wiring.projects,
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
                origin: wiring.origin,
                stopping: AtomicBool::new(false),
                leases,
                holder: OnceLock::new(),
                scheduling: Mutex::new(None),
                followed: Followed::default(),
                awake: wiring.awake.map(Awake::new),
                worktrees: wiring.worktrees,
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

    /// The projects this server serves, for a host to manage.
    pub fn projects(&self) -> &Arc<dyn Projects> {
        &self.inner.projects
    }

    /// Releases the worker of every thread held idle that works on one (call after
    /// [`Server::shutdown`], when no turn runs): what each left behind, or why it could not be
    /// released.
    pub async fn release_workers(&self) -> Vec<(ThreadId, Result<ReleasedWorker, HarnessError>)> {
        let idle: Vec<(ThreadId, Box<glave_harness::Thread>, jiff::Timestamp)> = {
            let mut threads = self.lock();
            let ids: Vec<ThreadId> = threads
                .iter()
                .filter(|(_, slot)| matches!(slot, Slot::Idle { .. }))
                .map(|(id, _)| *id)
                .collect();
            ids.into_iter()
                .filter_map(|id| match threads.remove(&id) {
                    Some(Slot::Idle { thread, since }) => Some((id, thread, since)),
                    _ => None,
                })
                .collect()
        };
        let mut released = Vec::new();
        for (id, mut thread, since) in idle {
            match self.inner.harness.release_worker(&mut thread).await {
                Ok(Some(left)) => released.push((id, Ok(left))),
                Ok(None) => {}
                Err(error) => released.push((id, Err(error))),
            }
            self.lock().insert(id, Slot::Idle { thread, since });
        }
        released
    }

    /// Starts a thread for `caller` in `project`. In a project with a folder, the thread works
    /// in a worktree of it where `worktree` says (the server's choice when it does not), else
    /// in the folder itself; a task's thread (`background`) always works on a copy, never in
    /// the folder. In the project whose settings chose workers, every thread works on a worker
    /// instead, which clones that project's repository. With no folder (a person's Chats, or a project without one), the thread gets a
    /// scratch folder of its own. With a model or a mode in `choice`, its turns go
    /// to that model, or ask that much.
    pub(crate) async fn new_thread(
        &self,
        caller: &Caller,
        scope: Scope,
        worktree: Option<bool>,
        background: bool,
        choice: &Choice,
    ) -> Result<ThreadId, ApiError> {
        // What the person chose meanwhile applies to the new thread.
        self.reread().await;
        if let Some(model) = &choice.model {
            self.known_model(model)?;
        }
        if let Some(effort) = &choice.effort {
            let model = choice.model.clone();
            let model = model.unwrap_or_else(|| self.inner.harness.default_model());
            self.offered_effort(&model, effort)?;
        }
        if let Some(mode) = choice.permissions {
            self.allowed_permissions(mode)?;
        }
        let project = scope.project();
        let own = self.inner.worktree.load(Ordering::Relaxed);
        let folder = self.folder_found(project).await?;
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
                    Sentence::NoFolderForWorktree,
                ));
            }
            (None, _, _) => Place::Scratch,
            (Some(_), Some(false), _) if background => return Err(crate::tasks::in_place()),
            (Some(folder), Some(true), _) => Place::Worktree(folder),
            (Some(_), None, Some((chosen, environment))) if *chosen == project => {
                Place::Worker(environment.clone())
            }
            (Some(folder), None, _) if background || copies => Place::Worktree(folder),
            (Some(folder), _, _) => Place::Folder(folder),
        };
        let copy = background && matches!(place, Place::Worktree(_));
        let mut thread = self
            .inner
            .harness
            .start_thread_in(scope, caller, place)
            .await
            .map_err(|error| match error {
                HarnessError::Workspace(ComputerError::Refused(_)) if copy => {
                    crate::tasks::no_copy()
                }
                error => ApiError::from(&error),
            })?;
        crate::choice::record(&self.inner.harness, &mut thread, caller, choice)
            .await
            .map_err(|error| ApiError::from(&error))?;
        let id = thread.id;
        self.place(id, scope);
        self.keep(id, thread).await;
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
        let latest = self
            .inner
            .harness
            .log()
            .latest_of(scope, thread, &[kind])
            .await?;
        latest
            .map(|raw| decode(raw).map(|event| event.body).map_err(ApiError::from))
            .transpose()
    }

    pub(crate) async fn events(
        &self,
        thread: ThreadId,
        after: Option<Seq>,
        limit: Option<usize>,
    ) -> Result<Reply, ApiError> {
        let (events, next) = self.page(thread, after, limit).await?;
        Ok(Reply::Events { events, next })
    }

    /// A page of `thread`'s events after `after`, at most `limit` (500 when absent), and the
    /// `seq` the next page follows, when there may be one.
    pub(crate) async fn page(
        &self,
        thread: ThreadId,
        after: Option<Seq>,
        limit: Option<usize>,
    ) -> Result<(Vec<RawEvent>, Option<Seq>), ApiError> {
        let limit = limit.unwrap_or(500).clamp(1, MAX_PAGE);
        if Reserved::of(thread).is_some() {
            return Err(no_thread(thread));
        }
        self.ensure_not_deleted(thread).await?;
        let scope = self.scope_of(thread).await?;
        let events = self
            .inner
            .harness
            .log()
            .read(&scope, thread, after, limit)
            .await?;
        if events.is_empty() && after.is_none() {
            return Err(no_thread(thread));
        }
        let next = (events.len() == limit)
            .then(|| events.last().map(|event| event.seq))
            .flatten();
        Ok((events, next))
    }

    pub(crate) async fn cancel(&self, thread: ThreadId) -> Result<Reply, ApiError> {
        let running = {
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
                    Some(finished.clone())
                }
                _ => None,
            }
        };
        let Some(mut finished) = running else {
            // A turn waiting for an answer holds nothing here: it goes on, stopped.
            return self.stop_waiting(thread).await;
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
            .ok_or_else(|| ApiError::new(code::INTERNAL, Sentence::CompactionStopped))?;
        let compacted = ended.as_ref().as_ref().map_err(ApiError::from)?;
        Ok(Reply::Compacted {
            before_tokens: compacted.before_tokens,
            after_tokens: compacted.after_tokens,
        })
    }
}

pub(crate) fn no_thread(thread: ThreadId) -> ApiError {
    ApiError::new(
        code::NO_THREAD,
        Sentence::NoThread {
            thread: thread.to_string(),
        },
    )
}

/// A finished turn's summary, for callers that only need the result.
pub(crate) type Ended = Result<TurnSummary, HarnessError>;
