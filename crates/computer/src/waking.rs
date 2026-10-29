//! A person's computer as the harness holds it while it may be asleep (`specs/computers.md`,
//! decision 6): a chat that needs no computer never wakes it. Its home and the scratch folders
//! opened from it are planned without asking the computer anything, at the roots that computer
//! puts them at; the first call that needs the computer (a command, a file read, written or
//! searched) wakes it, opens the workspace there and goes on. A computer that slept and woke
//! since opens it again. Anything else opened on it (a folder, a worktree, a clone) needs the
//! computer at once, and wakes it.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use glave_ports::computer::{
    Background, Bytes, Command, CommandId, ComputerError, ComputerId, ComputerInfo, Computers,
    Environment, FileWrite, Finished, GitCredential, GitCredentials, Glob, Globbed, Home, Listed,
    Listing, Output, Printed, Save, Saved, Search, Searched, Sources, Stat, WorkspaceId,
    WorkspaceInfo,
};
use glave_ports::{BoxFuture, Cancellation};

/// Wakes the computer, if it sleeps, and answers it awake with its home: what [`Waking`] asks
/// before each call that needs the computer.
pub type Wake = Arc<dyn Fn() -> BoxFuture<'static, Result<Home, ComputerError>> + Send + Sync>;

/// A person's computer that wakes only for the calls that need it.
pub struct Waking {
    wake: Wake,
    /// The computer as planned, and its home there.
    home: WorkspaceInfo,
    /// Where it makes scratch folders: each is this, `/`, and its name.
    scratch: String,
    /// Each workspace planned on it, and where it is open for real.
    workspaces: Mutex<BTreeMap<WorkspaceId, Arc<Planned>>>,
}

/// A workspace planned on the computer.
struct Planned {
    info: WorkspaceInfo,
    /// Where it is open, once a call needed it; opened again on a computer woken since.
    real: tokio::sync::Mutex<Option<Real>>,
}

/// A planned workspace, open on the computer awake.
#[derive(Clone)]
struct Real {
    computers: Arc<dyn Computers>,
    computer: ComputerId,
    workspace: WorkspaceId,
}

impl std::fmt::Debug for Waking {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Waking")
            .field("home", &self.home)
            .finish_non_exhaustive()
    }
}

impl Waking {
    /// The computer `wake` wakes, planned as `home`: its home's workspace, which names the
    /// computer, whose scratch folders are made in the folder `scratch`. Nothing is asked of it
    /// until a call needs it.
    pub fn home(wake: Wake, home: WorkspaceInfo, scratch: &str) -> Home {
        let waking = Self {
            wake,
            home: home.clone(),
            scratch: scratch.trim_end_matches('/').to_owned(),
            workspaces: Mutex::new(BTreeMap::new()),
        };
        waking.plan(home.clone(), None);
        Home {
            computers: Arc::new(waking),
            home,
        }
    }

    /// Keeps `info` as planned, open where `real` says when it is open already.
    fn plan(&self, info: WorkspaceInfo, real: Option<Real>) {
        let id = info.id;
        let planned = Arc::new(Planned {
            info,
            real: tokio::sync::Mutex::new(real),
        });
        self.workspaces
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, planned);
    }

    fn planned(&self, workspace: WorkspaceId) -> Result<Arc<Planned>, ComputerError> {
        self.workspaces
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&workspace)
            .map(Arc::clone)
            .ok_or_else(|| ComputerError::NotFound(format!("no workspace {workspace}")))
    }

    /// Where `workspace` is open, if a call has opened it; never wakes the computer.
    async fn opened(&self, workspace: WorkspaceId) -> Result<Option<Real>, ComputerError> {
        Ok(self.planned(workspace)?.real.lock().await.clone())
    }

    /// The computer awake, woken if it sleeps, and `workspace` open on it: opened there now
    /// when no call has opened it on the computer awake.
    async fn real(&self, workspace: WorkspaceId) -> Result<Real, ComputerError> {
        let planned = self.planned(workspace)?;
        let mut real = planned.real.lock().await;
        let awake = (self.wake)().await?;
        if let Some(open) = real.as_ref()
            && open.computer == awake.home.computer
        {
            return Ok(open.clone());
        }
        let info = if planned.info.sources == self.home.sources {
            awake.home.clone()
        } else {
            awake
                .computers
                .open(awake.home.computer, &planned.info.sources)
                .await?
        };
        if info.root != planned.info.root {
            return Err(ComputerError::Failed(format!(
                "your computer opened this folder at {}, not at {} as glave planned",
                info.root, planned.info.root
            )));
        }
        let open = Real {
            computers: awake.computers,
            computer: awake.home.computer,
            workspace: info.id,
        };
        *real = Some(open.clone());
        Ok(open)
    }
}

