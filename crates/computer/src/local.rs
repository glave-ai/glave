//! This machine as a computer: commands run here, files are read and written here, and
//! workspaces are folders here. Copies of repositories live under one base folder, one
//! folder per computer, so destroying a computer removes exactly its copies.

mod anchored;
mod background;
mod command;
mod credentials;
mod files;
mod finding;
pub(crate) mod git;
mod listing;
mod paths;
mod process;
mod project;
mod registry;
mod replace;
mod sandbox;
mod scratch;
mod walk;
pub(crate) mod worktree;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use glave_ports::secrets::without_secrets;
use glave_ports::{
    Background, BoxFuture, Bytes, Cancellation, Command, CommandId, ComputerError, ComputerId,
    ComputerInfo, Computers, Credentials, Environment, FileWrite, Finished, GitCredential, Glob,
    Globbed, Image, Listed, Listing, Network, Output, Printed, Save, Saved, Search, Searched, Size,
    Sources, Stat, State, WorkspaceId, WorkspaceInfo,
};

use background::Backgrounds;
use command::Keep;
pub(crate) use registry::new_id;
use registry::{Opened, Registry};

pub use sandbox::{sandbox_available, sandbox_helper};

/// This machine's computers.
#[derive(Debug)]
pub struct LocalComputers {
    base: PathBuf,
    /// Where threads' worktrees are kept, outside every repository; none means this machine
    /// opens no worktrees.
    worktrees: Option<PathBuf>,
    /// Worktrees made in a repository one at a time, by every computer here.
    worktree_locks: worktree::Locks,
    /// Where threads with no project get a scratch folder each; none means this machine
    /// makes none.
    scratch: Option<PathBuf>,
    /// Where new projects' folders are made; none means this machine makes none.
    projects: Option<PathBuf>,
    /// The id this machine goes by as the host computer, kept across runs; a new one each
    /// time when none.
    host: Option<ComputerId>,
    /// A program that runs [`crate::sandbox_helper`], for Linux's Landlock fallback.
    sandbox_helper: Option<PathBuf>,
    environment: Arc<Vec<(OsString, OsString)>>,
    registry: Registry,
    /// The commands its workspaces keep running in the background.
    background: Backgrounds,
}

