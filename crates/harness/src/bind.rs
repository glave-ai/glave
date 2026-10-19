//! Where a thread works: a folder worked on in place, such as its project's, a git worktree
//! of its own of one on a `glave/<thread>` branch, a worker, or a scratch folder. The binding
//! is recorded as `workspace.bound`, and resuming the thread reads it to find the workspace
//! again, and the instructions it opens with.

use std::path::Path;
use std::sync::Arc;

use glave_event::{Body, ComputerKind, ComputerRef, Event, Opened, ThreadId, WorkspaceBound};
use glave_ports::computer::{ComputerError, ComputerId, Environment, Sources};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider, Scope, ToolSource};

use crate::approval::Approver;
use crate::builtins::{BUILTINS, Builtins};
use crate::harness::{Harness, HarnessError, Thread};
use crate::removed::owner_label;
use crate::tools::workspace::Workspace;

/// Where a new thread works.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    /// The folder at `path` on the harness's computer, such as its project's, worked on in
    /// place.
    Folder(String),
    /// A git worktree of its own of the repository around the folder at `path`, on the branch
    /// [`branch_for`] names, outside the repository.
    Worktree(String),
    /// A worker of its own, made from this environment by the harness's workers (see
    /// [`crate::Workers`]), with the project's repository cloned into it.
    Worker(Environment),
    /// A folder of its own, made empty on the harness's computer, in the same sandbox: for a
    /// thread with no project folder, such as a one-off chat in a person's own space.
    Scratch,
    /// A computer that is already running and dialled in, such as a person's own laptop
    /// serving a server elsewhere. Not available yet: starting a thread there is refused until
    /// computers can dial out to a server.
    Computer(ComputerId),
}

/// The branch a thread's worktree is on: `glave/` and the last 12 hex digits of the thread's
/// id, its random part, so threads started in the same moment never share one.
pub fn branch_for(thread: ThreadId) -> String {
    let id = thread.as_uuid().simple().to_string();
    let short = id.get(id.len().saturating_sub(12)..).unwrap_or(&id);
    format!("glave/{short}")
}

/// How many of `events` are glave's own record of where the thread works (bindings and
/// removals), of whether the thread is shown, and of its commands in the background, which
/// call ids do not count.
pub(crate) fn own_records(events: &[Event]) -> u64 {
    let count = events
        .iter()
        .filter(|event| {
            matches!(
                event.body,
                Body::WorkspaceBound(_)
                    | Body::WorkspaceRemoved(_)
                    | Body::ThreadArchived(_)
                    | Body::ThreadUnarchived(_)
                    | Body::ThreadDeleted(_)
                    | Body::ThreadRenamed(_)
                    | Body::ModelChosen(_)
                    | Body::ApprovalModeChosen(_)
                    | Body::CommandStarted(_)
                    | Body::CommandEnded(_)
            )
        })
        .count();
    u64::try_from(count).unwrap_or(u64::MAX)
}

