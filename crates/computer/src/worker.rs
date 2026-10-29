//! Workers: disposable computers, one per thread, started from an environment's pinned image
//! by a backend (Docker, or Kubernetes with the `kubernetes` feature), with glave inside
//! serving the computer protocol. The harness drives each one through this adapter, which
//! keeps every call on the right worker over TLS (or a stream, for a worker with no network),
//! within policy's limits. Work leaves a worker only as a pushed branch, a file handed back, or
//! a store write; destroying it removes it. Each worker is labelled with the glave that started
//! it, and a glave starting removes those whose glave is gone ([`WorkerComputers::reap`]).

mod backend;
mod credentials;
mod docker;
mod engine;
#[cfg(feature = "kubernetes")]
mod kubernetes;
mod limits;
mod reconnect;
mod route;
mod tar;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use glave_ports::computer::{
    Command, ComputerError, ComputerId, ComputerInfo, Computers, Ending, Environment, Limits,
    Output, Refusal, Sources, WorkspaceId, WorkspaceInfo,
};
use glave_ports::{BoxFuture, Cancellation, Clock, NeverCancelled, SystemClock};
use glave_process::Glave;
use tokio::task::JoinHandle;

use crate::remote::RemoteComputers;
use crate::tls::{Identity, NO_HANDSHAKE, NOT_LISTENING, connect_tls};
use crate::token::Token;
use backend::{Backend, Link, Spec, Started};
use reconnect::Connection;

pub use credentials::{WorkerCredentials, https_url};
pub use docker::{Docker, DockerSettings};
#[cfg(feature = "kubernetes")]
pub use kubernetes::{Holding, Kubernetes, KubernetesSettings, People};

/// How long a worker's token opens connections: long enough for it to start.
const TOKEN_LIFETIME: Duration = Duration::from_mins(10);
/// How long a TLS worker has to start listening, tried every [`RETRY`]. One not listening
/// yet refuses the connection, or (behind a port proxy) drops it before the handshake: the
/// only failures tried again.
const START_WAIT: Duration = Duration::from_secs(60);
const RETRY: Duration = Duration::from_millis(250);
/// The longest one setup command may run.
const SETUP_TIMEOUT_SECS: u64 = 30 * 60;
/// The most setup output kept for an error.
const SETUP_TAIL: usize = 4_000;

/// One worker as the harness side holds it.
struct Worker {
    name: String,
    connection: Connection,
    environment: Environment,
    /// When it stops taking new work, if policy limits its run time.
    deadline: Option<SystemTime>,
    minutes: Option<u32>,
    /// Whether its commands may reach the network.
    network: bool,
    /// Whether its setup commands have run (they run in its first workspace).
    set_up: tokio::sync::Mutex<bool>,
    /// Moves the bytes of a stream link; stopped with the worker.
    link_task: Option<JoinHandle<()>>,
}

#[derive(Default)]
struct Held {
    workers: HashMap<ComputerId, Arc<Worker>>,
    workspaces: HashMap<WorkspaceId, ComputerId>,
}

/// Workers as computers. Each `computer/create` starts one; `computer/destroy` removes it.
pub struct WorkerComputers {
    backend: Box<dyn Backend>,
    limits: Limits,
    credentials: WorkerCredentials,
    /// The time now; injected so tests decide when a worker runs out of time.
    clock: Arc<dyn Clock>,
    held: Mutex<Held>,
    /// This glave, read once, which its workers are labelled with.
    starter: tokio::sync::OnceCell<Option<Glave>>,
}

