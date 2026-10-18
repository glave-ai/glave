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

use glave_event::ApprovalMode;
use glave_ports::{
    BlobStore, BoxFuture, EventLog, ModelProvider, Sandbox, SandboxLevel, SpaceId, ToolSource,
};

use crate::approval::Approver;
use crate::harness::{Harness, HarnessError, Thread};
use crate::hooks::Hooks;
use crate::judge::Judge;

/// What a thread working in a folder of its own opens with, and what its project chooses and
/// turns on, as the host reads them. Without one, every thread opens with the settings'
/// instructions and gets only the harness's own settings, tools and hooks.
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

    /// What the project whose folder is `project` chooses for its threads now. It starts
    /// nothing, only reads. Nothing by default.
    fn settled<'a>(&'a self, project: &'a Path) -> BoxFuture<'a, Settled> {
        let _ = project;
        Box::pin(async { Settled::default() })
    }

    /// What the project whose folder is `project`, the space `space`'s, chooses and turns on
    /// for its threads now: its choices, its trusted tools (started the first time they are
    /// asked for) and its trusted hooks. Nothing by default.
    fn brought<'a>(&'a self, project: &'a Path, space: SpaceId) -> BoxFuture<'a, Brought> {
        let _ = (project, space);
        Box::pin(async { Brought::default() })
    }
}

/// How a project's settings change the protections its threads' commands run under.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Protections {
    /// The mode its threads run in when they chose none: its own choice, `auto` once trusted;
    /// `ask` when the project turned the sandbox off with auto chosen nowhere; none to leave
    /// the harness's. Always set where its commands run unconfined.
    pub approval: Option<ApprovalMode>,
    /// Whether its threads' commands run without the sandbox, once trusted.
    pub unconfined: bool,
    /// A narrower sandbox than the harness's for its threads' commands.
    pub level: Option<SandboxLevel>,
    /// Whether its threads' sandboxed commands go without the network.
    pub offline: bool,
}

impl Protections {
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

    /// The sandbox its threads' commands run in, where the harness's is `own`: none where it
    /// turned the sandbox off, else `own` narrowed as it asks.
    pub fn sandbox(self, own: Option<&Sandbox>) -> Option<Sandbox> {
        if self.unconfined {
            return None;
        }
        own.map(|own| Sandbox {
            level: self.level.unwrap_or(own.level),
            network: own.network && !self.offline,
            ..own.clone()
        })
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
    /// Reads again what `thread`'s project chooses and turns on, at the start of a turn, and
    /// gives its commands the sandbox that follows: none where the project turned it off, else
    /// the harness's own as the project narrows it. A thread with no project of its own here
    /// gets nothing, and so does another person's thread than the one the harness works for,
    /// who trusted the project: no settings, no tools, no hooks.
    pub(crate) async fn bring(&self, thread: &mut Thread) -> Result<(), HarnessError> {
        let (Some(host), Some(project)) = (&self.projects, &thread.project) else {
            return Ok(());
        };
        let brought = if self.trusting(thread.for_) {
            host.brought(project, thread.scope.space).await
        } else {
            Brought::default()
        };
        if let Some(workspace) = &thread.workspace {
            let wanted = brought
                .settled
                .protections
                .sandbox(self.workspace.sandbox());
            if workspace.sandbox() != wanted.as_ref() {
                thread.workspace = Some(Arc::new(workspace.within(wanted.as_ref()).await?));
            }
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
        match &self.projects {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thread_never_runs_in_auto_where_its_project_unconfines_it_without_auto() {
        use ApprovalMode::{Ask, Auto};
        let none = Protections::default();
        assert_eq!(none.mode(None, Auto), Auto);
        assert_eq!(none.mode(Some(Ask), Auto), Ask, "its own choice");
        let auto = Protections {
            approval: Some(Auto),
            ..Protections::default()
        };
        assert_eq!(auto.mode(None, Ask), Auto, "the project's");
        assert_eq!(auto.mode(Some(Ask), Ask), Ask, "its own, nearer");
        let off = Protections {
            approval: Some(Ask),
            unconfined: true,
            ..Protections::default()
        };
        assert_eq!(
            off.mode(Some(Auto), Auto),
            Ask,
            "auto chosen behind the sandbox"
        );
        let both = Protections {
            approval: Some(Auto),
            unconfined: true,
            ..Protections::default()
        };
        assert_eq!(both.mode(None, Ask), Auto);
        assert_eq!(both.mode(Some(Ask), Ask), Ask);
    }

    #[test]
    fn a_project_narrows_its_threads_sandbox_or_turns_it_off_and_never_widens_it() {
        let own = Sandbox {
            level: SandboxLevel::Project,
            network: true,
            home: None,
            glave_home: None,
        };
        assert_eq!(
            Protections::default().sandbox(Some(&own)),
            Some(own.clone())
        );
        let narrowed = Protections {
            level: Some(SandboxLevel::ReadOnly),
            offline: true,
            ..Protections::default()
        };
        let sandbox = narrowed.sandbox(Some(&own)).unwrap();
        assert_eq!(
            (sandbox.level, sandbox.network),
            (SandboxLevel::ReadOnly, false)
        );
        assert_eq!(narrowed.sandbox(None), None, "no sandbox here to narrow");
        let off = Protections {
            unconfined: true,
            ..Protections::default()
        };
        assert_eq!(off.sandbox(Some(&own)), None);
    }
}
