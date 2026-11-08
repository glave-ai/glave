//! This machine as a computer: commands run here, files are read and written here, and
//! workspaces are folders here. Copies of repositories live under one base folder, one
//! folder per computer, so destroying a computer removes exactly its copies.

mod anchored;
mod background;
mod changes;
mod command;
mod credentials;
mod files;
mod finding;
pub(crate) mod git;
mod host;
mod listing;
mod on_path;
mod paths;
mod pressure;
mod process;
mod project;
mod registry;
mod replace;
mod sandbox;
mod scratch;
mod setup;
mod terminal;
mod walk;
pub(crate) mod worktree;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use glave_ports::computer::{
    Background, Bytes, Changes, Command, CommandId, ComputerError, ComputerId, ComputerInfo,
    Computers, DiskSpace, Environment, FileWrite, Finished, Freed, GitCredential, GitCredentials,
    Glob, Globbed, KeptWorktree, Listed, Listening, Listing, Output, Printed, Refusal, Save, Saved,
    Screenful, Search, Searched, Size, Sources, Stat, State, TerminalId, TerminalInfo,
    TerminalSize, WorkspaceId, WorkspaceInfo,
};
use glave_ports::secrets::without_secrets;
use glave_ports::{BoxFuture, Cancellation};
use glave_process::{Owner, Processes};

use background::Backgrounds;
use command::Keep;
pub(crate) use registry::new_id;
use registry::{Opened, Pushes, Registry};

use crate::disk::Largest;
pub use git::ask_repository;
pub use on_path::glave_on_path;
pub(crate) use sandbox::CACHE_FOLDER;
pub use sandbox::{sandbox_available, sandbox_helper};
use terminal::Terminals;
pub use terminal::terminal_helper;

/// What measures a disk in place of the system.
#[derive(Clone)]
struct Measure(Arc<dyn Fn() -> DiskSpace + Send + Sync>);

impl std::fmt::Debug for Measure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Measure")
    }
}

/// This machine's computers.
#[derive(Debug)]
pub struct LocalComputers {
    base: PathBuf,
    /// What measures its disk in place of the system, for a test that can't fill one.
    measure: Option<Measure>,
    /// What uses its disk most, as its keeper last looked.
    largest: Largest,
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
    /// glave's own program, which runs [`crate::sandbox_helper`] (Linux's Landlock fallback)
    /// and [`crate::terminal_helper`] (a terminal's shell, in a session of its own).
    helper: Option<PathBuf>,
    /// The person's terminals open here.
    terminals: Terminals,
    /// Places on this machine its sandboxed commands may not read, beside the home folder's
    /// credential stores.
    hidden: Arc<[PathBuf]>,
    /// A folder of glave's own holding `glave`, first on every command's `PATH` and readable in
    /// every sandbox, so a command reaches the glave that runs it whatever else is installed.
    glave_on_path: Option<PathBuf>,
    environment: Arc<Vec<(OsString, OsString)>>,
    registry: Registry,
    /// The commands its workspaces keep running in the background.
    background: Backgrounds,
    /// Where its commands are recorded while they run, each owned by its workspace.
    processes: Processes,
    /// The cgroup it runs in, where Linux counts the CPU its commands use and sets the limit on
    /// it.
    cgroup: Option<PathBuf>,
}

/// What a folder is opened as, which says whether to ask git where its history stands (`at`).
#[derive(Debug, Clone, Copy)]
enum Opening {
    /// Where a thread is placed, which its binding records (`workspace.bound`'s `since`).
    Placed,
    /// This machine's own folder, which no thread records.
    Own,
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
            measure: None,
            worktree_locks: worktree::Locks::default(),
            scratch: None,
            projects: None,
            host: None,
            helper: None,
            terminals: Terminals::default(),
            hidden: Arc::new([]),
            glave_on_path: None,
            environment: Arc::new(without_secrets(environment)),
            registry: Registry::default(),
            background: Backgrounds::default(),
            cgroup: pressure::cgroup(),
            processes: Processes::unrecorded(),
            largest: Largest::default(),
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

