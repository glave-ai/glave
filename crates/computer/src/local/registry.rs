//! What this machine holds: its computers and their open workspaces, with caps so the
//! record stays bounded.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use glave_ports::computer::{
    ComputerError, ComputerId, Environment, Refusal, Sandbox, SandboxNetwork, WorkspaceId,
};
use glave_ports::{Clock as _, Entropy as _, OsEntropy, SystemClock};
use tokio::sync::OnceCell;
use uuid::{Builder, Uuid};

use super::sandbox::{Outbound, Prepared};

/// The most computers one machine holds at once.
const MAX_COMPUTERS: usize = 64;
/// The most workspaces open at once, across its computers.
const MAX_WORKSPACES: usize = 1024;

/// An open workspace: where it is, and what its commands run with.
#[derive(Debug)]
pub(crate) struct Opened {
    pub(crate) computer: ComputerId,
    /// Canonical, so every path check compares like with like.
    pub(crate) root: PathBuf,
    /// For a worktree, the repository's shared git folder, which its commands may also write
    /// in a sandbox, so that `git commit` works there.
    pub(crate) git_common: Option<PathBuf>,
    pub(crate) environment: Arc<Vec<(OsString, OsString)>>,
    /// Places its sandboxed commands may not read, beside the home folder's credential stores.
    pub(crate) hidden: Arc<[PathBuf]>,
    /// glave's own folder on its commands' `PATH`, which its sandbox keeps readable.
    pub(crate) glave_on_path: Option<PathBuf>,
    /// The sandboxes its commands last ran in.
    pub(crate) prepared: Kept,
    /// Where its commands start, owned by it.
    pub(crate) processes: glave_process::Processes,
    /// How the outbound proxy its sandboxed commands may be kept to is made.
    pub(crate) outbound: Outbound,
}

/// The sandboxes a workspace's commands last ran in, each prepared once and kept while they ask
/// for the same one: the last kept to the outbound proxy apart from the last of any other, so a
/// command let out directly, or one whose network was taken, doesn't end the thread's network
/// and what listens there (`specs/sandbox.md`, "The outbound proxy").
#[derive(Debug, Default)]
pub(crate) struct Kept {
    proxied: Mutex<Option<Slot>>,
    other: Mutex<Option<Slot>>,
}

/// One kept sandbox, prepared once however many commands ask for it at the same time.
#[derive(Debug)]
struct Slot {
    sandbox: Sandbox,
    prepared: Arc<OnceCell<Arc<Prepared>>>,
}

impl Opened {
    /// `sandbox`, prepared for this workspace.
    /// `helper` runs the Linux sandbox's helper (see [`crate::sandbox_helper`]).
    pub(crate) async fn sandbox(
        &self,
        sandbox: &Sandbox,
        helper: Option<&Path>,
    ) -> Result<Arc<Prepared>, ComputerError> {
        let slot = if sandbox.network == SandboxNetwork::Proxied {
            &self.prepared.proxied
        } else {
            &self.prepared.other
        };
        let prepared = {
            let mut slot = slot.lock().unwrap_or_else(PoisonError::into_inner);
            let kept = slot.as_ref().filter(|kept| {
                kept.sandbox == *sandbox && kept.prepared.get().is_none_or(|kept| kept.lasts())
            });
            if let Some(kept) = kept {
                Arc::clone(&kept.prepared)
            } else {
                let prepared = Arc::new(OnceCell::new());
                *slot = Some(Slot {
                    sandbox: sandbox.clone(),
                    prepared: Arc::clone(&prepared),
                });
                prepared
            }
        };
        let prepared = prepared.get_or_try_init(|| async {
            let prepared = Prepared::new(
                sandbox,
                &self.root,
                self.git_common.as_deref(),
                (helper, &self.outbound, &self.processes),
                &self.hidden,
                self.glave_on_path.as_deref(),
            );
            prepared.await.map(Arc::new)
        });
        Ok(Arc::clone(prepared.await?))
    }
}

#[derive(Debug, Default)]
struct Held {
    computers: HashMap<ComputerId, Environment>,
    workspaces: HashMap<WorkspaceId, Arc<Opened>>,
}

#[derive(Debug, Default)]
pub(crate) struct Registry {
    held: Mutex<Held>,
}

/// A new UUID v7: this moment and the operating system's randomness, or why there is no
/// randomness: an id is never left with a zero-filled random part.
pub(crate) fn new_id() -> Result<Uuid, ComputerError> {
    let random = OsEntropy
        .bytes()
        .map_err(|error| ComputerError::Failed(error.to_string()))?;
    let millis = u64::try_from(SystemClock.now().as_millisecond()).unwrap_or(0);
    Ok(Builder::from_unix_timestamp_millis(millis, &random).into_uuid())
}

impl Registry {
    fn held(&self) -> Result<MutexGuard<'_, Held>, ComputerError> {
        self.held
            .lock()
            .map_err(|_| ComputerError::Failed("the computer registry's lock is poisoned".into()))
    }

    pub(crate) fn add_computer(
        &self,
        id: ComputerId,
        environment: Environment,
    ) -> Result<(), ComputerError> {
        let mut held = self.held()?;
        if held.computers.len() >= MAX_COMPUTERS {
            return Err(ComputerError::Refused(
                Refusal::TooManyComputers {
                    most: MAX_COMPUTERS.to_string(),
                }
                .into(),
            ));
        }
        held.computers.insert(id, environment);
        Ok(())
    }

    /// The computer's environment, or `None` when it is gone.
    pub(crate) fn environment(
        &self,
        computer: ComputerId,
    ) -> Result<Option<Environment>, ComputerError> {
        Ok(self.held()?.computers.get(&computer).cloned())
    }

    /// Forgets a computer and its workspaces.
    pub(crate) fn remove_computer(&self, computer: ComputerId) -> Result<(), ComputerError> {
        let mut held = self.held()?;
        if held.computers.remove(&computer).is_none() {
            return Err(ComputerError::NotFound(format!("no computer {computer}")));
        }
        held.workspaces
            .retain(|_, opened| opened.computer != computer);
        Ok(())
    }

    pub(crate) fn add_workspace(
        &self,
        id: WorkspaceId,
        opened: Opened,
    ) -> Result<(), ComputerError> {
        let mut held = self.held()?;
        if !held.computers.contains_key(&opened.computer) {
            return Err(ComputerError::NotFound(format!(
                "no computer {}",
                opened.computer
            )));
        }
        if held.workspaces.len() >= MAX_WORKSPACES {
            return Err(ComputerError::Refused(
                Refusal::TooManyWorkspaces {
                    most: MAX_WORKSPACES.to_string(),
                }
                .into(),
            ));
        }
        held.workspaces.insert(id, Arc::new(opened));
        Ok(())
    }

    /// Whether a workspace is open on `root`, or inside it.
    pub(crate) fn open_on(&self, root: &Path) -> Result<bool, ComputerError> {
        Ok(self
            .held()?
            .workspaces
            .values()
            .any(|opened| opened.root.starts_with(root)))
    }

    pub(crate) fn workspace(&self, id: WorkspaceId) -> Result<Arc<Opened>, ComputerError> {
        self.held()?
            .workspaces
            .get(&id)
            .cloned()
            .ok_or_else(|| ComputerError::NotFound(format!("no workspace {id}")))
    }

    pub(crate) fn remove_workspace(&self, id: WorkspaceId) -> Result<Arc<Opened>, ComputerError> {
        self.held()?
            .workspaces
            .remove(&id)
            .ok_or_else(|| ComputerError::NotFound(format!("no workspace {id}")))
    }
}
