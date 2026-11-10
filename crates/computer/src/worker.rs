//! People's computers in the cloud (`kubernetes/people`): each a Pod on Kubernetes while awake,
//! started from a pinned image on the person's disk, with glave inside serving the computer
//! protocol. The harness drives one through this adapter, which keeps every call on the
//! computer awake through bridges into its Pod, within policy's limits; destroying it puts it
//! to sleep, its disk kept.

mod kubernetes;
mod limits;
mod reconnect;
mod route;
mod tar;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use glave_ports::computer::{
    Command, ComputerError, ComputerId, ComputerInfo, Computers, Environment, Freed, KeptWorktree,
    Limits, Output, Refusal, Sources, WorkspaceId, WorkspaceInfo,
};
use glave_ports::{BoxFuture, Cancellation, Clock as _, SystemClock};

use crate::remote::RemoteComputers;
use crate::token::Token;
use kubernetes::{PersonComputer, Spec};
use reconnect::Connection;

pub use kubernetes::{Holding, Kubernetes, KubernetesSettings, Leasing, People, person_sandbox};

/// How long a computer's token opens connections: long enough for it to start.
const TOKEN_LIFETIME: Duration = Duration::from_mins(10);

/// The computer awake, by the id glave in it gave, and the workspaces open on it.
#[derive(Default)]
struct Held {
    computers: HashMap<ComputerId, Arc<Connection>>,
    workspaces: HashMap<WorkspaceId, ComputerId>,
}

/// One person's computer as computers: `computer/create` wakes it and `computer/destroy` puts it
/// to sleep, its disk kept.
pub(crate) struct WorkerComputers {
    computer: PersonComputer,
    limits: Limits,
    held: Mutex<Held>,
}

impl std::fmt::Debug for WorkerComputers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerComputers")
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl WorkerComputers {
    /// `computer`, woken within `limits`.
    pub(crate) fn new(computer: PersonComputer, limits: Limits) -> Self {
        Self {
            computer,
            limits,
            held: Mutex::new(Held::default()),
        }
    }

    fn held(&self) -> Result<MutexGuard<'_, Held>, ComputerError> {
        self.held
            .lock()
            .map_err(|_| ComputerError::Failed("the computer table's lock is poisoned".into()))
    }

    /// The connection to `computer`, while it is awake.
    fn worker(&self, computer: ComputerId) -> Result<Arc<Connection>, ComputerError> {
        self.held()?
            .computers
            .get(&computer)
            .cloned()
            .ok_or_else(|| ComputerError::NotFound(format!("no computer {computer}")))
    }

    /// The connection to the computer `workspace` is open on.
    fn worker_of(&self, workspace: WorkspaceId) -> Result<Arc<Connection>, ComputerError> {
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
        if self.computer.isolation() < self.limits.isolation {
            return Err(ComputerError::Refused(Refusal::NeedsIsolation.into()));
        }
        // The computer checks its token against its own clock, so the token keeps real time.
        let token = Token::mint(SystemTime::from(SystemClock.now()), TOKEN_LIFETIME)?;
        let spec = Spec {
            image: decided.image,
            env: environment.env.clone(),
            cpu_millis: decided.cpus.map(|cpus| cpus.saturating_mul(1_000)),
            memory_mib: decided.memory_gb.map(|gb| gb.saturating_mul(1_024)),
            network: decided.network,
            token: token.clone(),
        };
        self.computer.wake(&spec).await?;
        match self.reach(&token, environment).await {
            Ok((connection, info)) => {
                self.held()?.computers.insert(info.id, Arc::new(connection));
                Ok(info)
            }
            Err(error) => {
                let _ = self.computer.sleep().await;
                Err(error)
            }
        }
    }

    /// Connects to the computer just woken through a bridge into it, signed in with `token`,
    /// and makes its one computer: the container itself.
    async fn reach(
        &self,
        token: &Token,
        environment: &Environment,
    ) -> Result<(Connection, ComputerInfo), ComputerError> {
        let remote = reconnect::bridged(&self.computer, token.secret()).await?;
        let inside = Environment {
            env: environment.env.clone(),
            ..Environment::host()
        };
        let mut info = remote.create(&inside).await?;
        info.environment = Some(environment.clone());
        Ok((Connection::new(remote), info))
    }

    /// The connection `worker`, opened again (once) if it has died.
    async fn remote(&self, worker: &Connection) -> Result<Arc<RemoteComputers>, ComputerError> {
        worker.get(&self.computer).await
    }

    async fn open_now(
        &self,
        computer: ComputerId,
        sources: &Sources,
    ) -> Result<WorkspaceInfo, ComputerError> {
        let worker = self.worker(computer)?;
        let info = self.remote(&worker).await?.open(computer, sources).await?;
        self.held()?.workspaces.insert(info.id, computer);
        Ok(info)
    }

    async fn destroy_now(&self, computer: ComputerId) -> Result<(), ComputerError> {
        let awake = {
            let mut held = self.held()?;
            held.workspaces.retain(|_, on| *on != computer);
            held.computers.remove(&computer)
        };
        if awake.is_none() {
            return Err(ComputerError::NotFound(format!("no computer {computer}")));
        }
        self.computer.sleep().await
    }

    /// Runs `command` in `workspace`, asking `credentials` for each credential its git or forge
    /// tools need, when given.
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