    /// A host computer made from these computers, with `folder` open on it in place. Where the
    /// folder's history stands is not asked (`at` is none): no thread records this workspace as
    /// the one it was placed in, since a thread placed in the folder opens it again, and asking
    /// would run git once or twice at every start, before a thread is read.
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
        let workspace = computers
            .open_now(computer, &sources, None, Opening::Own)
            .await?;
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
            disk: None,
            largest: Vec::new(),
        }
    }

    /// What its disk holds and has free, where it keeps worktrees: the disk those are on.
    fn disk_space(&self) -> Option<DiskSpace> {
        if let Some(measure) = &self.measure {
            return Some((measure.0)());
        }
        let folder = self.worktrees.as_deref()?;
        let there = folder.ancestors().find(|path| path.exists())?;
        crate::disk::space(there).ok()
    }

    async fn open_now(
        &self,
        computer: ComputerId,
        sources: &Sources,
        credential: Option<&GitCredential>,
        opening: Opening,
    ) -> Result<WorkspaceInfo, ComputerError> {
        if let Some(credential) = credential {
            credential.check()?;
        }
        let environment = self
            .registry
            .environment(computer)?
            .ok_or_else(|| ComputerError::NotFound(format!("no computer {computer}")))?;
        let id = WorkspaceId::from_uuid(new_id()?);
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
            at: None,
        };
        let opened = Opened {
            computer,
            root,
            copy,
            git_common,
            environment,
            hidden: Arc::clone(&self.hidden),
            glave_on_path: self.glave_on_path.clone(),
            pushes: Pushes::of(sources, credential),
            prepared: Mutex::new(None),
            processes: self.processes.owned_by(Owner::Workspace(id.to_string())),
        };
        let info = WorkspaceInfo {
            at: match opening {
                Opening::Placed => changes::at(&opened).await,
                Opening::Own => None,
            },
            ..info
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
                git::clone(&self.processes, &env, url, at.as_deref(), &dest, credential).await?;
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
                let git = (&self.processes, env.as_slice());
                let made = worktree::open(git, locks, base, path, branch, owner.as_deref()).await?;
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
            cgroup: self.cgroup.as_deref(),
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
        let id = CommandId::from_uuid(new_id()?);
        let helper = self.helper.as_deref();
        let spawned = command::spawn(&opened, command, helper, None).await?;
        let limit = command.timeout_secs.map(Duration::from_secs);
        let named = (id, command.script.clone());
        let at = (opened.computer, workspace);
        match self.background.adopt(named, at, spawned, limit) {
            Ok(id) => Ok(id),
            // Another start took the last place meanwhile.
            Err(mut spawned) => {
                spawned.child.stop().await;
                Err(self
                    .background
                    .room(workspace)
                    .err()
                    .unwrap_or_else(|| ComputerError::Refused(Refusal::NoRoom.into())))
            }
        }
    }

    /// What commands on a computer see: this machine's environment, then the computer's own,
    /// with glave's own folder first on `PATH` when it has one.
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
        if let Some(folder) = &self.glave_on_path {
            let path = first_on_path(&all, folder);
            all.retain(|(name, _)| name != "PATH");
            all.push(("PATH".into(), path));
        }
        Arc::new(all)
    }
}

/// `environment`'s `PATH` with `folder` first.
pub(crate) fn first_on_path(environment: &[(OsString, OsString)], folder: &Path) -> OsString {
    let mut first = folder.as_os_str().to_owned();
    if let Some((_, path)) = environment.iter().find(|(name, _)| name == "PATH") {
        first.push(":");
        first.push(path);
    }
    first
}

