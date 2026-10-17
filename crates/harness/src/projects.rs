//! The projects threads work in, as the host reads them (`specs/policy.md`, "Project trust"):
//! what a thread opens with, and what its project turns on for it once the person trusts it:
//! the project's own tools and hooks, and settings that loosen how its commands run. They are
//! read again at the start of every turn, so what the person trusts meanwhile is on from their
//! next message, and a thread in one project never gets another's. Trust is the person's
//! decision about their own computer, so only their threads get any of it: another person's
//! threads in the project get none, and are told so.

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
    /// of the thread's own. `trusting`: the thread is the person's whose trust turns a
    /// project's own on; for anyone else's, everything the project brings stays off.
    fn for_folder<'a>(
        &'a self,
        root: &'a Path,
        project: Option<&'a Path>,
        trusting: bool,
    ) -> BoxFuture<'a, Option<String>>;

    /// How the project whose folder is `project` loosens the way its threads' commands run
    /// now. It starts nothing, only reads. Nothing by default.
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
    /// The mode its threads run in when they chose none: `auto` when it is chosen, `ask` when
    /// the project turned the sandbox off with auto chosen nowhere; none to leave the
    /// harness's. Always set where its commands run unconfined.
    pub approval: Option<ApprovalMode>,
    /// Whether its threads' commands run without the sandbox.
    pub unconfined: bool,
}

impl Loosened {
    /// The mode a thread of the project runs in: its own choice (`chosen`), else the
    /// project's, else the harness's (`own`). Where the project runs commands unconfined
    /// without auto chosen, it asks whatever the thread chose: its auto was chosen behind the
    /// sandbox, and commands run unconfined with nothing asked only when auto is chosen for
    /// that.
    pub fn mode(self, chosen: Option<ApprovalMode>, own: ApprovalMode) -> ApprovalMode {
        if self.unconfined && self.approval != Some(ApprovalMode::Auto) {
            return ApprovalMode::Ask;
        }
        chosen.or(self.approval).unwrap_or(own)
    }
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
    /// harness's own. A thread with no project of its own here gets nothing, and so does
    /// another person's thread than the one the harness works for, who trusted the project:
    /// no tools, no hooks, no settings.
    pub(crate) async fn bring(&self, thread: &mut Thread) -> Result<(), HarnessError> {
        let (Some(host), Some(project)) = (&self.projects, &thread.project) else {
            return Ok(());
        };
        let brought = if self.trusting(thread.for_) {
            host.brought(project).await
        } else {
            Brought::default()
        };
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

    /// Whether a thread for `person` is the person's the harness works for, whose trust turns
    /// a project's own on.
    pub(crate) fn trusting(&self, person: Option<glave_event::PrincipalId>) -> bool {
        person == self.actors.person
    }

    /// How `project` loosens the commands of its threads for the person the harness works
    /// for, as the host reads it now; nothing without a host.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thread_never_runs_in_auto_where_its_project_unconfines_it_without_auto() {
        use ApprovalMode::{Ask, Auto};
        let none = Loosened::default();
        assert_eq!(none.mode(None, Auto), Auto);
        assert_eq!(none.mode(Some(Ask), Auto), Ask, "its own choice");
        let auto = Loosened {
            approval: Some(Auto),
            unconfined: false,
        };
        assert_eq!(auto.mode(None, Ask), Auto, "the project's");
        assert_eq!(auto.mode(Some(Ask), Ask), Ask, "its own, nearer");
        let off = Loosened {
            approval: Some(Ask),
            unconfined: true,
        };
        assert_eq!(
            off.mode(Some(Auto), Auto),
            Ask,
            "auto chosen behind the sandbox"
        );
        let both = Loosened {
            approval: Some(Auto),
            unconfined: true,
        };
        assert_eq!(both.mode(None, Ask), Auto);
        assert_eq!(both.mode(Some(Ask), Ask), Ask);
    }
}
