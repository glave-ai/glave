//! The computer port: where work runs (`specs/computers.md`). A computer is made from an
//! environment, holds workspaces, and runs commands and file operations inside them, next to
//! the files. The calls are coarse and every type is serializable, so a computer on this
//! machine and one across the network answer the same calls in the same way.

mod background;
pub(crate) mod bytes;
mod credential;
mod environment;
mod finding;
mod limits;
mod refusal;
mod sandbox;
mod types;

use std::time::Duration;

use crate::tools::{BoxFuture, Cancellation};

pub use background::{Background, IDLE, KEPT_OUTPUT, MAX_BACKGROUND, MAX_WAIT, Printed};
pub use bytes::Bytes;
pub use credential::{CredentialAsk, CredentialHost, ForgeTool, GitCredential, GitCredentials};
pub use environment::{Environment, Image, Network, Size};
pub use finding::{Glob, Globbed, Match, Search, Searched, Stat, looks_binary};
pub use glave_event::Forge;
pub use limits::{Isolation, Limits};
pub use refusal::{Reason, Refusal};
pub use sandbox::{
    PROTECTED, Sandbox, SandboxLevel, is_protected, is_protected_refusal, refuse_protected,
};
pub use types::{
    Command, CommandId, ComputerError, ComputerId, ComputerInfo, Ending, Entry, EntryKind,
    FileWrite, Finished, Listed, Listing, OnTimeout, Pressure, Save, Saved, Sources, State,
    WorkspaceId, WorkspaceInfo,
};

/// A person's own computer, awake, and the workspace of their home on it: where their
/// threads open their folders, worktrees and scratch folders (`specs/computers.md`, "A
/// person's computer in the cloud").
#[derive(Clone)]
pub struct Home {
    pub computers: std::sync::Arc<dyn Computers>,
    pub home: WorkspaceInfo,
}

impl std::fmt::Debug for Home {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Home")
            .field("home", &self.home)
            .finish_non_exhaustive()
    }
}

/// Where a running command's output goes, as it arrives: standard output and standard error
/// together, as text. A slow sink slows the command's reader, never loses output.
pub trait Output: Send + Sync {
    fn write<'a>(&'a self, text: &'a str) -> BoxFuture<'a, ()>;
}

