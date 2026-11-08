//! A person's computer as the harness holds it while it may be asleep (`specs/computers.md`,
//! decision 6): a chat or a thread that needs no computer never wakes it. Its home, the scratch
//! folders and the folders (a project's) opened from it are planned without asking the computer
//! anything, at the roots that computer puts them at; the first call that needs the computer (a
//! command, a file read, written or searched) wakes it, opens the workspace there and goes on. A
//! computer that slept and woke since opens it again. Anything else opened on it (a worktree, a
//! clone, a new project's folder) needs the computer at once, and wakes it.

mod machine;
mod terminals;

pub use machine::{Grow, Lost, Short};

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use glave_ports::computer::{
    Background, Bytes, Changes, Command, CommandId, ComputerError, ComputerId, ComputerInfo,
    Computers, Environment, FileWrite, Finished, Freed, GitCredential, GitCredentials, Glob,
    Globbed, Home, KeptWorktree, Listed, Listening, Listing, Output, Printed, Save, Saved,
    Screenful, Search, Searched, Since, Sources, Stat, TerminalId, TerminalInfo, TerminalSize,
    WorkspaceId, WorkspaceInfo,
};
use glave_ports::{BoxFuture, Cancellation};
use tokio::time::Instant;

/// Wakes the computer, if it sleeps, and answers it awake with its home: what [`Waking`] asks
/// before each call that needs the computer.
pub type Wake = Arc<dyn Fn() -> BoxFuture<'static, Result<Home, ComputerError>> + Send + Sync>;

/// Looks whether the computer is awake, never waking it: its home there while this server
/// reaches it awake; none while it sleeps or is being woken, so nothing from before runs there;
/// unreachable while it runs where this server does not reach it (driven by another server, or
/// woken by this one before it restarted). What [`Waking`] asks before a call that must find
/// what already runs there, such as a person's terminal, and must never wake it.
pub type Peek =
    Arc<dyn Fn() -> BoxFuture<'static, Result<Option<Home>, ComputerError>> + Send + Sync>;

/// How long a computer that answered a call is taken to be awake without asking again: a turn
/// holds it awake, asking it how it is once a minute, so only a call after a quiet spell asks
/// whether it slept meanwhile.
const TRUSTED: Duration = Duration::from_secs(30);

/// A person's computer that wakes only for the calls that need it.
pub struct Waking {
    wake: Wake,
    peek: Peek,
    /// Grows it when a command runs short, and says whether it lost its machine, where it has
    /// a machine of its own to lose.
    machine: Option<(Grow, Lost)>,
    /// The computer as planned, and its home there.
    home: WorkspaceInfo,
    /// Where it makes scratch folders: each is this, `/`, and its name.
    scratch: String,
    /// Each workspace planned on it, and where it is open for real.
    workspaces: Mutex<BTreeMap<WorkspaceId, Arc<Planned>>>,
    /// The computer last found awake, and when a call on it last went through.
    awake: Mutex<Option<(ComputerId, Option<Instant>)>>,
}

/// A workspace planned on the computer.
struct Planned {
    info: WorkspaceInfo,
    /// Where it is open, once a call needed it; opened again on a computer woken since.
    real: Mutex<Option<Real>>,
    /// Held by the one call opening it, so two calls at once share one wake and one open.
    opening: tokio::sync::Mutex<()>,
    /// Let go of while a command of its ran in the background there: kept for that command
    /// until the computer sleeps, which ends it, then forgotten.
    closing: AtomicBool,
    /// Where its folder's history stood as it was first opened on the computer awake, which a
    /// plan made asleep can't know: what changes are read since when none is asked.
    first_at: Mutex<Option<String>>,
}

