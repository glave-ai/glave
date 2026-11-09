//! Workers: computers of their own, started from a pinned image by a backend on Kubernetes,
//! with glave inside serving the computer protocol: a person's computer (`kubernetes/people`),
//! or a disposable one. The harness drives each one through this adapter, which keeps every
//! call on the right worker through bridges into its Pod, within policy's limits; destroying
//! it removes it. Each disposable worker is labelled with the glave that started it, and a
//! glave starting removes those whose glave is gone ([`WorkerComputers::reap`]).

mod backend;
mod kubernetes;
mod limits;
mod reconnect;
mod route;
mod tar;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use glave_ports::computer::{
    Command, ComputerError, ComputerId, ComputerInfo, Computers, Ending, Environment, Freed,
    KeptWorktree, Limits, Output, Refusal, Sources, WorkspaceId, WorkspaceInfo,
};
use glave_ports::{BoxFuture, Cancellation, Clock as _, NeverCancelled, SystemClock};
use glave_process::Glave;

use crate::remote::RemoteComputers;
use crate::token::Token;
use backend::{Backend, Spec, Started};
use reconnect::Connection;

pub use kubernetes::{Holding, Kubernetes, KubernetesSettings, Leasing, People, person_sandbox};

/// How long a worker's token opens connections: long enough for it to start.
const TOKEN_LIFETIME: Duration = Duration::from_mins(10);
/// The longest one setup command may run.
const SETUP_TIMEOUT_SECS: u64 = 30 * 60;
/// The most setup output kept for an error.
const SETUP_TAIL: usize = 4_000;

/// One worker as the harness side holds it.
struct Worker {
    name: String,
    connection: Connection,
    environment: Environment,
    /// Whether its commands may reach the network.
    network: bool,
    /// Whether its setup commands have run (they run in its first workspace).
    set_up: tokio::sync::Mutex<bool>,
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
    /// Workers on Kubernetes, within `limits`.
    pub fn kubernetes(kubernetes: Kubernetes, limits: Limits) -> Self {
        Self::on(Box::new(kubernetes), limits)
    }

    pub(crate) fn on(backend: Box<dyn Backend>, limits: Limits) -> Self {
        Self {
            backend,
            limits,
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

    async fn create_now(&self, environment: &Environment) -> Result<ComputerInfo, ComputerError> {
        let decided = limits::decide(environment, &self.limits)?;
        if self.backend.isolation() < self.limits.isolation {
            return Err(ComputerError::Refused(Refusal::NeedsIsolation.into()));
        }
        // The worker checks its token against its own clock, so the token keeps real time.
        let token = Token::mint(SystemTime::from(SystemClock.now()), TOKEN_LIFETIME)?;
        let spec = Spec {
            image: decided.image,
            env: environment.env.clone(),
            cpu_millis: decided.cpus.map(|cpus| cpus.saturating_mul(1_000)),
            memory_mib: decided.memory_gb.map(|gb| gb.saturating_mul(1_024)),
            network: decided.network,
            token: token.clone(),
            starter: self.starter().await,
        };
        let started = self.backend.start(&spec).await?;
        let name = started.name.clone();
        match self.reach(started, &token, environment).await {
            Ok((connection, info)) => {
                let worker = Worker {
                    name,
                    connection,
                    environment: environment.clone(),
                    network: decided.network,
                    set_up: tokio::sync::Mutex::new(environment.setup.is_empty()),
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

    /// Connects to a started worker through a bridge into it and makes its one computer: the
    /// container itself.
    async fn reach(
        &self,
        started: Started,
        token: &Token,
        environment: &Environment,
    ) -> Result<(Connection, ComputerInfo), ComputerError> {
        let remote = reconnect::bridged(&*self.backend, &started.name, token.secret()).await?;
        let inside = Environment {
            env: environment.env.clone(),
            ..Environment::host()
        };
        let mut info = remote.create(&inside).await?;
        info.environment = Some(environment.clone());
        Ok((Connection::new(remote), info))
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
        if matches!(sources, Sources::Git { .. }) && !worker.network {
            return Err(ComputerError::Refused(Refusal::NoNetworkToClone.into()));
        }
        let info = self.remote(&worker).await?.open(computer, sources).await?;
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
        self.backend.remove(&worker.name).await
    }

    /// Runs `command` on its workspace's worker, asking `credentials` for each credential its git
    /// or forge tools need, when given.
    async fn run_now(
        &self,
        workspace: WorkspaceId,
        command: &Command,
        output: &dyn Output,
        cancel: &dyn Cancellation,
        credentials: Option<&dyn glave_ports::computer::GitCredentials>,
    ) -> Result<glave_ports::computer::Finished, ComputerError> {
        let worker = self.worker_of(workspace)?;
        let remote = self.remote(&worker).await?;
        match credentials {
            Some(credentials) => {
                remote
                    .run_with_credentials(workspace, command, output, cancel, credentials)
                    .await
            }
            None => remote.run(workspace, command, output, cancel).await,
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

    fn worktrees(
        &self,
        computer: ComputerId,
    ) -> BoxFuture<'_, Result<Vec<KeptWorktree>, ComputerError>> {
        Box::pin(async move {
            let worker = self.worker(computer)?;
            self.remote(&worker).await?.worktrees(computer).await
        })
    }

    fn free_worktree<'a>(
        &'a self,
        computer: ComputerId,
        path: &'a str,
    ) -> BoxFuture<'a, Result<Freed, ComputerError>> {
        Box::pin(async move {
            let worker = self.worker(computer)?;
            let remote = self.remote(&worker).await?;
            remote.free_worktree(computer, path).await
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
            self.remote(&worker).await?.start(workspace, command).await
        })
    }

    route::forward!();
}

#[cfg(test)]
mod tests;
