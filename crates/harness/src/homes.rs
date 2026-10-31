//! People's own computers (`specs/computers.md`, "A person's computer in the cloud"): where a
//! deployment gives each person a computer of their own, a thread for them works there, in
//! their home, rather than on the harness's computer. Their folders, worktrees and scratch
//! folders are opened on it as they would be here; a scratch folder or a project's folder is
//! planned without waking it, and the first tool call that needs the computer wakes it (decision
//! 6). What a thread in a project's folder or a chat there opens with, and the skills it is
//! offered, are read there too, through the computer port, only when glave has just used the computer (its folder made, or a
//! turn that changed files there ending), and kept: a thread there opens with what was kept.

mod freeing;
mod kept;

pub use freeing::ShortOfRoom;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use glave_event::{ComputerKind, InstructionsRead, Opened, PrincipalId, WorkspaceBound};
use glave_ports::computer::{ComputerError, Home, Sandbox};
use glave_ports::{BlobStore, EventLog, ModelProvider, OrgId, Scope, ToolSource};

use crate::approval::Approver;
use crate::background::Commands;
use crate::harness::{Harness, HarnessError, Thread};
use crate::project_folders::{Brought, Settled, project_of};
use crate::removed::owner_label;
use crate::skills::on_home::skills_on;
use crate::skills::{Skill, SkillSource, Skills};
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

    /// The sandbox commands there run in, at its widest; none where the computer's own wall is
    /// their only one (`specs/sandbox.md`, "On a person's computer").
    fn sandbox(&self) -> Option<Sandbox>;
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
        match person.and_then(|person| self.own_home(scope.org(), person)) {
            Some(home) => Ok((Arc::new(home?), ComputerKind::Person)),
            None => Ok((Arc::clone(&self.workspace), ComputerKind::Local)),
        }
    }

    /// The home of `person`'s own computer in `org`, as planned, never woken, where people have
    /// computers of their own: where their threads work and their projects' folders are made.
    /// `None` where they have none.
    pub fn own_home(
        &self,
        org: OrgId,
        person: PrincipalId,
    ) -> Option<Result<Workspace, HarnessError>> {
        let homes = self.homes.as_ref()?;
        Some(homes.home(org, person).map_err(Into::into).map(|planned| {
            let home = Workspace::new(planned.computers, planned.home).asleep_at_times();
            // Their git sign-ins reach their computer as they reach this one: asked at each use.
            match self.workspace.credentials() {
                Some(credentials) => home.with_credentials(Arc::clone(credentials)),
                None => home,
            }
        }))
    }

    /// Opens `thread`'s workspace on its person's own computer when it works there and has none
    /// there yet: at the start of each turn, for a thread read from the log. A folder or a
    /// scratch folder is planned without waking the computer, which opens it again itself once
    /// it woke since. Its commands in the background are read again from its log, for the
    /// computer they ran on.
    pub(crate) async fn come_home(&self, thread: &mut Thread) -> Result<(), HarnessError> {
        let Some(bound) = thread.home.as_ref().map(|home| home.bound.clone()) else {
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

    /// What a thread in `scope` for `person`, working on their own computer where `bound` says,
    /// opens with (decision 6), never the server's: in a project's folder, their own
    /// instructions and the folder's AGENTS.md (`agents_md_on`) as last read there and kept; in
    /// a chat's scratch folder, their own instructions as last read and kept; so starting it
    /// never reaches the computer. In a worktree, which woke the computer to be made, what is
    /// read there now.
    pub(crate) async fn instructions_at_home(
        &self,
        scope: Scope,
        bound: &WorkspaceBound,
        person: Option<PrincipalId>,
    ) -> Option<String> {
        let root = Path::new(&bound.root);
        let files = match &bound.opened {
            Opened::Folder { .. } => self.last_read(scope, root).await?.files,
            Opened::Scratch { .. } => {
                let (home, _) = self.home_of(scope, person).ok()?;
                self.last_read(scope, home.root()).await?.files
            }
            Opened::Worktree { .. } => self.read_at_home(scope, person, bound).await.ok()?.0.files,
            Opened::Clone { .. } => return None,
        };
        crate::instructions::compose(root, &files)
    }

    /// What the project in `scope`, its folder at `root` on its person's own computer, chooses
    /// for its threads, from what was last read there: the server's account of a thread there,
    /// which reaches nothing.
    pub async fn settled_at_home(&self, scope: Scope, root: &Path) -> Settled {
        let Some(host) = &self.project_folders else {
            return Settled::default();
        };
        match self.kept_config(scope, root).await {
            Some(config) => host.settled_at_home(scope, root, &config).await,
            None => Settled::default(),
        }
    }

    /// The `config.toml` last read for `root` on its person's own computer and kept, which
    /// reaches nothing: what their project there asks them to trust is read from it.
    pub async fn kept_config(&self, scope: Scope, root: &Path) -> Option<String> {
        self.last_read(scope, root)
            .await
            .and_then(|read| read.config)
    }

    /// Reads again what `thread` opens with on its person's own computer, after a turn that ran
    /// a command or changed a file there, which the computer is awake for: what a compaction
    /// opens the conversation with and, kept, what the next thread in its folder, or the next
    /// chat, opens with.
    pub(crate) async fn refresh_at_home(&self, thread: &mut Thread) {
        let Some(home) = thread.home.as_mut().filter(|home| home.changed) else {
            return;
        };
        home.changed = false;
        let bound = home.bound.clone();
        // It has just been used, so asking it wakes nothing.
        self.free_room_at_home(thread).await;
        if matches!(bound.opened, Opened::Clone { .. }) {
            return;
        }
        let Ok((read, kept)) = self.read_at_home(thread.scope, thread.for_, &bound).await else {
            return;
        };
        thread.instructions = crate::instructions::compose(Path::new(&bound.root), &read.files);
        if kept {
            self.keep_read(thread.scope, read).await;
        } else if let Some(home) = thread.home.as_mut() {
            home.own = Some(read);
        }
    }

    /// Reads what a thread in the project folder at `root` in `scope` opens with through
    /// `home`, its person's home on their own computer, just woken to make that folder, and
    /// keeps it, so the project's threads open with it.
    pub async fn keep_instructions(&self, scope: Scope, home: &Workspace, root: &str) {
        let root = Path::new(root);
        match read_there(home, root, Some(root)).await {
            Ok(read) => self.keep_read(scope, read).await,
            Err(error) => tracing::warn!(%error, "a new project's instructions were not read"),
        }
    }

    /// What a thread of `person` working on their own computer where `bound` says opens with
    /// and is offered, read there now through its planned home, which wakes it if it sleeps, so
    /// only for a computer glave has just used; and where it is kept: for a project's folder, the
    /// folder; for a chat, which reads only the person's own, their home; a worktree's is not
    /// kept.
    async fn read_at_home(
        &self,
        scope: Scope,
        person: Option<PrincipalId>,
        bound: &WorkspaceBound,
    ) -> Result<(InstructionsRead, bool), HarnessError> {
        let (home, _) = self.home_of(scope, person)?;
        let root = Path::new(&bound.root);
        Ok(match bound.opened {
            Opened::Scratch { .. } => (read_there(&home, home.root(), None).await?, true),
            Opened::Folder { .. } => (read_there(&home, root, Some(root)).await?, true),
            Opened::Worktree { .. } | Opened::Clone { .. } => {
                (read_there(&home, root, Some(root)).await?, false)
            }
        })
    }

    /// What `thread` on its person's own computer brings for its next turn: what was last read
    /// there and kept, never the server's: its project's choices, within the server's
    /// constraints, and the `skill` tool for what was found there, with glave's own beside them.
    /// None read yet, or that read failing, only glave's own for this turn. A worktree goes by
    /// what its repository's folder was last read to hold, until a turn of its own reads its own.
    /// Loading a skill reads it there, as any tool call does.
    pub(crate) async fn brought_at_home(&self, thread: &Thread) -> Brought {
        let Some(at) = thread.home.as_ref() else {
            return Brought::default();
        };
        let (scope, person) = (thread.scope, thread.for_);
        let Ok((home, _)) = self.home_of(scope, person) else {
            return Brought::default();
        };
        let read = match (&at.own, &at.bound.opened) {
            (Some(own), _) => Some(own.clone()),
            (
                None,
                Opened::Folder { path }
                | Opened::Worktree {
                    repository: path, ..
                },
            ) => self.last_read(scope, Path::new(path)).await,
            (None, Opened::Scratch { .. }) => self.last_read(scope, home.root()).await,
            (None, Opened::Clone { .. }) => None,
        };
        let (config, found) = read.map_or_else(Default::default, |read| (read.config, read.skills));
        // A worktree goes by its repository's folder, which its project's yes is kept for.
        let root = project_of(&at.bound.opened).unwrap_or_else(|| PathBuf::from(&at.bound.root));
        let settled = match (&self.project_folders, &config) {
            (Some(host), Some(config)) => host.settled_at_home(scope, &root, config).await,
            _ => Settled::default(),
        };
        let mut skills: Vec<Skill> = found
            .into_iter()
            .map(|skill| Skill {
                name: skill.name,
                description: skill.description,
                source: SkillSource::File(PathBuf::from(skill.path)),
            })
            .collect();
        if let Some(own) = &self.skills {
            skills.extend(own.built_in());
        }
        Brought {
            settled,
            tools: Skills::offer_on(skills, home)
                .map(|skills| Arc::new(skills) as Arc<dyn ToolSource>)
                .into_iter()
                .collect(),
            ..Brought::default()
        }
    }
}

/// What a thread at `root`, or a chat with none, opens with and is offered, read through `home`,
/// which wakes the computer if it sleeps; as read for `kept_as`.
async fn read_there(
    home: &Workspace,
    kept_as: &Path,
    root: Option<&Path>,
) -> Result<InstructionsRead, ComputerError> {
    let (files, config) = match root {
        Some(root) => (
            crate::instructions::agents_md_on(home, root).await?,
            crate::instructions::config_on(home, root).await?,
        ),
        None => (crate::instructions::personal_on(home).await?, None),
    };
    Ok(InstructionsRead {
        root: kept_as.display().to_string(),
        files,
        skills: skills_on(home, root).await?,
        config,
    })
}

/// Where a thread works on its person's own computer.
#[derive(Debug, Clone)]
pub(crate) struct AtHome {
    /// Its binding there, its latest.
    pub(crate) bound: WorkspaceBound,
    /// Whether its turn ran a command or changed a file there, so what it opens with is read
    /// there again as the turn ends.
    pub(crate) changed: bool,
    /// What a turn of its own read in a worktree there, which no other thread goes by and so
    /// isn't kept; `None` for all else, which goes by what was kept.
    pub(crate) own: Option<InstructionsRead>,
}

impl AtHome {
    /// The same place, with nothing changed there yet: a subagent's, working where its parent
    /// does.
    pub(crate) fn fresh(&self) -> Self {
        Self {
            bound: self.bound.clone(),
            changed: false,
            own: self.own.clone(),
        }
    }
}

/// Where a thread works on its person's own computer, from `bound`, its latest binding; `None`
/// for any other.
pub(crate) fn at_home(bound: Option<&WorkspaceBound>) -> Option<AtHome> {
    bound
        .filter(|bound| bound.computer.kind == ComputerKind::Person)
        .map(|bound| AtHome {
            bound: bound.clone(),
            changed: false,
            own: None,
        })
}

/// Counts `thread`'s turn as having changed files on its person's own computer, when it works
/// there.
pub(crate) fn changed(thread: &mut Thread) {
    if let Some(home) = thread.home.as_mut() {
        home.changed = true;
    }
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