impl Computers for LocalComputers {
    fn create<'a>(
        &'a self,
        environment: &'a Environment,
    ) -> BoxFuture<'a, Result<ComputerInfo, ComputerError>> {
        Box::pin(async move {
            host::check(environment)?;
            let id = ComputerId::from_uuid(new_id()?);
            self.registry.add_computer(id, environment.clone())?;
            Ok(Self::describe(id, Some(environment.clone())))
        })
    }

    fn status(&self, computer: ComputerId) -> BoxFuture<'_, Result<ComputerInfo, ComputerError>> {
        Box::pin(async move {
            let environment = self.registry.environment(computer)?;
            let mut info = Self::describe(computer, environment);
            info.disk = self.disk_space();
            info.largest = self.largest.get();
            Ok(info)
        })
    }

    fn open_terminal<'a>(
        &'a self,
        computer: ComputerId,
        folder: &'a str,
        owner: &'a str,
        size: TerminalSize,
    ) -> BoxFuture<'a, Result<TerminalId, ComputerError>> {
        Box::pin(async move {
            self.registry.environment(computer)?;
            let start = terminal::Start {
                helper: self.helper.as_deref(),
                processes: &self.processes,
                environment: &self.environment,
            };
            let at = (folder, owner);
            self.terminals.open(computer, at, size, &start).await
        })
    }

    fn terminals<'a>(
        &'a self,
        computer: ComputerId,
        owner: &'a str,
    ) -> BoxFuture<'a, Result<Vec<TerminalInfo>, ComputerError>> {
        Box::pin(std::future::ready(Ok(self.terminals.list(computer, owner))))
    }

    fn write_terminal<'a>(
        &'a self,
        computer: ComputerId,
        at: (TerminalId, &'a str),
        input: &'a Bytes,
    ) -> BoxFuture<'a, Result<(), ComputerError>> {
        Box::pin(self.terminals.write(computer, at, input))
    }

    fn read_terminal<'a>(
        &'a self,
        computer: ComputerId,
        at: (TerminalId, &'a str),
        from: u64,
        wait: Duration,
    ) -> BoxFuture<'a, Result<Screenful, ComputerError>> {
        Box::pin(self.terminals.read(computer, at, from, wait))
    }

    fn resize_terminal<'a>(
        &'a self,
        computer: ComputerId,
        at: (TerminalId, &'a str),
        size: TerminalSize,
    ) -> BoxFuture<'a, Result<(), ComputerError>> {
        Box::pin(std::future::ready(
            self.terminals.resize(computer, at, size),
        ))
    }

    fn close_terminal<'a>(
        &'a self,
        computer: ComputerId,
        at: (TerminalId, &'a str),
    ) -> BoxFuture<'a, Result<(), ComputerError>> {
        Box::pin(self.terminals.close(computer, at))
    }

    fn worktrees(
        &self,
        computer: ComputerId,
    ) -> BoxFuture<'_, Result<Vec<KeptWorktree>, ComputerError>> {
        Box::pin(async move {
            self.registry.environment(computer)?;
            let Some(base) = &self.worktrees else {
                return Ok(Vec::new());
            };
            let recorded = crate::worktrees::recorded(base).await?;
            Ok(recorded
                .into_iter()
                .map(|(path, record)| KeptWorktree {
                    path: path.to_string_lossy().into_owned(),
                    branch: Some(record.branch).filter(|branch| !branch.is_empty()),
                    owner: record.owner,
                })
                .collect())
        })
    }

    fn free_worktree<'a>(
        &'a self,
        computer: ComputerId,
        path: &'a str,
    ) -> BoxFuture<'a, Result<Freed, ComputerError>> {
        Box::pin(async move {
            self.registry.environment(computer)?;
            let kept_because = |why: &str| {
                Ok(Freed::Kept {
                    why: why.to_owned(),
                })
            };
            let Some(base) = &self.worktrees else {
                return kept_because("this computer keeps no worktrees");
            };
            let (environment, processes) = (&self.environment, &self.processes);
            let found = crate::worktrees::at(base, Path::new(path), environment, processes);
            let Some(kept) = found.await? else {
                return kept_because("it is not here any more");
            };
            // Nothing opens a worktree of this repository meanwhile, and a thread that had this
            // one open is seen last, with the removal to follow at once.
            let _turn = self.worktree_locks.lock(&kept.repository).await?;
            if let Some(risk) = kept.at_risk() {
                return kept_because(&risk);
            }
            if !crate::worktrees::only_rebuildable(&kept, environment, processes).await? {
                return kept_because("it holds more than a build would make again");
            }
            if self.registry.open_on(&kept.path)? {
                return kept_because("a thread has it open");
            }
            crate::worktrees::remove(&kept, environment, processes).await?;
            Ok(Freed::Removed)
        })
    }

    fn destroy(&self, computer: ComputerId) -> BoxFuture<'_, Result<(), ComputerError>> {
        Box::pin(async move {
            self.registry.remove_computer(computer)?;
            self.background.stop_all(|on, _| on == computer).await;
            self.terminals.close_all(computer).await;
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
        Box::pin(self.open_now(computer, sources, None, Opening::Placed))
    }

    fn open_with_credential<'a>(
        &'a self,
        computer: ComputerId,
        sources: &'a Sources,
        credential: &'a GitCredential,
    ) -> BoxFuture<'a, Result<WorkspaceInfo, ComputerError>> {
        Box::pin(self.open_now(computer, sources, Some(credential), Opening::Placed))
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
            let helper = self.helper.as_deref();
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
        credentials: &'a dyn GitCredentials,
    ) -> BoxFuture<'a, Result<Finished, ComputerError>> {
        Box::pin(async move {
            let opened = self.registry.workspace(workspace)?;
            let helper = self.helper.as_deref();
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

    fn ended(
        &self,
        workspace: WorkspaceId,
        command: CommandId,
        wait: Duration,
    ) -> BoxFuture<'_, Result<Option<Finished>, ComputerError>> {
        Box::pin(async move {
            self.registry.workspace(workspace)?;
            self.background.ended(workspace, command, wait).await
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

    fn listening(
        &self,
        workspace: WorkspaceId,
    ) -> BoxFuture<'_, Result<Vec<Listening>, ComputerError>> {
        Box::pin(async move {
            self.registry.workspace(workspace)?;
            Ok(self.background.listening(workspace, &self.processes).await)
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

    fn changes<'a>(
        &'a self,
        workspace: WorkspaceId,
        since: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Option<Changes>, ComputerError>> {
        Box::pin(async move {
            let opened = self.registry.workspace(workspace)?;
            changes::since(&opened, since, &self.base).await
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
