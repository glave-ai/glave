//! What this machine holds: its computers and their open workspaces, with caps so the
//! record stays bounded.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use glave_ports::computer::{
    ComputerError, ComputerId, Environment, GitCredential, Sandbox, WorkspaceId,
};
use glave_ports::{Clock as _, Entropy as _, OsEntropy, SystemClock};
use uuid::{Builder, Uuid};

use super::sandbox::Prepared;

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
    /// A copy made for this workspace, removed when it closes; otherwise a folder worked on
    /// in place, which is kept.
    pub(crate) copy: bool,
    /// For a worktree, the repository's shared git folder, which its commands may also write
    /// in a sandbox, so that `git commit` works there.
    pub(crate) git_common: Option<PathBuf>,
    pub(crate) environment: Arc<Vec<(OsString, OsString)>>,
    /// Places its sandboxed commands may not read, beside the home folder's credential stores.
    pub(crate) hidden: Arc<[PathBuf]>,
    /// glave's own folder on its commands' `PATH`, which its sandbox keeps readable.
    pub(crate) glave_on_path: Option<PathBuf>,
    /// The credential its sources were opened with, kept in memory only, for saving them.
    pub(crate) credential: Option<GitCredential>,
    /// The sandbox its commands last ran in, prepared once and kept while they ask for the
    /// same one.
    pub(crate) prepared: Mutex<Option<Arc<Prepared>>>,
    /// Where its commands start, owned by it.
    pub(crate) processes: glave_process::Processes,
}

impl Opened {
    /// `sandbox`, prepared for this workspace.
    /// `helper` runs the Linux sandbox's helper (see [`crate::sandbox_helper`]).
    pub(crate) async fn sandbox(
        &self,
        sandbox: &Sandbox,
        helper: Option<&Path>,
    ) -> Result<Arc<Prepared>, ComputerError> {
        let kept = self
            .prepared
            .lock()
            .ok()
            .and_then(|kept| kept.clone())
            .filter(|kept| kept.sandbox == *sandbox);
        if let Some(kept) = kept {
            return Ok(kept);
        }
        let git_common = self.git_common.as_deref();
        let glave_on_path = self.glave_on_path.as_deref();
        let prepared = Prepared::new(
            sandbox,
            &self.root,
            git_common,
            helper,
            &self.hidden,
            glave_on_path,
        );
        let prepared = Arc::new(prepared.await?);
        if let Ok(mut kept) = self.prepared.lock() {
            *kept = Some(Arc::clone(&prepared));
        }
        Ok(prepared)
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
            return Err(ComputerError::Refused(format!(
                "this machine already holds {MAX_COMPUTERS} computers; destroy one first"
            )));
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
            return Err(ComputerError::Refused(format!(
                "{MAX_WORKSPACES} workspaces are already open; close one first"
            )));
        }
        held.workspaces.insert(id, Arc::new(opened));
        Ok(())
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
