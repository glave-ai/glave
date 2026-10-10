//! Where a thread works: the harness's workspace, the folder worked on in place, or a git
//! worktree of its own on a `glave/<thread>` branch. The binding is recorded as
//! `workspace.bound`, and resuming the thread reads it to find the workspace again.

use std::sync::Arc;

use glave_event::{Body, ComputerKind, ComputerRef, Event, Opened, ThreadId, WorkspaceBound};
use glave_ports::{BlobStore, EventLog, ModelProvider, Scope, Sources, ToolSource};

use crate::approval::Approver;
use crate::builtins::Builtins;
use crate::caller::Caller;
use crate::harness::{Harness, HarnessError, Thread};
use crate::tools::workspace::Workspace;

/// Where a new thread works.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Place {
    /// The harness's workspace, as it is: the folder the person is in.
    #[default]
    Here,
    /// A git worktree of its own, on the branch [`branch_for`] names, beside the repository.
    Worktree,
}

/// The branch a thread's worktree is on: `glave/` and the last 12 hex digits of the thread's
/// id, its random part, so threads started in the same moment never share one.
pub fn branch_for(thread: ThreadId) -> String {
    let id = thread.as_uuid().simple().to_string();
    let short = id.get(id.len().saturating_sub(12)..).unwrap_or(&id);
    format!("glave/{short}")
}

/// How many times `events` record a binding, which call ids do not count.
pub(crate) fn bindings(events: &[Event]) -> u64 {
    let count = events
        .iter()
        .filter(|event| matches!(event.body, Body::WorkspaceBound(_)))
        .count();
    u64::try_from(count).unwrap_or(u64::MAX)
}

/// The binding of a thread to `workspace`, as recorded.
fn bound(workspace: &Workspace) -> WorkspaceBound {
    let info = workspace.info();
    let opened = match &info.sources {
        Sources::Folder { path } => Opened::Folder { path: path.clone() },
        Sources::Worktree { path, branch } => Opened::Worktree {
            repository: path.clone(),
            branch: branch.clone(),
        },
        Sources::Git { url, at } => Opened::Clone {
            url: url.clone(),
            at: at.clone(),
        },
    };
    WorkspaceBound {
        workspace: info.id.as_uuid(),
        computer: ComputerRef {
            id: info.computer.as_uuid(),
            // Every computer a harness binds to is this machine's until workers come.
            kind: ComputerKind::Local,
        },
        opened,
        root: info.root.clone(),
    }
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Starts a new thread for `caller` working at `place`, and records where as
    /// `workspace.bound` right after `thread.started`. A worktree that cannot be made (the
    /// folder is not in a repository) fails the start, and nothing is recorded.
    pub async fn start_thread_in(
        &self,
        scope: Scope,
        caller: &Caller,
        place: Place,
    ) -> Result<Thread, HarnessError> {
        self.start_thread_from(scope, None, caller, Some(place))
            .await
    }

    /// The workspace for a thread `id` at `place`, and its binding to record.
    pub(crate) async fn place(
        &self,
        id: ThreadId,
        place: Place,
    ) -> Result<(Option<Arc<Workspace>>, WorkspaceBound), HarnessError> {
        match place {
            Place::Here => Ok((None, bound(&self.workspace))),
            Place::Worktree => {
                let worktree = self.workspace.worktree(&branch_for(id)).await?;
                let recorded = bound(&worktree);
                Ok((Some(Arc::new(worktree)), recorded))
            }
        }
    }

    /// The workspace a resumed thread's events bound it to: its worktree, opened again (made
    /// again from its branch if it was removed), or `None` for the harness's own.
    pub(crate) async fn reopen(
        &self,
        events: &[Event],
    ) -> Result<Option<Arc<Workspace>>, HarnessError> {
        let latest = events.iter().rev().find_map(|event| match &event.body {
            Body::WorkspaceBound(bound) => Some(bound),
            _ => None,
        });
        match latest.map(|bound| &bound.opened) {
            Some(Opened::Worktree { branch, .. }) => {
                Ok(Some(Arc::new(self.workspace.worktree(branch).await?)))
            }
            Some(Opened::Folder { .. } | Opened::Clone { .. }) | None => Ok(None),
        }
    }

    /// The workspace `thread`'s tools act on.
    pub(crate) fn workspace_for<'a>(&'a self, thread: &'a Thread) -> &'a Arc<Workspace> {
        thread.workspace.as_ref().unwrap_or(&self.workspace)
    }

    /// The tool source at `index` for `thread`: the built-in tools on its own workspace when
    /// it has one, or the source as the harness holds it.
    pub(crate) fn source_for(&self, thread: &Thread, index: usize) -> Option<Arc<dyn ToolSource>> {
        match (&thread.workspace, index) {
            (Some(workspace), 0) => Some(Arc::new(Builtins {
                workspace: Arc::clone(workspace),
            })),
            _ => self.sources.get(index).cloned(),
        }
    }
}