/// Computers and the workspaces on them. Placement happens behind this port: the caller asks
/// for a computer from an environment and a workspace on it, never for a machine.
pub trait Computers: Send + Sync {
    /// Makes a computer from `environment` (`computer/create`). An adapter that cannot honour
    /// the environment refuses it rather than quietly running something else.
    fn create<'a>(
        &'a self,
        environment: &'a Environment,
    ) -> BoxFuture<'a, Result<ComputerInfo, ComputerError>>;

    /// What the computer is and whether it is running (`computer/status`). A computer this
    /// adapter does not hold, destroyed or never made, is [`State::Gone`].
    fn status(&self, computer: ComputerId) -> BoxFuture<'_, Result<ComputerInfo, ComputerError>>;

    /// Removes the computer, its workspace copies and its disk (`computer/destroy`). Folders
    /// that were worked on in place stay.
    fn destroy(&self, computer: ComputerId) -> BoxFuture<'_, Result<(), ComputerError>>;

    /// Opens `sources` as a new workspace on `computer` (`workspace/open`).
    fn open<'a>(
        &'a self,
        computer: ComputerId,
        sources: &'a Sources,
    ) -> BoxFuture<'a, Result<WorkspaceInfo, ComputerError>>;

    /// Opens `sources` as [`Computers::open`] does, fetching (and later saving) them with
    /// `credential` (`workspace/open` with a `credential`). The computer keeps it in memory for
    /// the workspace's life and hands it to git through a credential helper for each use; it
    /// is never written to a disk, the repository's config, or the commands' environment. A
    /// computer that cannot keep a credential that way refuses it.
    fn open_with_credential<'a>(
        &'a self,
        computer: ComputerId,
        sources: &'a Sources,
        credential: &'a GitCredential,
    ) -> BoxFuture<'a, Result<WorkspaceInfo, ComputerError>> {
        let _ = (computer, sources, credential);
        Box::pin(std::future::ready(Err(ComputerError::Unsupported(
            "this computer cannot keep a credential for its sources".into(),
        ))))
    }

    /// Removes a workspace (`workspace/close`): a copy is deleted, a folder worked on in place
    /// is only forgotten.
    fn close(&self, workspace: WorkspaceId) -> BoxFuture<'_, Result<(), ComputerError>>;

    /// Runs one command at the workspace root (`command/run`), streaming its output to
    /// `output`, until it ends, times out (or moves to the background, as its [`OnTimeout`]
    /// asks), or `cancel` fires.
    fn run<'a>(
        &'a self,
        workspace: WorkspaceId,
        command: &'a Command,
        output: &'a dyn Output,
        cancel: &'a dyn Cancellation,
    ) -> BoxFuture<'a, Result<Finished, ComputerError>>;

    /// Runs one command as [`Computers::run`] does, where git and the forge's own tools may
    /// ask `credentials` for a credential each time they need one, for the hosts it names
    /// (`credentials/request`). The value reaches only the process that asked: never the
    /// command's environment, a file, or the model. A computer that cannot ask runs the
    /// command without them.
    fn run_with_credentials<'a>(
        &'a self,
        workspace: WorkspaceId,
        command: &'a Command,
        output: &'a dyn Output,
        cancel: &'a dyn Cancellation,
        credentials: &'a dyn GitCredentials,
    ) -> BoxFuture<'a, Result<Finished, ComputerError>> {
        let _ = credentials;
        self.run(workspace, command, output, cancel)
    }

    /// Starts one command at the workspace root and keeps it running in the background
    /// (`command/start`), in the sandbox it carries, until it ends, is stopped, its workspace
    /// closes, or nothing reads its output for [`IDLE`]. Its `timeout_secs`, when set, is its
    /// time limit there. A workspace keeps at most [`MAX_BACKGROUND`] running, and refuses one
    /// more. It is offered no credentials. A computer that cannot keep commands running
    /// answers unsupported.
    fn start<'a>(
        &'a self,
        workspace: WorkspaceId,
        command: &'a Command,
    ) -> BoxFuture<'a, Result<CommandId, ComputerError>> {
        let _ = (workspace, command);
        Box::pin(std::future::ready(Err(no_background())))
    }

    /// What a command in the background printed since the last read (`command/output`),
    /// waiting up to `wait` (at most [`MAX_WAIT`]) for it to end first. The read that answers
    /// its ending is its last: the computer then forgets it.
    fn output(
        &self,
        workspace: WorkspaceId,
        command: CommandId,
        wait: Duration,
    ) -> BoxFuture<'_, Result<Printed, ComputerError>> {
        let _ = (workspace, command, wait);
        Box::pin(std::future::ready(Err(no_background())))
    }

    /// How a command in the background ended, waiting up to `wait` (at most [`MAX_WAIT`]) for
    /// it to end, or nothing while it still runs (`command/ended`): however it ended, by itself,
    /// killed by another command, or stopped. Not a read: what it printed waits for the next
    /// one, and it is not forgotten.
    fn ended(
        &self,
        workspace: WorkspaceId,
        command: CommandId,
        wait: Duration,
    ) -> BoxFuture<'_, Result<Option<Finished>, ComputerError>> {
        let _ = (workspace, command, wait);
        Box::pin(std::future::ready(Err(no_background())))
    }

    /// Stops a command in the background and everything it started, then forgets it
    /// (`command/stop`), answering what it printed since the last read and how it ended. One
    /// that has already ended is only forgotten.
    fn stop(
        &self,
        workspace: WorkspaceId,
        command: CommandId,
    ) -> BoxFuture<'_, Result<Printed, ComputerError>> {
        let _ = (workspace, command);
        Box::pin(std::future::ready(Err(no_background())))
    }

    /// The commands in the background in a workspace, oldest first: running, or ended and not
    /// yet read (`command/list`).
    fn commands(
        &self,
        workspace: WorkspaceId,
    ) -> BoxFuture<'_, Result<Vec<Background>, ComputerError>> {
        let _ = workspace;
        Box::pin(std::future::ready(Err(no_background())))
    }

    /// Reads one file in the workspace (`files/read`).
    fn read<'a>(
        &'a self,
        workspace: WorkspaceId,
        path: &'a str,
    ) -> BoxFuture<'a, Result<Bytes, ComputerError>>;

    /// Writes files in the workspace, creating missing folders (`files/write`). Every path is
    /// checked before any file is written.
    fn write<'a>(
        &'a self,
        workspace: WorkspaceId,
        files: &'a [FileWrite],
    ) -> BoxFuture<'a, Result<(), ComputerError>>;

    /// Lists a folder in the workspace, or everything under it (`files/list`).
    fn list<'a>(
        &'a self,
        workspace: WorkspaceId,
        listing: &'a Listing,
    ) -> BoxFuture<'a, Result<Listed, ComputerError>>;

    /// What is at a path in the workspace (`files/stat`): where it resolves to, what it is,
    /// and for a file its digest. Nothing there yet is an answer, not an error.
    fn stat<'a>(
        &'a self,
        workspace: WorkspaceId,
        path: &'a str,
    ) -> BoxFuture<'a, Result<Stat, ComputerError>>;

    /// Finds files by name or path, newest first (`files/glob`).
    fn glob<'a>(
        &'a self,
        workspace: WorkspaceId,
        glob: &'a Glob,
    ) -> BoxFuture<'a, Result<Globbed, ComputerError>>;

    /// Finds lines matching a regular expression (`files/search`).
    fn search<'a>(
        &'a self,
        workspace: WorkspaceId,
        search: &'a Search,
    ) -> BoxFuture<'a, Result<Searched, ComputerError>>;

    /// Commits the workspace's changes to a branch and pushes it to where the sources came
    /// from (`workspace/save`): the only way work leaves a computer. A folder that is not a
    /// repository is refused.
    fn save<'a>(
        &'a self,
        workspace: WorkspaceId,
        save: &'a Save,
    ) -> BoxFuture<'a, Result<Saved, ComputerError>>;
}

/// What a computer that keeps no commands in the background answers.
fn no_background() -> ComputerError {
    ComputerError::Unsupported(
        "this computer cannot keep commands running in the background".into(),
    )
}
