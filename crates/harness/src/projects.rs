//! The projects threads work in, as the host reads them (`specs/policy.md`, "Project trust"):
//! what a thread opens with, and what its project turns on for it once the person trusts it:
//! the project's own tools and hooks, and settings that loosen how its commands run. They are
//! read again at the start of every turn, so what the person trusts meanwhile is on from their
//! next message, and a thread in one project never gets another's.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use glave_event::ApprovalMode;
use glave_ports::{BlobStore, BoxFuture, EventLog, ModelProvider, ToolSource};

use crate::approval::Approver;
use crate::harness::{Harness, HarnessError, Thread};
use crate::hooks::Hooks;

/// What a thread working in a folder of its own opens with, and what its project turns on, as
/// the host reads them. Without one, every thread opens with the settings' instructions and
/// gets only the harness's own tools and hooks.
pub trait Projects: Send + Sync {
    /// The instructions for a thread working at `root`, in the project whose folder is
    /// `project`: the person's own instructions, the folder's AGENTS.md, and what the project
    /// brings that stays off and why. `None` for `project` when the folder is a scratch folder
    /// of the thread's own.
    fn for_folder<'a>(
        &'a self,
        root: &'a Path,
        project: Option<&'a Path>,
    ) -> BoxFuture<'a, Option<String>>;

    /// How the project whose folder is `project` loosens the way its threads' commands run
    /// now. Cheap: it starts nothing. Nothing by default.
    fn loosened<'a>(&'a self, project: &'a Path) -> BoxFuture<'a, Loosened> {
        let _ = project;
        Box::pin(async { Loosened::default() })
    }

    /// What the project whose folder is `project` turns on for its threads now: its trusted
    /// tools (started the first time they are asked for), its trusted hooks, and how it
    /// loosens their commands. Nothing by default.
    fn brought<'a>(&'a self, project: &'a Path) -> BoxFuture<'a, Brought> {
        let _ = project;
        Box::pin(async { Brought::default() })
    }
}

/// How a trusted project's settings loosen the way its threads' commands run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Loosened {
    /// The mode its threads run in when they chose none: `auto` when the project chose it,
    /// `ask` when it turned the sandbox off without choosing auto; none to leave the harness's.
    pub approval: Option<ApprovalMode>,
    /// Whether its threads' commands run without the sandbox.
    pub unconfined: bool,
}

/// What a project turns on for the threads that work in it.
#[derive(Clone, Default)]
pub struct Brought {
    /// Its tools, offered after the harness's own: a tool keeps the first source that names it.
    pub tools: Vec<Arc<dyn ToolSource>>,
    /// Its hooks, run after the harness's own at each point.
    pub hooks: Hooks,
    pub loosened: Loosened,
}

impl std::fmt::Debug for Brought {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Brought")
            .field("tools", &self.tools.len())
            .field("hooks", &self.hooks)
            .field("loosened", &self.loosened)
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
    /// Reads again what `thread`'s project turns on, at the start of a turn, and gives its
    /// commands the sandbox that follows: none where the project turned it off, else the
    /// harness's own. A thread with no project of its own here gets nothing.
    pub(crate) async fn bring(&self, thread: &mut Thread) -> Result<(), HarnessError> {
        let (Some(host), Some(project)) = (&self.projects, &thread.project) else {
            return Ok(());
        };
        let brought = host.brought(project).await;
        if let Some(workspace) = &thread.workspace {
            let wanted = if brought.loosened.unconfined {
                None
            } else {
                self.workspace.sandbox()
            };
            if workspace.sandbox() != wanted {
                thread.workspace = Some(Arc::new(workspace.within(wanted).await?));
            }
        }
        thread.brought = Arc::new(brought);
        Ok(())
    }

    /// How `project` loosens its threads' commands now, as the host reads it; nothing without
    /// a host.
    pub async fn loosened(&self, project: &Path) -> Loosened {
        match &self.projects {
            Some(host) => host.loosened(project).await,
            None => Loosened::default(),
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