impl Waking {
    /// `opened`, a workspace opened on the computer awake, `home` there, kept as planned on
    /// `computer` under an id of its own.
    fn opened_now(
        &self,
        computer: ComputerId,
        home: Real,
        opened: WorkspaceInfo,
    ) -> Result<WorkspaceInfo, ComputerError> {
        let real = Real {
            workspace: opened.id,
            ..home
        };
        let info = WorkspaceInfo {
            id: WorkspaceId::from_uuid(crate::local::new_id()?),
            computer,
            ..opened
        };
        self.plan(info.clone(), Some(real));
        Ok(info)
    }
}

#[deny(clippy::missing_trait_methods)]
impl Computers for Waking {
    fn create<'a>(
        &'a self,
        _environment: &'a Environment,
    ) -> BoxFuture<'a, Result<ComputerInfo, ComputerError>> {
        Box::pin(std::future::ready(Err(ComputerError::Unsupported(
            "a person's computer is made with the person, not by a thread".into(),
        ))))
    }

    /// What it is, woken to say.
    fn status(&self, computer: ComputerId) -> BoxFuture<'_, Result<ComputerInfo, ComputerError>> {
        Box::pin(async move {
            let home = self.real(self.home.id).await?;
            let mut info = home.computers.status(home.computer).await?;
            info.id = computer;
            Ok(info)
        })
    }

    fn destroy(&self, _computer: ComputerId) -> BoxFuture<'_, Result<(), ComputerError>> {
        Box::pin(std::future::ready(Err(ComputerError::Unsupported(
            "a person's computer is removed with the person, not by a thread".into(),
        ))))
    }

    /// A scratch folder is planned at once, at the root the computer makes it at; anything
    /// else is opened on the computer, woken for it.
    fn open<'a>(
        &'a self,
        computer: ComputerId,
        sources: &'a Sources,
    ) -> BoxFuture<'a, Result<WorkspaceInfo, ComputerError>> {
        Box::pin(async move {
            if computer != self.home.computer {
                return Err(ComputerError::NotFound(format!("no computer {computer}")));
            }
            if let Sources::Scratch { name, .. } = sources {
                let info = WorkspaceInfo {
                    id: WorkspaceId::from_uuid(crate::local::new_id()?),
                    root: format!("{}/{name}", self.scratch),
                    sources: sources.clone(),
                    ..self.home.clone()
                };
                self.plan(info.clone(), None);
                return Ok(info);
            }
            let home = self.real(self.home.id).await?;
            let opened = home.computers.open(home.computer, sources).await?;
            self.opened_now(computer, home, opened)
        })
    }

    fn open_with_credential<'a>(
        &'a self,
        computer: ComputerId,
        sources: &'a Sources,
        credential: &'a GitCredential,
    ) -> BoxFuture<'a, Result<WorkspaceInfo, ComputerError>> {
        Box::pin(async move {
            if computer != self.home.computer {
                return Err(ComputerError::NotFound(format!("no computer {computer}")));
            }
            let home = self.real(self.home.id).await?;
            let opened = home
                .computers
                .open_with_credential(home.computer, sources, credential)
                .await?;
            self.opened_now(computer, home, opened)
        })
    }

    /// Closed where it is open, and forgotten; one never opened is only forgotten.
    fn close(&self, workspace: WorkspaceId) -> BoxFuture<'_, Result<(), ComputerError>> {
        Box::pin(async move {
            let open = self.opened(workspace).await?;
            self.workspaces
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&workspace);
            match open {
                Some(open) => open.computers.close(open.workspace).await,
                None => Ok(()),
            }
        })
    }

    fn run<'a>(
        &'a self,
        workspace: WorkspaceId,
        command: &'a Command,
        output: &'a dyn Output,
        stop: &'a dyn Cancellation,
    ) -> BoxFuture<'a, Result<Finished, ComputerError>> {
        Box::pin(async move {
            let open = self.real(workspace).await?;
            open.computers
                .run(open.workspace, command, output, stop)
                .await
        })
    }

    fn run_with_credentials<'a>(
        &'a self,
        workspace: WorkspaceId,
        command: &'a Command,
        output: &'a dyn Output,
        stop: &'a dyn Cancellation,
        credentials: &'a dyn GitCredentials,
    ) -> BoxFuture<'a, Result<Finished, ComputerError>> {
        Box::pin(async move {
            let open = self.real(workspace).await?;
            open.computers
                .run_with_credentials(open.workspace, command, output, stop, credentials)
                .await
        })
    }

    fn start<'a>(
        &'a self,
        workspace: WorkspaceId,
        command: &'a Command,
    ) -> BoxFuture<'a, Result<CommandId, ComputerError>> {
        Box::pin(async move {
            let open = self.real(workspace).await?;
            open.computers.start(open.workspace, command).await
        })
    }

    /// Commands in the background are only where the workspace is open: one never opened has
    /// none, and is not woken to say so.
    fn output(
        &self,
        workspace: WorkspaceId,
        command: CommandId,
        wait: Duration,
    ) -> BoxFuture<'_, Result<Printed, ComputerError>> {
        Box::pin(async move {
            match self.opened(workspace).await? {
                Some(open) => open.computers.output(open.workspace, command, wait).await,
                None => Err(ComputerError::NotFound(format!("no command {command}"))),
            }
        })
    }

    fn ended(
        &self,
        workspace: WorkspaceId,
        command: CommandId,
        wait: Duration,
    ) -> BoxFuture<'_, Result<Option<Finished>, ComputerError>> {
        Box::pin(async move {
            match self.opened(workspace).await? {
                Some(open) => open.computers.ended(open.workspace, command, wait).await,
                None => Err(ComputerError::NotFound(format!("no command {command}"))),
            }
        })
    }

    fn stop(
        &self,
        workspace: WorkspaceId,
        command: CommandId,
    ) -> BoxFuture<'_, Result<Printed, ComputerError>> {
        Box::pin(async move {
            match self.opened(workspace).await? {
                Some(open) => open.computers.stop(open.workspace, command).await,
                None => Err(ComputerError::NotFound(format!("no command {command}"))),
            }
        })
    }

    fn commands(
        &self,
        workspace: WorkspaceId,
    ) -> BoxFuture<'_, Result<Vec<Background>, ComputerError>> {
        Box::pin(async move {
            match self.opened(workspace).await? {
                Some(open) => open.computers.commands(open.workspace).await,
                None => Ok(Vec::new()),
            }
        })
    }

    fn read<'a>(
        &'a self,
        workspace: WorkspaceId,
        path: &'a str,
    ) -> BoxFuture<'a, Result<Bytes, ComputerError>> {
        Box::pin(async move {
            let open = self.real(workspace).await?;
            open.computers.read(open.workspace, path).await
        })
    }

    fn write<'a>(
        &'a self,
        workspace: WorkspaceId,
        files: &'a [FileWrite],
    ) -> BoxFuture<'a, Result<(), ComputerError>> {
        Box::pin(async move {
            let open = self.real(workspace).await?;
            open.computers.write(open.workspace, files).await
        })
    }

    fn list<'a>(
        &'a self,
        workspace: WorkspaceId,
        listing: &'a Listing,
    ) -> BoxFuture<'a, Result<Listed, ComputerError>> {
        Box::pin(async move {
            let open = self.real(workspace).await?;
            open.computers.list(open.workspace, listing).await
        })
    }

    fn stat<'a>(
        &'a self,
        workspace: WorkspaceId,
        path: &'a str,
    ) -> BoxFuture<'a, Result<Stat, ComputerError>> {
        Box::pin(async move {
            let open = self.real(workspace).await?;
            open.computers.stat(open.workspace, path).await
        })
    }

    fn glob<'a>(
        &'a self,
        workspace: WorkspaceId,
        glob: &'a Glob,
    ) -> BoxFuture<'a, Result<Globbed, ComputerError>> {
        Box::pin(async move {
            let open = self.real(workspace).await?;
            open.computers.glob(open.workspace, glob).await
        })
    }

    fn search<'a>(
        &'a self,
        workspace: WorkspaceId,
        search: &'a Search,
    ) -> BoxFuture<'a, Result<Searched, ComputerError>> {
        Box::pin(async move {
            let open = self.real(workspace).await?;
            open.computers.search(open.workspace, search).await
        })
    }

    fn save<'a>(
        &'a self,
        workspace: WorkspaceId,
        save: &'a Save,
    ) -> BoxFuture<'a, Result<Saved, ComputerError>> {
        Box::pin(async move {
            let open = self.real(workspace).await?;
            open.computers.save(open.workspace, save).await
        })
    }
}

#[cfg(test)]
mod tests;
