//! The computer port: where work runs (`specs/computers.md`). A computer is made from an
//! environment, holds workspaces, and runs commands and file operations inside them, next to
//! the files. The calls are coarse and every type is serializable, so a computer on this
//! machine and one across the network answer the same calls in the same way.

mod bytes;
mod environment;
mod types;

use crate::tools::{BoxFuture, Cancellation};

pub use bytes::Bytes;
pub use environment::{Environment, Image, Network, Size};
pub use types::{
    Command, ComputerError, ComputerId, ComputerInfo, Ending, Entry, EntryKind, FileWrite,
    Finished, Listed, Listing, Save, Saved, Sources, State, WorkspaceId, WorkspaceInfo,
};

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

    /// Removes a workspace (`workspace/close`): a copy is deleted, a folder worked on in place
    /// is only forgotten.
    fn close(&self, workspace: WorkspaceId) -> BoxFuture<'_, Result<(), ComputerError>>;

    /// Runs one command at the workspace root (`command/run`), streaming its output to
    /// `output`, until it ends, times out, or `cancel` fires.
    fn run<'a>(
        &'a self,
        workspace: WorkspaceId,
        command: &'a Command,
        output: &'a dyn Output,
        cancel: &'a dyn Cancellation,
    ) -> BoxFuture<'a, Result<Finished, ComputerError>>;

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

    /// Commits the workspace's changes to a branch and pushes it to where the sources came
    /// from (`workspace/save`): the only way work leaves a computer. A plain folder gets a
    /// hidden repository of its own.
    fn save<'a>(
        &'a self,
        workspace: WorkspaceId,
        save: &'a Save,
    ) -> BoxFuture<'a, Result<Saved, ComputerError>>;
}
