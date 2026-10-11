//! Threads on workers: a disposable computer per thread, made from an environment, with the
//! project's repository cloned into it and the person's attachments copied in. The model is
//! told where its workspace came from and how work leaves it. Releasing the worker hands back
//! the files the model put out, pushes anything not yet pushed to `glave/wip/<thread>`, then
//! destroys it, and records `workspace.removed` with the reason `released`.

use std::sync::Arc;

use glave_event::{
    ComputerKind, Opened, RemovedReason, ThreadId, WorkspaceBound, WorkspaceRemoved,
};
use glave_ports::{
    BlobStore, Bytes, ComputerError, Computers, Ending, EntryKind, Environment, EventLog,
    FileWrite, Listing, ModelProvider, Save, Sources,
};

use crate::approval::Approver;
use crate::bind::{bound, branch_for};
use crate::harness::{Harness, HarnessError, Thread};
use crate::tools::workspace::Workspace;

/// Where a worker's attachments go, in its workspace.
pub const ATTACHMENTS: &str = ".glave/in";
/// Where the model puts files to hand back, in its workspace; never committed.
pub const HAND_BACK: &str = ".glave/out";
/// The most files handed back from one worker.
const MAX_HANDED_BACK: usize = 100;

/// Where threads on workers get their computer and their work.
#[derive(Clone)]
pub struct Workers {
    /// The computers workers are, such as Docker's.
    pub computers: Arc<dyn Computers>,
    /// What every worker's workspace is opened from: the project's repository, at a commit.
    pub sources: Sources,
    /// Files copied into every worker's workspace when it starts, under [`ATTACHMENTS`].
    pub attachments: Vec<FileWrite>,
}

impl std::fmt::Debug for Workers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Workers")
            .field("sources", &self.sources)
            .field("attachments", &self.attachments.len())
            .finish_non_exhaustive()
    }
}

/// What releasing a worker left behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleasedWorker {
    /// The branch unsaved work was pushed to, when there was any.
    pub branch: Option<String>,
    /// The files the model handed back, by their path under [`HAND_BACK`].
    pub files: Vec<(String, Bytes)>,
}

/// The branch a released worker's unpushed work goes to: `glave/wip/` and the thread's short id.
pub fn wip_branch(thread: ThreadId) -> String {
    branch_for(thread).replacen("glave/", "glave/wip/", 1)
}

/// What the model is told about a worker it works on: where its workspace came from, and how
/// work leaves it. `None` for anywhere else, so nothing the model sees changes there.
pub(crate) fn note(bound: &WorkspaceBound) -> Option<String> {
    if bound.computer.kind != ComputerKind::Worker {
        return None;
    }
    let from = match &bound.opened {
        Opened::Clone { url, at: Some(at) } => format!("a fresh clone of {url} at {at}"),
        Opened::Clone { url, at: None } => format!("a fresh clone of {url}"),
        Opened::Folder { path } => format!("the folder {path}"),
        Opened::Worktree { repository, .. } => format!("a worktree of {repository}"),
    };
    Some(format!(
        "You are working on a cloud worker: a disposable Linux computer started for this \
         thread. Your workspace is {from}. The worker is removed when this session ends, and \
         only what leaves it is kept: commit your work and push it to a branch (anything left \
         unpushed is pushed to a glave/wip/ branch when the worker is released). To hand a \
         file back to the person without committing it, put it in {HAND_BACK}/. Files the \
         person attached, if any, are in {ATTACHMENTS}/."
    ))
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// A worker of its own for a new thread: made from `environment`, the repository cloned
    /// into it, the attachments copied in. A worker that cannot be set up is destroyed.
    pub(crate) async fn place_worker(
        &self,
        environment: &Environment,
    ) -> Result<(Option<Arc<Workspace>>, WorkspaceBound), HarnessError> {
        let Some(workers) = &self.workers else {
            return Err(ComputerError::Unsupported(
                "no worker backend is configured: set [workers] in config.toml".into(),
            )
            .into());
        };
        let computers = &workers.computers;
        let computer = computers.create(environment).await?;
        let opened = async {
            let info = computers.open(computer.id, &workers.sources).await?;
            let files: Vec<FileWrite> = workers
                .attachments
                .iter()
                .map(|file| FileWrite {
                    path: format!("{ATTACHMENTS}/{}", file.path),
                    content: file.content.clone(),
                })
                .collect();
            if !files.is_empty() {
                computers.write(info.id, &files).await?;
            }
            Ok::<_, ComputerError>(info)
        };
        match opened.await {
            Ok(info) => {
                let workspace = Workspace::new(Arc::clone(computers), info);
                let recorded = bound(&workspace, ComputerKind::Worker);
                Ok((Some(Arc::new(workspace)), recorded))
            }
            Err(error) => {
                let _ = computers.destroy(computer.id).await;
                Err(error.into())
            }
        }
    }

    /// Releases the worker `thread` works on, if it works on one of its own: hands back the
    /// files under [`HAND_BACK`], pushes unpushed work to [`wip_branch`], destroys the worker,
    /// and records it. `None` for a thread on anything else, or a subagent sharing its
    /// parent's worker.
    pub async fn release_worker(
        &self,
        thread: &mut Thread,
    ) -> Result<Option<ReleasedWorker>, HarnessError> {
        let Some(workspace) = thread.workspace.clone() else {
            return Ok(None);
        };
        if thread.note.is_none() || thread.delegated() {
            return Ok(None);
        }
        let files = handed_back(&workspace).await?;
        let _ = workspace.quietly(&format!("rm -rf {HAND_BACK}")).await;
        let (ended, said) = workspace
            .quietly("git status --porcelain && git rev-list HEAD --not --remotes")
            .await?;
        let unsaved = ended.ending != (Ending::Exited { code: 0 }) || !said.trim().is_empty();
        let branch = if unsaved {
            let branch = wip_branch(thread.id);
            let save = Save {
                branch: branch.clone(),
                message: "glave: work in progress, saved when the worker was released".into(),
            };
            workspace
                .computers()
                .save(workspace.info().id, &save)
                .await?;
            Some(branch)
        } else {
            None
        };
        let info = workspace.info();
        workspace.computers().destroy(info.computer).await?;
        let removed = WorkspaceRemoved {
            root: info.root.clone(),
            branch: branch.clone(),
            reason: RemovedReason::Released,
            trash: None,
        };
        self.record(thread, &removed).await?;
        thread.note = None;
        Ok(Some(ReleasedWorker { branch, files }))
    }
}

/// The files the model put under [`HAND_BACK`], read whole.
async fn handed_back(workspace: &Workspace) -> Result<Vec<(String, Bytes)>, HarnessError> {
    let listing = Listing {
        path: HAND_BACK.into(),
        recursive: true,
        skip_ignored: false,
    };
    let listed = match workspace.list(&listing).await {
        Ok(listed) => listed,
        Err(ComputerError::NotFound(_)) => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut files = Vec::new();
    for entry in listed
        .entries
        .iter()
        .filter(|entry| entry.kind == EntryKind::File)
        .take(MAX_HANDED_BACK)
    {
        let content = workspace.read(&entry.path).await?;
        let name = entry
            .path
            .strip_prefix(HAND_BACK)
            .map_or(entry.path.as_str(), |rest| rest.trim_start_matches('/'));
        files.push((name.to_owned(), content));
    }
    Ok(files)
}
