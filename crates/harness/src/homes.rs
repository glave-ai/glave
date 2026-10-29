//! People's own computers (`specs/computers.md`, "A person's computer in the cloud"): where a
//! deployment gives each person a computer of their own, a thread for them works there, in
//! their home, rather than on the harness's computer. Their folders, worktrees and scratch
//! folders are opened on it as they would be here; a scratch folder is planned without waking
//! it, and the first tool call that needs the computer wakes it (decision 6).

use std::sync::Arc;

use glave_event::{ComputerKind, Opened, PrincipalId, WorkspaceBound};
use glave_ports::computer::{ComputerError, Home};
use glave_ports::{BlobStore, EventLog, ModelProvider, OrgId, Scope};

use crate::approval::Approver;
use crate::background::Commands;
use crate::harness::{Harness, HarnessError, Thread};
use crate::removed::owner_label;
use crate::tools::workspace::Workspace;

/// Each person's own computer, woken by the calls that need it. The harness cannot reach a
/// cluster itself (its crate depends on no adapter), so the host hands it this.
pub trait Homes: Send + Sync {
    /// The computer of `person` in `org` and the workspace of their home on it, as planned: at
    /// once, never waking it. The first call on it that needs the computer (a command, a file
    /// read or written) wakes it and waits for it, and the same computer comes back each time.
    fn home(&self, org: OrgId, person: PrincipalId) -> Result<Home, ComputerError>;

    /// Keeps the computer of `person` in `org` from sleeping while what it answers is kept:
    /// for a turn or a task, which may run long between its commands. It does not wake it.
    fn hold(&self, org: OrgId, person: PrincipalId) -> Result<Box<dyn Send>, ComputerError>;
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Where the threads of `person` in `scope` open their folders, worktrees and scratch
    /// folders, and the kind of computer that is: their own computer's home, as planned (woken
    /// by the first call that needs it), where people have computers of their own; else the
    /// harness's own workspace.
    pub(crate) fn home_of(
        &self,
        scope: Scope,
        person: Option<PrincipalId>,
    ) -> Result<(Arc<Workspace>, ComputerKind), HarnessError> {
        let (Some(homes), Some(person)) = (&self.homes, person) else {
            return Ok((Arc::clone(&self.workspace), ComputerKind::Local));
        };
        let planned = homes.home(scope.org(), person)?;
        let home = Workspace::new(planned.computers, planned.home);
        // Their git sign-ins reach their computer as they reach this one: asked at each use.
        let home = match self.workspace.credentials() {
            Some(credentials) => home.with_credentials(Arc::clone(credentials)),
            None => home,
        };
        Ok((Arc::new(home), ComputerKind::Person))
    }

    /// Opens `thread`'s workspace on its person's own computer when it works there and has none
    /// there yet: at the start of each turn, for a thread read from the log. A scratch folder
    /// is planned without waking the computer, which opens it again itself once it woke since.
    /// Its commands in the background are read again from its log, for the computer they ran
    /// on.
    pub(crate) async fn come_home(&self, thread: &mut Thread) -> Result<(), HarnessError> {
        let Some(bound) = thread.home.clone() else {
            return Ok(());
        };
        if self.homes.is_none() {
            return Err(ComputerError::Unsupported(
                "this thread worked on a cloud computer this server does not reach. Start a new \
                 thread to go on"
                    .into(),
            )
            .into());
        }
        let (home, _) = self.home_of(thread.scope, thread.for_)?;
        let there = home.info().computer;
        if thread
            .workspace
            .as_ref()
            .is_some_and(|workspace| workspace.info().computer == there)
        {
            return Ok(());
        }
        let owner = owner_label(thread.scope, thread.id);
        let workspace = match &bound.opened {
            Opened::Folder { path } => home.folder(path).await?,
            Opened::Worktree { repository, branch } => {
                home.worktree(repository, branch, Some(&owner)).await?
            }
            Opened::Scratch { .. } => home.scratch(&thread.id.to_string(), Some(&owner)).await?,
            Opened::Clone { .. } => return Ok(()),
        };
        let events = self.load(&thread.scope, thread.id).await?;
        thread.commands = Arc::new(Commands::rebuilt(&events, workspace.computers()));
        thread.workspace = Some(Arc::new(workspace));
        Ok(())
    }
}

/// The binding of a thread that works on its person's own computer, from `bound`, its
/// latest; `None` for any other.
pub(crate) fn at_home(bound: Option<&WorkspaceBound>) -> Option<WorkspaceBound> {
    bound
        .filter(|bound| bound.computer.kind == ComputerKind::Person)
        .cloned()
}

/// Closes `workspace`, opened on a person's own computer for a thread let go of: one never
/// opened there is only forgotten, and one where a command of its still runs stays open until
/// the computer sleeps (the computer's own adapter keeps it, `Waking` in `glave-computer`).
pub(crate) async fn close_away(workspace: Option<Arc<Workspace>>) {
    let Some(workspace) = workspace else {
        return;
    };
    let id = workspace.info().id;
    if let Err(error) = workspace.computers().close(id).await {
        tracing::warn!(%error, %id, "could not close a workspace on a person's computer");
    }
}
