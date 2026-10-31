//! The projects threads work in, as the host reads them (`specs/policy.md`, "Project trust"):
//! what a thread opens with, and what its project chooses and turns on for it: the project's
//! own settings (its model, its judge, titles, how its commands run), and once the person
//! trusts it, its own tools and hooks and the settings that loosen its commands. They are read
//! again at the start of every turn, so what the person trusts meanwhile is on from their next
//! message, and a thread in one project never gets another's. The nearest choice wins: the
//! thread's own, then its project's, then the harness's. Trust is the person's decision about
//! their own computer, so only their threads get any of it: another person's threads in the
//! project get none, and are told so.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use glave_event::Permissions;
use glave_ports::settings::{Hooks, Judge, Protections};
use glave_ports::{BlobStore, BoxFuture, EventLog, ModelProvider, ProjectId, ToolSource};

use crate::approval::Approver;
use crate::harness::{Harness, HarnessError, Thread};

/// What a thread working in a folder of its own opens with, and what its project chooses and
/// turns on, as the host reads them. Without one, every thread opens with the settings'
/// instructions and gets only the harness's own settings, tools and hooks.
pub trait ProjectFolders: Send + Sync {
    /// The instructions for a thread working at `root`, in the project whose folder is
    /// `project`: the person's own instructions, the folder's AGENTS.md, and what the project
    /// brings that stays off and why. `None` for `project` when the folder is a scratch folder
    /// of the thread's own. `trusting`: the thread is the person's whose trust turns a
    /// project's own on; for anyone else's, everything the project brings stays off.
    fn for_folder<'a>(
        &'a self,
        root: &'a Path,
        project: Option<&'a Path>,
        trusting: bool,
    ) -> BoxFuture<'a, Option<String>>;

    /// What the project whose folder is `project` chooses for its threads now. It starts
    /// nothing, only reads. Nothing by default.
    fn settled<'a>(&'a self, project: &'a Path) -> BoxFuture<'a, Settled> {
        let _ = project;
        Box::pin(async { Settled::default() })
    }

    /// What a project on a person's own computer chooses for its threads, from `config`, its
    /// `config.toml` as last read there at `project`: within the server's constraints, and
    /// without what loosens commands, which nothing there has trusted. Nothing by default.
    fn settled_at_home<'a>(&'a self, project: &'a Path, config: &'a str) -> BoxFuture<'a, Settled> {
        let _ = (project, config);
        Box::pin(async { Settled::default() })
    }

    /// What the project whose folder is `project`, the project `id`, chooses and turns on
    /// for its threads now: its choices, its trusted tools (started the first time they are
    /// asked for) and its trusted hooks. Nothing by default.
    fn brought<'a>(&'a self, project: &'a Path, id: ProjectId) -> BoxFuture<'a, Brought> {
        let _ = (project, id);
        Box::pin(async { Brought::default() })
    }
}

/// What a project chooses for the threads that work in it, nearest first: where it chooses
/// nothing, the harness's own holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settled {
    pub protections: Protections,
    /// The model its threads that chose none go to: one the host offers.
    pub model: Option<String>,
    /// Who judges flagged actions in its threads.
    pub judge: Option<Judge>,
    /// Whether a model titles its threads after their first exchange.
    pub titles: Option<bool>,
    /// Whether its new threads work in a worktree of their own.
    pub worktree: Option<bool>,
}

/// What a project chooses and turns on for the threads that work in it.
#[derive(Clone, Default)]
pub struct Brought {
    pub settled: Settled,
    /// Its tools, offered after the harness's own: a tool keeps the first source that names it.
    pub tools: Vec<Arc<dyn ToolSource>>,
    /// Its hooks, run after the harness's own at each point.
    pub hooks: Hooks,
    /// `web_fetch` as its own settings set it up, in place of the harness's.
    pub web: Option<Arc<dyn ToolSource>>,
}

impl std::fmt::Debug for Brought {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Brought")
            .field("settled", &self.settled)
            .field("tools", &self.tools.len())
            .field("hooks", &self.hooks)
            .field("web", &self.web.is_some())
            .finish()
    }
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Takes the harness's defaults as they are now, and reads again what `thread`'s project
    /// chooses and turns on, at the start of a turn; then gives its commands on this computer
    /// the sandbox that follows from its mode, within the most a sandbox here allows, and
    /// without the network where the project takes it. A thread with no project of its own here gets nothing of a
    /// project, and so does another person's thread than the one the harness works for, who
    /// trusted the project: no settings, no tools, no hooks.
    pub(crate) async fn bring(&self, thread: &mut Thread) -> Result<(), HarnessError> {
        self.come_home(thread).await?;
        thread.defaults = self.defaults();
        let brought = match (&self.project_folders, &thread.project) {
            _ if thread.home.is_some() => self.brought_at_home(thread).await,
            (Some(host), Some(project)) if self.trusting(thread.for_) => {
                host.brought(project, thread.scope.project()).await
            }
            _ => Brought::default(),
        };
        let workspace = self.workspace_for(thread);
        let protections = brought.settled.protections;
        let mode = thread.defaults.permissions(protections, thread.permissions);
        let planning = mode == Permissions::Plan;
        // A worker's own container or virtual machine is its wall: only this computer's
        // commands and those on a person's own computer run in the sandbox.
        let here = workspace.info().computer == self.workspace.info().computer;
        if here || thread.home.is_some() {
            let most = if here {
                thread.defaults.sandbox.as_ref()
            } else {
                thread.defaults.home_sandbox.as_ref()
            };
            let wanted = protections.sandbox(mode, most);
            let asks = thread.defaults.ask_network;
            if workspace.sandbox() != wanted.as_ref()
                || workspace.asks_network() != asks
                || workspace.plans() != planning
            {
                let within = workspace.within(wanted.as_ref()).await?;
                let within = within.asking_network(asks).planning(planning);
                thread.workspace = Some(Arc::new(within));
            }
        } else if workspace.plans() != planning {
            let same = workspace.within(workspace.sandbox()).await?;
            thread.workspace = Some(Arc::new(same.planning(planning)));
        }
        thread.brought = Arc::new(brought);
        Ok(())
    }

    /// Whether a thread for `person` is the person's the harness works for, whose trust turns
    /// a project's own on and whose own hooks and tools are the harness's.
    pub(crate) fn trusting(&self, person: Option<glave_event::PrincipalId>) -> bool {
        person == self.actors.person
    }

    /// What `project` chooses for the threads of the person the harness works for, as the
    /// host reads it now; nothing without a host.
    pub async fn settled(&self, project: &Path) -> Settled {
        match &self.project_folders {
            Some(host) => host.settled(project).await,
            None => Settled::default(),
        }
    }
}

/// The folder of the project a thread works in, from where it was bound: the folder it works
/// in place, or the one its worktree was made from. None for a scratch folder or a worker.
pub(crate) fn project_of(opened: &glave_event::Opened) -> Option<PathBuf> {
    match opened {
        glave_event::Opened::Folder { path } => Some(PathBuf::from(path)),
        glave_event::Opened::Worktree { repository, .. } => Some(PathBuf::from(repository)),
        glave_event::Opened::Scratch { .. } | glave_event::Opened::Clone { .. } => None,
    }
}
