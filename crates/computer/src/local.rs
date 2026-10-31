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
mod on_path;
mod paths;
mod pressure;
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

use glave_ports::computer::{
    Background, Bytes, Command, CommandId, ComputerError, ComputerId, ComputerInfo, Computers,
    DiskSpace, DiskUse, Environment, FileWrite, Finished, Freed, GitCredential, GitCredentials,
    Glob, Globbed, Image, KeptWorktree, Listed, Listing, Network, Output, Printed, Refusal, Save,
    Saved, Search, Searched, Size, Sources, Stat, State, WorkspaceId, WorkspaceInfo,
};
use glave_ports::secrets::without_secrets;
use glave_ports::{BoxFuture, Cancellation};
use glave_process::{Owner, Processes};

use background::Backgrounds;
use command::Keep;
pub(crate) use registry::new_id;
use registry::{Opened, Registry};

pub use on_path::glave_on_path;
pub(crate) use sandbox::CACHE_FOLDER;
pub use sandbox::{sandbox_available, sandbox_helper};

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
    /// The cgroup it runs in, where Linux counts the time its commands are held at its CPU
    /// limit.
    cgroup: Option<PathBuf>,
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
            sandbox_helper: None,
            hidden: Arc::new([]),
            glave_on_path: None,
            environment: Arc::new(without_secrets(environment)),
            registry: Registry::default(),
            background: Backgrounds::default(),
            cgroup: pressure::cgroup(),
            processes: Processes::unrecorded(),
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

    /// Says its disk holds what `measure` answers, for a test that can't fill one.
    #[must_use]
    pub fn with_disk_measure(
        mut self,
        measure: impl Fn() -> DiskSpace + Send + Sync + 'static,
    ) -> Self {
        self.measure = Some(Measure(Arc::new(measure)));
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

    /// Records the commands its workspaces start in `processes`' ledger while they run, so a
    /// glave that starts after this one crashed stops them (`glave_process`). Without it they
    /// are still stopped with their workspace, only not recorded.
    #[must_use]
    pub fn with_processes(mut self, processes: Processes) -> Self {
        self.processes = processes;
        self
    }

    /// Hides `places` from its sandboxed commands too, as the home folder's credential stores
    /// are, whether they exist yet or not (on Linux, from the next command once they do): the
    /// evals hide this machine's own crate downloads, so an attempt starts as a fresh computer
    /// would.
    #[must_use]
    pub fn with_hidden(mut self, places: impl IntoIterator<Item = PathBuf>) -> Self {
        self.hidden = places.into_iter().collect();
        self
    }

    /// Puts `folder`, which holds `glave` (see [`crate::glave_on_path`]), first on every
    /// command's `PATH`, and keeps it readable in every sandbox though it sits in glave's own
    /// folder, which they hide: a command's `glave` is then the one running it.
    #[must_use]
    pub fn with_glave_on_path(mut self, folder: impl Into<PathBuf>) -> Self {
        self.glave_on_path = Some(folder.into());
        self
    }

    /// Whether a command still runs here in the background: for a computer that ends when
    /// idle ([`crate::Idle`]).
    pub fn busy(&self) -> bool {
        self.background.running()
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
            disk: None,
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
        };
        let opened = Opened {
            computer,
            root,
            copy,
            git_common,
            environment,
            hidden: Arc::clone(&self.hidden),
            glave_on_path: self.glave_on_path.clone(),
            credential: credential.cloned(),
            prepared: Mutex::new(None),
            processes: self.processes.owned_by(Owner::Workspace(id.to_string())),
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
        let helper = self.sandbox_helper.as_deref();
        let sandbox = command.sandbox.as_ref();
        let spawned = command::spawn(&opened, &command.script, sandbox, helper, None).await?;
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
            Ok(info)
        })
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

    fn disk_use(&self, computer: ComputerId) -> BoxFuture<'_, Result<Vec<DiskUse>, ComputerError>> {
        Box::pin(async move {
            self.registry.environment(computer)?;
            crate::disk::largest_in(&self.environment).await
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
        credentials: &'a dyn GitCredentials,
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