/// The binding of a thread to `workspace`, as recorded.
pub(crate) fn bound(workspace: &Workspace, kind: ComputerKind) -> WorkspaceBound {
    let info = workspace.info();
    let opened = match &info.sources {
        Sources::Folder { path } => Opened::Folder { path: path.clone() },
        Sources::Project { .. } => Opened::Folder {
            path: info.root.clone(),
        },
        Sources::Worktree { path, branch, .. } => Opened::Worktree {
            repository: path.clone(),
            branch: branch.clone(),
        },
        Sources::Git { url, at } => Opened::Clone {
            url: url.clone(),
            at: at.clone(),
        },
        Sources::Scratch { .. } => Opened::Scratch {
            path: info.root.clone(),
        },
    };
    WorkspaceBound {
        workspace: info.id.as_uuid(),
        computer: ComputerRef {
            id: info.computer.as_uuid(),
            kind,
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

    /// The workspace for a thread `id` in `scope` at `place`, and its binding to record. A
    /// worktree is kept with the thread as its owner, for cleaning it up later.
    pub(crate) async fn place(
        &self,
        scope: Scope,
        id: ThreadId,
        place: Place,
    ) -> Result<(Option<Arc<Workspace>>, WorkspaceBound), HarnessError> {
        match place {
            Place::Folder(path) => {
                let folder = self.workspace.folder(&path).await?;
                let recorded = bound(&folder, ComputerKind::Local);
                Ok((Some(Arc::new(folder)), recorded))
            }
            Place::Worktree(path) => {
                let owner = owner_label(scope, id);
                let worktree = self
                    .workspace
                    .worktree(&path, &branch_for(id), Some(&owner))
                    .await?;
                let recorded = bound(&worktree, ComputerKind::Local);
                Ok((Some(Arc::new(worktree)), recorded))
            }
            Place::Worker(environment) => self.place_worker(&environment).await,
            Place::Computer(computer) => Err(HarnessError::Invalid(format!(
                "a thread on computer {computer} is not available yet: computers cannot dial in \
                 to a server until a later version"
            ))),
            Place::Scratch => {
                let owner = owner_label(scope, id);
                let scratch = self
                    .workspace
                    .scratch(&id.to_string(), Some(&owner))
                    .await?;
                let recorded = bound(&scratch, ComputerKind::Local);
                Ok((Some(Arc::new(scratch)), recorded))
            }
        }
    }

    /// The workspace a resumed thread's events bound it to, and that binding: its folder or
    /// worktree, opened again (a worktree made again from its branch if it was removed, and no
    /// longer held), or `None` for the harness's own. A folder that isn't there anymore, where
    /// its project's folder is now `moved` elsewhere, opens there instead, with the binding to
    /// record for it, and `true`.
    pub(crate) async fn reopen(
        &self,
        scope: Scope,
        id: ThreadId,
        events: &[Event],
        moved: Option<&str>,
    ) -> Result<(Option<Arc<Workspace>>, Option<WorkspaceBound>, bool), HarnessError> {
        let latest = events.iter().rev().find_map(|event| match &event.body {
            Body::WorkspaceBound(bound) => Some(bound),
            _ => None,
        });
        if latest.is_some_and(|bound| bound.computer.kind == ComputerKind::Worker) {
            return Err(ComputerError::Unsupported(
                "this thread worked on a cloud worker, which was released when its session \
                 ended; its work is on the branches it pushed. Start a new thread to go on"
                    .into(),
            )
            .into());
        }
        let owner = owner_label(scope, id);
        let elsewhere = |was: &str| moved.filter(|now| *now != was);
        let workspace = match latest.map(|bound| &bound.opened) {
            Some(Opened::Folder { path }) => {
                match (self.workspace.folder(path).await, elsewhere(path)) {
                    (Err(ComputerError::NotFound(_)), Some(now)) => {
                        let folder = self.workspace.folder(now).await?;
                        let recorded = bound(&folder, ComputerKind::Local);
                        return Ok((Some(Arc::new(folder)), Some(recorded), true));
                    }
                    (opened, _) => Some(opened?),
                }
            }
            Some(Opened::Worktree { repository, branch }) => {
                match self
                    .workspace
                    .worktree(repository, branch, Some(&owner))
                    .await
                {
                    Err(ComputerError::NotFound(_)) if elsewhere(repository).is_some() => {
                        return Err(ComputerError::Unsupported(
                            "this thread worked on a copy of its project made before the \
                             project's folder moved, and glave can't move that copy yet. Start \
                             a new thread to go on"
                                .into(),
                        )
                        .into());
                    }
                    opened => Some(opened?),
                }
            }
            // Made again, empty, if a cleanup removed it.
            Some(Opened::Scratch { .. }) => Some(
                self.workspace
                    .scratch(&id.to_string(), Some(&owner))
                    .await?,
            ),
            Some(Opened::Clone { .. }) | None => None,
        };
        Ok((workspace.map(Arc::new), latest.cloned(), false))
    }

    /// What a thread for `person` working where `bound` says opens with: what the host reads
    /// for its folder (its project's, or none for a scratch folder), when the harness has a
    /// reader and the thread a folder of its own here; else the settings' own instructions.
    pub(crate) async fn instructions_for(
        &self,
        workspace: Option<&Workspace>,
        bound: Option<&WorkspaceBound>,
        person: Option<glave_event::PrincipalId>,
    ) -> Option<String> {
        let (Some(reader), Some(workspace), Some(bound)) =
            (&self.project_folders, workspace, bound)
        else {
            return self.settings.instructions.clone();
        };
        let project = match &bound.opened {
            Opened::Folder { path } => Some(Path::new(path)),
            Opened::Worktree { repository, .. } => Some(Path::new(repository)),
            Opened::Scratch { .. } => None,
            Opened::Clone { .. } => return self.settings.instructions.clone(),
        };
        let trusting = self.trusting(person);
        reader.for_folder(workspace.root(), project, trusting).await
    }

    /// The workspace `thread`'s tools act on.
    pub(crate) fn workspace_for<'a>(&'a self, thread: &'a Thread) -> &'a Arc<Workspace> {
        thread.workspace.as_ref().unwrap_or(&self.workspace)
    }

    /// The tool source at `index` for `thread`, as [`Harness::tools`] numbers them: the
    /// built-in tools on its workspace, with its commands in the background, else the source
    /// [`Harness::sources_for`] lists there.
    pub(crate) fn source_for(&self, thread: &Thread, index: usize) -> Option<Arc<dyn ToolSource>> {
        match index {
            BUILTINS => Some(Arc::new(Builtins {
                workspace: Arc::clone(self.workspace_for(thread)),
                commands: Arc::clone(&thread.commands),
            })),
            _ => self.sources_for(thread).get(index).cloned(),
        }
    }
}