impl LocalComputers {
    /// Computers on this machine, keeping repository copies under `base`. Commands run with
    /// `environment` (usually glave's own) less anything named like a secret, such as the
    /// model provider's API key, plus each computer's environment variables.
    pub fn new(
        base: impl Into<PathBuf>,
        environment: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> Self {
        Self {
            base: base.into(),
            worktrees: None,
            worktree_locks: worktree::Locks::default(),
            scratch: None,
            projects: None,
            host: None,
            sandbox_helper: None,
            environment: Arc::new(without_secrets(environment)),
            registry: Registry::default(),
            background: Backgrounds::default(),
        }
    }

    /// This machine as a computer, with `folder` open on it in place: where the person's own
    /// thread works, and what the CLI, the session server and evals run on by default.
    /// Commands run with `environment`, less its secrets.
    pub async fn here(
        folder: impl AsRef<Path>,
        environment: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> Result<(Arc<Self>, WorkspaceInfo), ComputerError> {
        Self::for_here(environment).open_here(folder).await
    }

    /// This machine's computers as [`LocalComputers::here`] makes them, before a folder is
    /// opened: to choose where worktrees are kept first.
    pub fn for_here(environment: impl IntoIterator<Item = (OsString, OsString)>) -> Self {
        Self::new(std::env::temp_dir().join("glave-workspaces"), environment)
    }

    /// Keeps threads' worktrees under `folder`, one folder per repository, so a computer here
    /// can open [`Sources::Worktree`]. They outlive the computer: removing them is the
    /// person's choice.
    #[must_use]
    pub fn with_worktrees(mut self, folder: impl Into<PathBuf>) -> Self {
        self.worktrees = Some(folder.into());
        self
    }

    /// Makes threads' scratch folders under `folder`, so a computer here can open
    /// [`Sources::Scratch`]. Like worktrees, they outlive the computer.
    #[must_use]
    pub fn with_scratch(mut self, folder: impl Into<PathBuf>) -> Self {
        self.scratch = Some(folder.into());
        self
    }

    /// Makes new projects' folders under `folder`, so a computer here can open
    /// [`Sources::Project`]. They are the person's: nothing glave does removes them.
    #[must_use]
    pub fn with_projects(mut self, folder: impl Into<PathBuf>) -> Self {
        self.projects = Some(folder.into());
        self
    }

    /// Opens this machine as the host computer under `id`, one kept across runs, so whatever
    /// names a computer (a thread's binding, a project's checkout) names this one the same way
    /// each time.
    #[must_use]
    pub fn known_as(mut self, id: ComputerId) -> Self {
        self.host = Some(id);
        self
    }

    /// Names `program` as the sandbox's helper: a program (usually this one, from
    /// `std::env::current_exe`) that calls [`crate::sandbox_helper`] first thing in `main`.
    /// Linux needs it to confine `read-only` commands with Landlock where bubblewrap cannot
    /// start; without it, such a machine refuses sandboxed commands.
    #[must_use]
    pub fn with_sandbox_helper(mut self, program: impl Into<PathBuf>) -> Self {
        self.sandbox_helper = Some(program.into());
        self
    }

    /// A host computer made from these computers, with `folder` open on it in place.
    pub async fn open_here(
        self,
        folder: impl AsRef<Path>,
    ) -> Result<(Arc<Self>, WorkspaceInfo), ComputerError> {
        let computers = Arc::new(self);
        let computer = match computers.host {
            Some(id) => {
                computers.registry.add_computer(id, Environment::host())?;
                id
            }
            None => computers.create(&Environment::host()).await?.id,
        };
        let sources = Sources::Folder {
            path: folder.as_ref().to_string_lossy().into_owned(),
        };
        let workspace = computers.open(computer, &sources).await?;
        Ok((computers, workspace))
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
        credential: Option<&GitCredential>,
    ) -> Result<WorkspaceInfo, ComputerError> {
        if let Some(credential) = credential {
            credential.check()?;
        }
        let environment = self
            .registry
            .environment(computer)?
            .ok_or_else(|| ComputerError::NotFound(format!("no computer {computer}")))?;
        let id = WorkspaceId::from_uuid(registry::new_id());
        let (root, copy, git_common) = self
            .made(computer, id, &environment, sources, credential)
            .await?;
        let environment = self.command_environment(&environment);
        let variable = |name: &str| {
            environment
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.to_string_lossy().into_owned())
        };
        let info = WorkspaceInfo {
            id,
            computer,
            root: root.to_string_lossy().into_owned(),
            sources: sources.clone(),
            home: variable("HOME"),
            temp: variable("TMPDIR"),
        };
        let opened = Opened {
            computer,
            root,
            copy,
            git_common,
            environment,
            credential: credential.cloned(),
            prepared: Mutex::new(None),
        };
        if let Err(error) = self.registry.add_workspace(id, opened) {
            if copy {
                let _ = tokio::fs::remove_dir_all(&info.root).await;
            }
            return Err(error);
        }
        Ok(info)
    }

    /// Where `sources` open as the workspace `id` on `computer`: its root, whether it is a
    /// copy glave removes on close, and a worktree's shared git folder.
    async fn made(
        &self,
        computer: ComputerId,
        id: WorkspaceId,
        environment: &Environment,
        sources: &Sources,
        credential: Option<&GitCredential>,
    ) -> Result<(PathBuf, bool, Option<PathBuf>), ComputerError> {
        Ok(match sources {
            Sources::Folder { path } => (paths::folder(path).await?, false, None),
            Sources::Git { url, at } => {
                let dest = self.base.join(computer.to_string()).join(id.to_string());
                let env = self.command_environment(environment);
                git::clone(&env, url, at.as_deref(), &dest, credential).await?;
                (paths::folder(&dest.to_string_lossy()).await?, true, None)
            }
            Sources::Worktree {
                path,
                branch,
                owner,
            } => {
                let base = self.worktrees.as_ref().ok_or_else(|| {
                    ComputerError::Unsupported(
                        "this computer keeps no worktrees; work in the folder itself".into(),
                    )
                })?;
                let env = self.command_environment(environment);
                let locks = &self.worktree_locks;
                let made =
                    worktree::open(&env, locks, base, path, branch, owner.as_deref()).await?;
                (made.root, false, Some(made.common))
            }
            Sources::Scratch { name, owner } => {
                let base = self.scratch.as_ref().ok_or_else(|| {
                    ComputerError::Unsupported(
                        "this computer makes no scratch folders; work in a project folder".into(),
                    )
                })?;
                (
                    scratch::open(base, name, owner.as_deref()).await?,
                    false,
                    None,
                )
            }
            Sources::Project { name } => {
                let base = self.projects.as_ref().ok_or_else(|| {
                    ComputerError::Unsupported(
                        "this computer makes no folders for new projects; make the project from \
                         a folder already on it"
                            .into(),
                    )
                })?;
                (project::make(base, name).await?, false, None)
            }
        })
    }

    /// Where a command in `workspace` that reaches its timeout is kept running, if it asks.
    fn keep(&self, workspace: WorkspaceId) -> Keep<'_> {
        Keep {
            background: &self.background,
            workspace,
        }
    }