impl std::fmt::Debug for WorkerComputers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerComputers")
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl WorkerComputers {
    /// Workers on Docker, within `limits`, fetching sources with `credentials`.
    pub fn docker(docker: Docker, limits: Limits, credentials: WorkerCredentials) -> Self {
        Self::on(Box::new(docker), limits, credentials)
    }

    /// Workers on Kubernetes, within `limits`, fetching sources with `credentials`.
    #[cfg(feature = "kubernetes")]
    pub fn kubernetes(
        kubernetes: Kubernetes,
        limits: Limits,
        credentials: WorkerCredentials,
    ) -> Self {
        Self::on(Box::new(kubernetes), limits, credentials)
    }

    pub(crate) fn on(
        backend: Box<dyn Backend>,
        limits: Limits,
        credentials: WorkerCredentials,
    ) -> Self {
        Self {
            backend,
            limits,
            credentials,
            clock: Arc::new(SystemClock),
            held: Mutex::new(Held::default()),
            starter: tokio::sync::OnceCell::new(),
        }
    }

    /// Labels the workers this starts with `starter` instead of this glave, for the tests that
    /// leave workers as a glave that is gone would.
    #[cfg(test)]
    pub(crate) fn started_by(self, starter: Option<Glave>) -> Self {
        Self {
            starter: tokio::sync::OnceCell::new_with(Some(starter)),
            ..self
        }
    }

    /// This glave, as its workers are labelled with it.
    async fn starter(&self) -> Option<Glave> {
        self.starter
            .get_or_init(|| async {
                tokio::task::spawn_blocking(Glave::this)
                    .await
                    .ok()
                    .flatten()
            })
            .await
            .clone()
    }

    /// Removes the workers on this backend whose glave is gone, as a glave starts: each
    /// labelled with a glave that ran on this machine and whose process is no longer there.
    /// A worker of a glave still running, of one on another machine, or with no label, is
    /// left alone. Returns how many were removed.
    pub async fn reap(&self) -> Result<usize, ComputerError> {
        let listed = self.backend.list().await?;
        let held: Vec<String> = self
            .held()?
            .workers
            .values()
            .map(|worker| worker.name.clone())
            .collect();
        let labelled: Vec<(String, Glave)> = listed
            .into_iter()
            .filter(|listed| !held.contains(&listed.name))
            .filter_map(|listed| {
                let starter = serde_json::from_str(listed.started_by.as_deref()?).ok()?;
                Some((listed.name, starter))
            })
            .collect();
        let starters: Vec<Glave> = labelled.iter().map(|(_, glave)| glave.clone()).collect();
        let gone = tokio::task::spawn_blocking(move || Glave::gone(&starters))
            .await
            .map_err(|error| {
                ComputerError::Failed(format!("could not tell which glaves are gone: {error}"))
            })?;
        let mut removed = 0;
        for ((name, _), gone) in labelled.iter().zip(gone) {
            if gone {
                self.backend.remove(name).await?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Uses `clock` for the time, instead of the system's.
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Removes every worker still held: for when the harness side stops.
    pub async fn destroy_all(&self) -> Result<(), ComputerError> {
        let ids: Vec<ComputerId> = self.held()?.workers.keys().copied().collect();
        let mut first = None;
        for id in ids {
            if let Err(error) = self.destroy(id).await {
                first.get_or_insert(error);
            }
        }
        first.map_or(Ok(()), Err)
    }

    fn held(&self) -> Result<MutexGuard<'_, Held>, ComputerError> {
        self.held
            .lock()
            .map_err(|_| ComputerError::Failed("the worker table's lock is poisoned".into()))
    }

    fn worker(&self, computer: ComputerId) -> Result<Arc<Worker>, ComputerError> {
        self.held()?
            .workers
            .get(&computer)
            .cloned()
            .ok_or_else(|| ComputerError::NotFound(format!("no worker {computer}")))
    }

    fn worker_of(&self, workspace: WorkspaceId) -> Result<Arc<Worker>, ComputerError> {
        let computer = self
            .held()?
            .workspaces
            .get(&workspace)
            .copied()
            .ok_or_else(|| ComputerError::NotFound(format!("no workspace {workspace}")))?;
        self.worker(computer)
    }

    /// Refuses new work on a worker past its run time; saving and releasing still work.
    fn check_time(&self, worker: &Worker) -> Result<Option<Duration>, ComputerError> {
        let Some(deadline) = worker.deadline else {
            return Ok(None);
        };
        match deadline.duration_since(SystemTime::from(self.clock.now())) {
            Ok(left) if !left.is_zero() => Ok(Some(left)),
            _ => Err(ComputerError::Refused(
                format!(
                    "this worker has run its {} minutes ([workers] max_minutes); its work is kept \
                 when it is released",
                    worker.minutes.unwrap_or_default()
                )
                .into(),
            )),
        }
    }

    async fn create_now(&self, environment: &Environment) -> Result<ComputerInfo, ComputerError> {
        let running = self.held()?.workers.len();
        let decided = limits::decide(environment, &self.limits, running)?;
        if self.backend.isolation() < self.limits.isolation {
            return Err(ComputerError::Refused(Refusal::NeedsIsolation.into()));
        }
        let now = SystemTime::from(self.clock.now());
        // The worker checks its token against its own clock, so the token keeps real time.
        let token = Token::mint(SystemTime::from(SystemClock.now()), TOKEN_LIFETIME)?;
        let spec = Spec {
            image: decided.image,
            env: environment.env.clone(),
            cpus: decided.cpus,
            memory_gb: decided.memory_gb,
            network: decided.network,
            identity: Identity::mint()?,
            token: token.clone(),
            starter: self.starter().await,
        };
        let started = self.backend.start(&spec).await?;
        let name = started.name.clone();
        match self.reach(started, &token, environment).await {
            Ok((connection, link_task, info)) => {
                let deadline = decided.max_minutes.and_then(|minutes| {
                    now.checked_add(Duration::from_secs(u64::from(minutes).saturating_mul(60)))
                });
                let worker = Worker {
                    name,
                    connection,
                    environment: environment.clone(),
                    deadline,
                    minutes: decided.max_minutes,
                    network: decided.network,
                    set_up: tokio::sync::Mutex::new(environment.setup.is_empty()),
                    link_task,
                };
                self.held()?.workers.insert(info.id, Arc::new(worker));
                Ok(info)
            }
            Err(error) => {
                let _ = self.backend.remove(&name).await;
                Err(error)
            }
        }
    }

    /// Connects to a started worker and makes its one computer: the container itself.
    async fn reach(
        &self,
        started: Started,
        token: &Token,
        environment: &Environment,
    ) -> Result<(Connection, Option<JoinHandle<()>>, ComputerInfo), ComputerError> {
        let (remote, task, address) = match started.link {
            Link::Tls {
                address,
                fingerprint,
                forward,
            } => {
                // Tried again until a minute from now, however long each try takes.
                let until = tokio::time::Instant::now() + START_WAIT;
                let remote = loop {
                    match connect_tls(address, &fingerprint, token.secret()).await {
                        Ok(remote) => break remote,
                        Err(ComputerError::Unreachable(why))
                            if (why.starts_with(NOT_LISTENING)
                                || why.starts_with(NO_HANDSHAKE))
                                && tokio::time::Instant::now() < until =>
                        {
                            tokio::time::sleep(RETRY).await;
                        }
                        Err(error) => {
                            if let Some(forward) = &forward {
                                forward.abort();
                            }
                            return Err(error);
                        }
                    }
                };
                (remote, forward, Some((address, fingerprint)))
            }
            Link::Stream { read, write, task } => {
                let remote = RemoteComputers::connect(read, write);
                if let Err(error) = remote.check_protocol().await {
                    task.abort();
                    return Err(error);
                }
                (remote, Some(task), None)
            }
        };
        let inside = Environment {
            env: environment.env.clone(),
            ..Environment::host()
        };
        let mut info = remote.create(&inside).await?;
        info.environment = Some(environment.clone());
        Ok((Connection::new(remote, address), task, info))
    }

    /// The connection to `worker`, opened again (once) if it has died.
    async fn remote(&self, worker: &Worker) -> Result<Arc<RemoteComputers>, ComputerError> {
        worker.connection.get(&worker.name, &*self.backend).await
    }

    /// Runs the worker's setup commands once, in order, at the root of its first workspace.
    async fn set_up(&self, worker: &Worker, workspace: WorkspaceId) -> Result<(), ComputerError> {
        let mut done = worker.set_up.lock().await;
        if *done {
            return Ok(());
        }
        for script in &worker.environment.setup {
            let mut command = Command::new(script.clone());
            command.timeout_secs = Some(SETUP_TIMEOUT_SECS);
            let output = route::Tail::default();
            let finished = self
                .remote(worker)
                .await?
                .run(workspace, &command, &output, &NeverCancelled)
                .await?;
            if finished.ending != (Ending::Exited { code: 0 }) {
                return Err(ComputerError::Failed(format!(
                    "the setup command {script:?} did not succeed ({:?}): {}",
                    finished.ending,
                    output.text(SETUP_TAIL)
                )));
            }
        }
        *done = true;
        Ok(())
    }

    async fn open_now(
        &self,
        computer: ComputerId,
        sources: &Sources,
    ) -> Result<WorkspaceInfo, ComputerError> {
        let worker = self.worker(computer)?;
        self.check_time(&worker)?;
        let credential = match sources {
            Sources::Git { .. } if !worker.network => {
                return Err(ComputerError::Refused(Refusal::NoNetworkToClone.into()));
            }
            Sources::Git { url, .. } => self.credentials.for_url(url).await?,
            Sources::Folder { .. }
            | Sources::Worktree { .. }
            | Sources::Scratch { .. }
            | Sources::Project { .. } => None,
        };
        let info = match &credential {
            Some(credential) => {
                self.remote(&worker)
                    .await?
                    .open_with_credential(computer, sources, credential)
                    .await?
            }
            None => self.remote(&worker).await?.open(computer, sources).await?,
        };
        self.held()?.workspaces.insert(info.id, computer);
        if let Err(error) = self.set_up(&worker, info.id).await {
            let _ = self.close(info.id).await;
            return Err(error);
        }
        Ok(info)
    }

    async fn destroy_now(&self, computer: ComputerId) -> Result<(), ComputerError> {
        let worker = {
            let mut held = self.held()?;
            held.workspaces.retain(|_, on| *on != computer);
            held.workers.remove(&computer)
        };
        let Some(worker) = worker else {
            return Err(ComputerError::NotFound(format!("no worker {computer}")));
        };
        if let Some(task) = &worker.link_task {
            task.abort();
        }
        self.backend.remove(&worker.name).await
    }

    /// Runs `command` on its workspace's worker, within the worker's time, asking `credentials`
    /// for each credential its git or forge tools need, when given.
    async fn run_now(
        &self,
        workspace: WorkspaceId,
        command: &Command,
        output: &dyn Output,
        cancel: &dyn Cancellation,
        credentials: Option<&dyn glave_ports::computer::GitCredentials>,
    ) -> Result<glave_ports::computer::Finished, ComputerError> {
        let worker = self.worker_of(workspace)?;
        let left = self.check_time(&worker)?;
        let command = within(command, left);
        let remote = self.remote(&worker).await?;
        match credentials {
            Some(credentials) => {
                remote
                    .run_with_credentials(workspace, &command, output, cancel, credentials)
                    .await
            }
            None => remote.run(workspace, &command, output, cancel).await,
        }
    }
}

impl Computers for WorkerComputers {
    fn create<'a>(
        &'a self,
        environment: &'a Environment,
    ) -> BoxFuture<'a, Result<ComputerInfo, ComputerError>> {
        Box::pin(self.create_now(environment))
    }

    fn status(&self, computer: ComputerId) -> BoxFuture<'_, Result<ComputerInfo, ComputerError>> {
        Box::pin(async move {
            match self.worker(computer) {
                Ok(worker) => self.remote(&worker).await?.status(computer).await,
                Err(_) => Ok(route::gone(computer)),
            }
        })
    }

    fn destroy(&self, computer: ComputerId) -> BoxFuture<'_, Result<(), ComputerError>> {
        Box::pin(self.destroy_now(computer))
    }

    fn open<'a>(
        &'a self,
        computer: ComputerId,
        sources: &'a Sources,
    ) -> BoxFuture<'a, Result<WorkspaceInfo, ComputerError>> {
        Box::pin(self.open_now(computer, sources))
    }

    fn close(&self, workspace: WorkspaceId) -> BoxFuture<'_, Result<(), ComputerError>> {
        Box::pin(async move {
            let worker = self.worker_of(workspace)?;
            self.remote(&worker).await?.close(workspace).await?;
            self.held()?.workspaces.remove(&workspace);
            Ok(())
        })
    }

    fn run<'a>(
        &'a self,
        workspace: WorkspaceId,
        command: &'a Command,
        output: &'a dyn Output,
        cancel: &'a dyn Cancellation,
    ) -> BoxFuture<'a, Result<glave_ports::computer::Finished, ComputerError>> {
        Box::pin(self.run_now(workspace, command, output, cancel, None))
    }

    fn run_with_credentials<'a>(
        &'a self,
        workspace: WorkspaceId,
        command: &'a Command,
        output: &'a dyn Output,
        cancel: &'a dyn Cancellation,
        credentials: &'a dyn glave_ports::computer::GitCredentials,
    ) -> BoxFuture<'a, Result<glave_ports::computer::Finished, ComputerError>> {
        Box::pin(self.run_now(workspace, command, output, cancel, Some(credentials)))
    }

    fn start<'a>(
        &'a self,
        workspace: WorkspaceId,
        command: &'a Command,
    ) -> BoxFuture<'a, Result<glave_ports::computer::CommandId, ComputerError>> {
        Box::pin(async move {
            let worker = self.worker_of(workspace)?;
            let command = within(command, self.check_time(&worker)?);
            self.remote(&worker).await?.start(workspace, &command).await
        })
    }

    route::forward!();
}

/// `command` within the worker's time `left`, when policy limits it: it never outlives that
/// time, in the foreground or the background, and one the worker's time ends stops then rather
/// than moving to the background.
fn within(command: &Command, left: Option<Duration>) -> Command {
    let mut command = command.clone();
    if let Some(left) = left {
        let most = left.as_secs().max(1);
        let asked = command.timeout_secs;
        command.timeout_secs = Some(asked.map_or(most, |asked| asked.min(most)));
        if asked.is_none_or(|asked| most < asked) {
            command.on_timeout = glave_ports::computer::OnTimeout::Stop;
        }
    }
    command
}

#[cfg(test)]
mod tests;