impl Planned {
    /// Where its folder's history stood as it was first opened on the computer awake.
    fn first_at(&self) -> Option<String> {
        self.first_at
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Keeps `at` as where its folder's history stood when first opened, unless it was opened
    /// before.
    fn opened_at(&self, at: Option<&String>) {
        let mut first = self.first_at.lock().unwrap_or_else(PoisonError::into_inner);
        if first.is_none() {
            *first = at.cloned();
        }
    }

    fn real(&self) -> Option<Real> {
        self.real
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
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

/// Whether a workspace opened from `sources` can be opened again on the computer woken since,
/// at the same root: a folder, a worktree on its branch, a scratch folder. A clone or a new
/// project's folder would be another.
fn reopens(sources: &Sources) -> bool {
    matches!(
        sources,
        Sources::Folder { .. } | Sources::Worktree { .. } | Sources::Scratch { .. }
    )
}

impl Waking {
    /// The computer `wake` wakes, `peek` finds awake without waking it, and `machine`, if
    /// given, grows and says whether it lost its machine, planned as `home`: its home's
    /// workspace, which names the computer, whose scratch folders are made in the folder
    /// `scratch`. Nothing is asked of it until a call needs it.
    pub fn home(
        (wake, peek): (Wake, Peek),
        machine: Option<(Grow, Lost)>,
        home: WorkspaceInfo,
        scratch: &str,
    ) -> Home {
        let waking = Self {
            wake,
            peek,
            machine,
            home: home.clone(),
            scratch: scratch.trim_end_matches('/').to_owned(),
            workspaces: Mutex::new(BTreeMap::new()),
            awake: Mutex::new(None),
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
            first_at: Mutex::new(info.at.clone()),
            info,
            real: Mutex::new(real),
            opening: tokio::sync::Mutex::new(()),
            closing: AtomicBool::new(false),
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

    fn forget(&self, workspace: WorkspaceId) {
        self.workspaces
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&workspace);
    }

    /// The computer last found awake, and whether a call on it went through lately.
    fn last_awake(&self) -> Option<(ComputerId, bool)> {
        let awake = *self.awake.lock().unwrap_or_else(PoisonError::into_inner);
        awake.map(|(computer, at)| (computer, at.is_some_and(|at| at.elapsed() < TRUSTED)))
    }

    /// Whether `computer` is the one [`Peek`] finds awake now: never so once the server let go
    /// of it, its machine lost, so a call after that wakes a new one.
    async fn still_awake(&self, computer: ComputerId) -> bool {
        matches!((self.peek)().await, Ok(Some(awake)) if awake.home.computer == computer)
    }

    /// Where `workspace` is open on the computer last found awake, if a call opened it there;
    /// never wakes the computer. Opened on one that slept since, it is not: its commands are
    /// gone with it.
    fn opened(&self, planned: &Planned) -> Option<Real> {
        let open = planned.real()?;
        let (computer, _) = self.last_awake()?;
        (open.computer == computer).then_some(open)
    }

    /// Counts `computer` awake as of now, and forgets what was kept only for commands on a
    /// computer before it.
    fn saw(&self, computer: ComputerId) {
        let before = self
            .awake
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .replace((computer, Some(Instant::now())));
        if before.is_some_and(|(was, _)| was != computer) {
            self.workspaces
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .retain(|_, planned| !planned.closing.load(Ordering::SeqCst));
        }
    }

    /// `result`, a call's on the computer awake: one that went through counts it awake as of
    /// now; one that found it unreachable has the next call ask whether it slept.
    fn done<T>(&self, result: Result<T, ComputerError>) -> Result<T, ComputerError> {
        let mut awake = self.awake.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, at)) = awake.as_mut() {
            *at = match &result {
                Err(ComputerError::Unreachable(_)) => None,
                _ => Some(Instant::now()),
            };
        }
        result
    }

    /// The computer awake, woken if it sleeps, and `workspace` open on it: opened there now
    /// when no call has opened it on the computer awake. One a call went through on lately is
    /// taken as awake without asking.
    async fn real(&self, workspace: WorkspaceId) -> Result<Real, ComputerError> {
        let planned = self.planned(workspace)?;
        let trusted = |open: &Real| {
            self.last_awake()
                .is_some_and(|(computer, lately)| lately && computer == open.computer)
        };
        if let Some(open) = planned.real().filter(trusted)
            && self.still_awake(open.computer).await
        {
            return Ok(open);
        }
        let _opening = planned.opening.lock().await;
        let awake = (self.wake)().await?;
        self.saw(awake.home.computer);
        if let Some(open) = planned.real()
            && open.computer == awake.home.computer
        {
            return Ok(open);
        }
        let info = if workspace == self.home.id {
            awake.home.clone()
        } else if reopens(&planned.info.sources) {
            match awake
                .computers
                .open(awake.home.computer, &planned.info.sources)
                .await
            {
                Err(ComputerError::NotFound(_))
                    if matches!(planned.info.sources, Sources::Folder { .. }) =>
                {
                    return Err(ComputerError::NotFound(format!(
                        "this project's folder, {}, isn't on your computer anymore; make a new \
                         project there to go on",
                        planned.info.root
                    )));
                }
                opened => opened?,
            }
        } else {
            return Err(ComputerError::Unsupported(
                "this workspace was on your computer before it slept and cannot be made again \
                 there. Start a new thread to go on"
                    .into(),
            ));
        };
        if info.root != planned.info.root {
            if workspace != self.home.id {
                let _ = awake.computers.close(info.id).await;
            }
            return Err(ComputerError::Failed(format!(
                "your computer opened this folder at {}, not at {} as glave planned",
                info.root, planned.info.root
            )));
        }
        planned.opened_at(info.at.as_ref());
        let open = Real {
            computers: awake.computers,
            computer: awake.home.computer,
            workspace: info.id,
        };
        *planned.real.lock().unwrap_or_else(PoisonError::into_inner) = Some(open.clone());
        Ok(open)
    }

    /// Has the next call ask whether the computer is awake.
    fn distrust(&self) {
        if let Some((_, at)) = self
            .awake
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_mut()
        {
            *at = None;
        }
    }

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

    fn worktrees(
        &self,
        _computer: ComputerId,
    ) -> BoxFuture<'_, Result<Vec<KeptWorktree>, ComputerError>> {
        Box::pin(async move {
            let home = self.real(self.home.id).await?;
            home.computers.worktrees(home.computer).await
        })
    }

    fn free_worktree<'a>(
        &'a self,
        _computer: ComputerId,
        path: &'a str,
    ) -> BoxFuture<'a, Result<Freed, ComputerError>> {
        Box::pin(async move {
            let home = self.real(self.home.id).await?;
            home.computers.free_worktree(home.computer, path).await
        })
    }

    fn destroy(&self, _computer: ComputerId) -> BoxFuture<'_, Result<(), ComputerError>> {
        Box::pin(std::future::ready(Err(ComputerError::Unsupported(
            "a person's computer is removed with the person, not by a thread".into(),
        ))))
    }

    /// A scratch folder or a folder is planned at once, at the root the computer opens it at:
    /// a scratch folder where it makes them, a folder at its own path, which glave keeps as
    /// that computer named it. Anything else is opened on the computer, woken for it.
    fn open<'a>(
        &'a self,
        computer: ComputerId,
        sources: &'a Sources,
    ) -> BoxFuture<'a, Result<WorkspaceInfo, ComputerError>> {
        Box::pin(async move {
            if computer != self.home.computer {
                return Err(ComputerError::NotFound(format!("no computer {computer}")));
            }
            let root = match sources {
                Sources::Scratch { name, .. } => Some(format!("{}/{name}", self.scratch)),
                Sources::Folder { path } => Some(path.clone()),
                _ => None,
            };
            if let Some(root) = root {
                // Where its history stands is not known without asking the computer.
                let info = WorkspaceInfo {
                    id: WorkspaceId::from_uuid(crate::local::new_id()?),
                    root,
                    sources: sources.clone(),
                    at: None,
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

    /// Closed where it is open, and forgotten; one never opened, or opened on a computer that
    /// slept since, is only forgotten. One where a command of its still runs in the background
    /// stays open there for it until the computer sleeps.
    fn close(&self, workspace: WorkspaceId) -> BoxFuture<'_, Result<(), ComputerError>> {
        Box::pin(async move {
            let planned = self.planned(workspace)?;
            let Some(open) = self.opened(&planned) else {
                self.forget(workspace);
                return Ok(());
            };
            match self.done(open.computers.commands(open.workspace).await) {
                Ok(running) if !running.is_empty() => {
                    planned.closing.store(true, Ordering::SeqCst);
                    Ok(())
                }
                Ok(_) => {
                    self.forget(workspace);
                    match self.done(open.computers.close(open.workspace).await) {
                        Err(ComputerError::NotFound(_)) => Ok(()),
                        closed => closed,
                    }
                }
                // Its computer is gone, and its commands with it.
                Err(_) => {
                    self.forget(workspace);
                    Ok(())
                }
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
            let ran = self.done(
                open.computers
                    .run(open.workspace, command, output, stop)
                    .await,
            );
            self.ran(ran, output).await
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
            let ran = self.done(
                open.computers
                    .run_with_credentials(open.workspace, command, output, stop, credentials)
                    .await,
            );
            self.ran(ran, output).await
        })
    }

    fn start<'a>(
        &'a self,
        workspace: WorkspaceId,
        command: &'a Command,
    ) -> BoxFuture<'a, Result<CommandId, ComputerError>> {
        Box::pin(async move {
            let open = self.real(workspace).await?;
            self.told(open.computers.start(open.workspace, command).await)
                .await
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
            match self.opened(&*self.planned(workspace)?) {
                Some(open) => {
                    self.gone_with_it(open.computers.output(open.workspace, command, wait).await)
                        .await
                }
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
            match self.opened(&*self.planned(workspace)?) {
                Some(open) => {
                    self.gone_with_it(open.computers.ended(open.workspace, command, wait).await)
                        .await
                }
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
            match self.opened(&*self.planned(workspace)?) {
                Some(open) => {
                    self.gone_with_it(open.computers.stop(open.workspace, command).await)
                        .await
                }
                None => Err(ComputerError::NotFound(format!("no command {command}"))),
            }
        })
    }

    fn commands(
        &self,
        workspace: WorkspaceId,
    ) -> BoxFuture<'_, Result<Vec<Background>, ComputerError>> {
        Box::pin(async move {
            match self.opened(&*self.planned(workspace)?) {
                Some(open) => {
                    self.gone_with_it(open.computers.commands(open.workspace).await)
                        .await
                }
                None => Ok(Vec::new()),
            }
        })
    }

    fn listening(
        &self,
        workspace: WorkspaceId,
    ) -> BoxFuture<'_, Result<Vec<Listening>, ComputerError>> {
        Box::pin(async move {
            match self.opened(&*self.planned(workspace)?) {
                Some(open) => self.done(open.computers.listening(open.workspace).await),
                None => Ok(Vec::new()),
            }
        })
    }

    fn open_terminal<'a>(
        &'a self,
        computer: ComputerId,
        folder: &'a str,
        owner: &'a str,
        size: TerminalSize,
    ) -> BoxFuture<'a, Result<TerminalId, ComputerError>> {
        Box::pin(self.open_terminal_woken(computer, (folder, owner), size))
    }

    fn terminals<'a>(
        &'a self,
        _computer: ComputerId,
        owner: &'a str,
    ) -> BoxFuture<'a, Result<Vec<TerminalInfo>, ComputerError>> {
        Box::pin(self.terminals_awake(owner))
    }

    fn write_terminal<'a>(
        &'a self,
        _computer: ComputerId,
        at: (TerminalId, &'a str),
        input: &'a Bytes,
    ) -> BoxFuture<'a, Result<(), ComputerError>> {
        Box::pin(async move {
            let awake = self.terminal_awake(at.0).await?;
            awake
                .computers
                .write_terminal(awake.computer, at, input)
                .await
        })
    }

    fn read_terminal<'a>(
        &'a self,
        _computer: ComputerId,
        at: (TerminalId, &'a str),
        from: u64,
        wait: Duration,
    ) -> BoxFuture<'a, Result<Screenful, ComputerError>> {
        Box::pin(async move {
            let awake = self.terminal_awake(at.0).await?;
            let read = awake
                .computers
                .read_terminal(awake.computer, at, from, wait);
            read.await
        })
    }

    fn resize_terminal<'a>(
        &'a self,
        _computer: ComputerId,
        at: (TerminalId, &'a str),
        size: TerminalSize,
    ) -> BoxFuture<'a, Result<(), ComputerError>> {
        Box::pin(async move {
            let awake = self.terminal_awake(at.0).await?;
            let resized = awake.computers.resize_terminal(awake.computer, at, size);
            resized.await
        })
    }

    fn close_terminal<'a>(
        &'a self,
        _computer: ComputerId,
        at: (TerminalId, &'a str),
    ) -> BoxFuture<'a, Result<(), ComputerError>> {
        Box::pin(async move {
            let awake = self.terminal_awake(at.0).await?;
            awake.computers.close_terminal(awake.computer, at).await
        })
    }

    fn read<'a>(
        &'a self,
        workspace: WorkspaceId,
        path: &'a str,
    ) -> BoxFuture<'a, Result<Bytes, ComputerError>> {
        Box::pin(async move {
            let open = self.real(workspace).await?;
            self.told(open.computers.read(open.workspace, path).await)
                .await
        })
    }

    fn write<'a>(
        &'a self,
        workspace: WorkspaceId,
        files: &'a [FileWrite],
    ) -> BoxFuture<'a, Result<(), ComputerError>> {
        Box::pin(async move {
            let open = self.real(workspace).await?;
            self.told(open.computers.write(open.workspace, files).await)
                .await
        })
    }

    fn list<'a>(
        &'a self,
        workspace: WorkspaceId,
        listing: &'a Listing,
    ) -> BoxFuture<'a, Result<Listed, ComputerError>> {
        Box::pin(async move {
            let open = self.real(workspace).await?;
            self.told(open.computers.list(open.workspace, listing).await)
                .await
        })
    }

    fn stat<'a>(
        &'a self,
        workspace: WorkspaceId,
        path: &'a str,
    ) -> BoxFuture<'a, Result<Stat, ComputerError>> {
        Box::pin(async move {
            let open = self.real(workspace).await?;
            self.told(open.computers.stat(open.workspace, path).await)
                .await
        })
    }

    fn glob<'a>(
        &'a self,
        workspace: WorkspaceId,
        glob: &'a Glob,
    ) -> BoxFuture<'a, Result<Globbed, ComputerError>> {
        Box::pin(async move {
            let open = self.real(workspace).await?;
            self.told(open.computers.glob(open.workspace, glob).await)
                .await
        })
    }

    fn search<'a>(
        &'a self,
        workspace: WorkspaceId,
        search: &'a Search,
    ) -> BoxFuture<'a, Result<Searched, ComputerError>> {
        Box::pin(async move {
            let open = self.real(workspace).await?;
            self.told(open.computers.search(open.workspace, search).await)
                .await
        })
    }

    /// Asked only of a computer a call went through on lately, where the workspace is open:
    /// reading what changed never wakes one. With no point asked, since where its folder stood
    /// as it was first opened awake, which nothing of the thread's changed before.
    fn changes<'a>(
        &'a self,
        workspace: WorkspaceId,
        since: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Option<Changes>, ComputerError>> {
        Box::pin(async move {
            let planned = self.planned(workspace)?;
            let lately = self.last_awake().is_some_and(|(_, lately)| lately);
            let Some(open) = self.opened(&planned).filter(|_| lately) else {
                return Err(ComputerError::Unsupported(
                    "the computer may be asleep, and asking it what changed would wake it".into(),
                ));
            };
            let first = since.is_none().then(|| planned.first_at()).flatten();
            let asked = since.or(first.as_deref());
            let read = open.computers.changes(open.workspace, asked).await;
            Ok(self.done(read)?.map(|changes| match changes.since {
                Since::Asked if first.is_some() => Changes {
                    since: Since::Opened,
                    ..changes
                },
                _ => changes,
            }))
        })
    }

    fn save<'a>(
        &'a self,
        workspace: WorkspaceId,
        save: &'a Save,
    ) -> BoxFuture<'a, Result<Saved, ComputerError>> {
        Box::pin(async move {
            let open = self.real(workspace).await?;
            self.told(open.computers.save(open.workspace, save).await)
                .await
        })
    }
}

#[cfg(test)]
mod tests;