    /// Starts `command` in `workspace` and keeps it running in the background.
    async fn start_now(
        &self,
        workspace: WorkspaceId,
        command: &Command,
    ) -> Result<CommandId, ComputerError> {
        let opened = self.registry.workspace(workspace)?;
        self.background.room(workspace)?;
        let helper = self.sandbox_helper.as_deref();
        let sandbox = command.sandbox.as_ref();
        let spawned = command::spawn(&opened, &command.script, sandbox, helper, None).await?;
        let limit = command.timeout_secs.map(Duration::from_secs);
        let script = command.script.clone();
        match (self.background).adopt(opened.computer, workspace, script, spawned, limit) {
            Ok(id) => Ok(id),
            // Another start took the last place meanwhile.
            Err(mut spawned) => {
                process::stop(&mut spawned.child).await;
                Err(self
                    .background
                    .room(workspace)
                    .err()
                    .unwrap_or_else(|| ComputerError::Refused("there is no room for it".into())))
            }
        }
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
            self.background.stop_all(|on, _| on == computer).await;
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
        Box::pin(self.open_now(computer, sources, None))
    }

    fn open_with_credential<'a>(
        &'a self,
        computer: ComputerId,
        sources: &'a Sources,
        credential: &'a GitCredential,
    ) -> BoxFuture<'a, Result<WorkspaceInfo, ComputerError>> {
        Box::pin(self.open_now(computer, sources, Some(credential)))
    }

    fn close(&self, workspace: WorkspaceId) -> BoxFuture<'_, Result<(), ComputerError>> {
        Box::pin(async move {
            let opened = self.registry.remove_workspace(workspace)?;
            self.background.stop_all(|_, at| at == workspace).await;
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
            let helper = self.sandbox_helper.as_deref();
            let keep = self.keep(workspace);
            command::run(&opened, command, (output, cancel), helper, None, keep).await
        })
    }

    fn run_with_credentials<'a>(
        &'a self,
        workspace: WorkspaceId,
        command: &'a Command,
        output: &'a dyn Output,
        cancel: &'a dyn Cancellation,
        credentials: &'a dyn Credentials,
    ) -> BoxFuture<'a, Result<Finished, ComputerError>> {
        Box::pin(async move {
            let opened = self.registry.workspace(workspace)?;
            let helper = self.sandbox_helper.as_deref();
            let keep = self.keep(workspace);
            let io = (output, cancel);
            command::run(&opened, command, io, helper, Some(credentials), keep).await
        })
    }

    fn start<'a>(
        &'a self,
        workspace: WorkspaceId,
        command: &'a Command,
    ) -> BoxFuture<'a, Result<CommandId, ComputerError>> {
        Box::pin(self.start_now(workspace, command))
    }

    fn output(
        &self,
        workspace: WorkspaceId,
        command: CommandId,
        wait: Duration,
    ) -> BoxFuture<'_, Result<Printed, ComputerError>> {
        Box::pin(async move {
            self.registry.workspace(workspace)?;
            self.background.output(workspace, command, wait).await
        })
    }

    fn stop(
        &self,
        workspace: WorkspaceId,
        command: CommandId,
    ) -> BoxFuture<'_, Result<Printed, ComputerError>> {
        Box::pin(async move {
            self.registry.workspace(workspace)?;
            self.background.stop(workspace, command).await
        })
    }

    fn commands(
        &self,
        workspace: WorkspaceId,
    ) -> BoxFuture<'_, Result<Vec<Background>, ComputerError>> {
        Box::pin(async move {
            self.registry.workspace(workspace)?;
            Ok(self.background.list(workspace))
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
            listing::list(&opened.root, listing).await
        })
    }

    fn stat<'a>(
        &'a self,
        workspace: WorkspaceId,
        path: &'a str,
    ) -> BoxFuture<'a, Result<Stat, ComputerError>> {
        Box::pin(async move {
            let opened = self.registry.workspace(workspace)?;
            files::stat(&opened.root, path).await
        })
    }

    fn glob<'a>(
        &'a self,
        workspace: WorkspaceId,
        glob: &'a Glob,
    ) -> BoxFuture<'a, Result<Globbed, ComputerError>> {
        Box::pin(async move {
            let opened = self.registry.workspace(workspace)?;
            finding::glob(&opened.root, glob).await
        })
    }

    fn search<'a>(
        &'a self,
        workspace: WorkspaceId,
        search: &'a Search,
    ) -> BoxFuture<'a, Result<Searched, ComputerError>> {
        Box::pin(async move {
            let opened = self.registry.workspace(workspace)?;
            finding::search(&opened.root, search).await
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

#[cfg(test)]
mod confinement_tests;
