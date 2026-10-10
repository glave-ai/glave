//! This machine as a computer: commands run here, files are read and written here, and
//! workspaces are folders here. Copies of repositories live under one base folder, one
//! folder per computer, so destroying a computer removes exactly its copies.

mod command;
mod files;
mod git;
mod paths;
mod registry;

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use glave_ports::{
    BoxFuture, Bytes, Cancellation, Command, ComputerError, ComputerId, ComputerInfo, Computers,
    Environment, FileWrite, Finished, Image, Listed, Listing, Network, Output, Save, Saved, Size,
    Sources, State, WorkspaceId, WorkspaceInfo,
};

use registry::{Opened, Registry};

/// This machine's computers.
#[derive(Debug)]
pub struct LocalComputers {
    base: PathBuf,
    environment: Arc<Vec<(OsString, OsString)>>,
    registry: Registry,
}

impl LocalComputers {
    /// Computers on this machine, keeping repository copies under `base`. Commands run with
    /// `environment`, which must already be free of secrets (glave's own environment, less
    /// anything named like a key or token), plus each computer's environment variables.
    pub fn new(base: impl Into<PathBuf>, environment: Vec<(OsString, OsString)>) -> Self {
        Self {
            base: base.into(),
            environment: Arc::new(environment),
            registry: Registry::default(),
        }
    }

    fn describe(id: ComputerId, environment: Option<Environment>) -> ComputerInfo {
        let cpus = std::thread::available_parallelism()
            .ok()
            .and_then(|count| u32::try_from(count.get()).ok());
        ComputerInfo {
            id,
            state: if environment.is_some() {
                State::Running
            } else {
                State::Gone
            },
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
            size: Size {
                cpus,
                ..Size::default()
            },
            environment,
        }
    }

    async fn open_now(
        &self,
        computer: ComputerId,
        sources: &Sources,
    ) -> Result<WorkspaceInfo, ComputerError> {
        let environment = self
            .registry
            .environment(computer)?
            .ok_or_else(|| ComputerError::NotFound(format!("no computer {computer}")))?;
        let id = WorkspaceId::from_uuid(registry::new_id());
        let (root, copy) = match sources {
            Sources::Folder { path } => (paths::folder(path).await?, false),
            Sources::Git { url, at } => {
                let dest = self.base.join(computer.to_string()).join(id.to_string());
                let env = self.command_environment(&environment);
                git::clone(&env, url, at.as_deref(), &dest).await?;
                (paths::folder(&dest.to_string_lossy()).await?, true)
            }
        };
        let info = WorkspaceInfo {
            id,
            computer,
            root: root.to_string_lossy().into_owned(),
            sources: sources.clone(),
        };
        let opened = Opened {
            computer,
            root,
            copy,
            environment: self.command_environment(&environment),
        };
        if let Err(error) = self.registry.add_workspace(id, opened) {
            if copy {
                let _ = tokio::fs::remove_dir_all(&info.root).await;
            }
            return Err(error);
        }
        Ok(info)
    }

    /// What commands on a computer see: this machine's environment, then the computer's own.
    fn command_environment(&self, environment: &Environment) -> Arc<Vec<(OsString, OsString)>> {
        let mut all: Vec<(OsString, OsString)> = self
            .environment
            .iter()
            .filter(|(name, _)| !environment.env.contains_key(&*name.to_string_lossy()))
            .cloned()
            .collect();
        all.extend(
            environment
                .env
                .iter()
                .map(|(name, value)| (name.into(), value.into())),
        );
        Arc::new(all)
    }
}

/// Refuses what a computer on the host cannot honour, rather than quietly ignoring it.
fn check_host(environment: &Environment) -> Result<(), ComputerError> {
    if let Image::Oci(image) = &environment.image {
        return Err(ComputerError::Unsupported(format!(
            "this machine runs work on the host itself and cannot run the image {image}; \
             computers made from images come with cloud computers"
        )));
    }
    if !environment.setup.is_empty() {
        return Err(ComputerError::Unsupported(
            "setup commands are cached in a snapshot of an image; on the host, run them \
             yourself"
                .into(),
        ));
    }
    if matches!(environment.network, Some(Network::Off | Network::Allowlist)) {
        return Err(ComputerError::Unsupported(
            "the host cannot limit the network per computer; use the sandbox, or a computer \
             made from an image"
                .into(),
        ));
    }
    Ok(())
}

impl Computers for LocalComputers {
    fn create<'a>(
        &'a self,
        environment: &'a Environment,
    ) -> BoxFuture<'a, Result<ComputerInfo, ComputerError>> {
        Box::pin(async move {
            check_host(environment)?;
            let id = ComputerId::from_uuid(registry::new_id());
            self.registry.add_computer(id, environment.clone())?;
            Ok(Self::describe(id, Some(environment.clone())))
        })
    }

    fn status(&self, computer: ComputerId) -> BoxFuture<'_, Result<ComputerInfo, ComputerError>> {
        Box::pin(async move {
            let environment = self.registry.environment(computer)?;
            Ok(Self::describe(computer, environment))
        })
    }

    fn destroy(&self, computer: ComputerId) -> BoxFuture<'_, Result<(), ComputerError>> {
        Box::pin(async move {
            self.registry.remove_computer(computer)?;
            let copies = self.base.join(computer.to_string());
            match tokio::fs::remove_dir_all(&copies).await {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(ComputerError::Failed(format!(
                    "could not remove {}: {error}",
                    copies.display()
                ))),
            }
        })
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
            let opened = self.registry.remove_workspace(workspace)?;
            if opened.copy {
                tokio::fs::remove_dir_all(&opened.root)
                    .await
                    .map_err(|error| {
                        ComputerError::Failed(format!(
                            "could not remove {}: {error}",
                            opened.root.display()
                        ))
                    })?;
            }
            Ok(())
        })
    }

    fn run<'a>(
        &'a self,
        workspace: WorkspaceId,
        command: &'a Command,
        output: &'a dyn Output,
        cancel: &'a dyn Cancellation,
    ) -> BoxFuture<'a, Result<Finished, ComputerError>> {
        Box::pin(async move {
            let opened = self.registry.workspace(workspace)?;
            command::run(&opened, command, output, cancel).await
        })
    }

    fn read<'a>(
        &'a self,
        workspace: WorkspaceId,
        path: &'a str,
    ) -> BoxFuture<'a, Result<Bytes, ComputerError>> {
        Box::pin(async move {
            let opened = self.registry.workspace(workspace)?;
            files::read(&opened.root, path).await
        })
    }

    fn write<'a>(
        &'a self,
        workspace: WorkspaceId,
        files: &'a [FileWrite],
    ) -> BoxFuture<'a, Result<(), ComputerError>> {
        Box::pin(async move {
            let opened = self.registry.workspace(workspace)?;
            files::write(&opened.root, files).await
        })
    }

    fn list<'a>(
        &'a self,
        workspace: WorkspaceId,
        listing: &'a Listing,
    ) -> BoxFuture<'a, Result<Listed, ComputerError>> {
        Box::pin(async move {
            let opened = self.registry.workspace(workspace)?;
            files::list(&opened.root, listing).await
        })
    }

    fn save<'a>(
        &'a self,
        workspace: WorkspaceId,
        save: &'a Save,
    ) -> BoxFuture<'a, Result<Saved, ComputerError>> {
        Box::pin(async move {
            let opened = self.registry.workspace(workspace)?;
            git::save(&opened, save).await
        })
    }
}

#[cfg(test)]
mod tests;
