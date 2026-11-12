//! The computer port: where work runs (`specs/computers.md`). A computer is made from an
//! environment, holds workspaces, and runs commands and file operations inside them, next to
//! the files. The calls are coarse and every type is serializable, so a computer on this
//! machine and one across the network answer the same calls in the same way.

mod background;
pub(crate) mod bytes;
mod changes;
mod credential;
mod environment;
mod finding;
mod limits;
mod refusal;
mod sandbox;
mod terminal;
mod types;

use std::time::Duration;

use crate::tools::{BoxFuture, Cancellation};

pub use background::{Background, IDLE, KEPT_OUTPUT, Listening, MAX_BACKGROUND, MAX_WAIT, Printed};
pub use bytes::Bytes;
pub use changes::{ChangeKind, ChangedFile, Changes, DiffLine, FileChange, LineKind, Since};
pub use credential::{CredentialAsk, CredentialHost, ForgeTool, GitCredential, GitCredentials};
pub use environment::{Environment, Image, Network, Size, is_variable_name};
pub use finding::{Glob, Globbed, Match, Search, Searched, Stat, looks_binary};
pub use glave_event::Forge;
pub use limits::{Isolation, Limits};
pub use refusal::{Reason, Refusal};
pub use sandbox::{
    PROTECTED, Sandbox, SandboxLevel, git_settings, is_kept, is_protected, is_protected_refusal,
    refuse_protected,
};
pub use terminal::{
    MAX_OWNED_TERMINALS, MAX_TERMINALS, Screenful, TERMINAL_CHUNK, TERMINAL_KEPT, TerminalInfo,
    TerminalSize,
};
pub use types::{
    Command, CommandId, ComputerError, ComputerId, ComputerInfo, DiskSpace, DiskUse, Ending, Entry,
    EntryKind, FileWrite, Finished, Freed, KeptWorktree, Listed, Listing, OnTimeout, Save, Saved,
    Sources, State, TerminalId, WorkspaceId, WorkspaceInfo,
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

    /// Opens `sources` again as [`Computers::open`] does, for a thread whose log already says
    /// where their history stood: nothing reads [`WorkspaceInfo::at`], which a computer for
    /// which asking costs something leaves out.
    fn reopen<'a>(
        &'a self,
        computer: ComputerId,
        sources: &'a Sources,
    ) -> BoxFuture<'a, Result<WorkspaceInfo, ComputerError>> {
        self.open(computer, sources)
    }

    /// Lets go of a workspace (`workspace/close`): its folder is kept, only forgotten.
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

    /// Where the commands still running in the background in a workspace listen: those whose
    /// processes listen on a TCP address the computer's own loopback reaches, oldest first.
    /// Asked for only where something reaches that loopback, as finding them reads every
    /// process's sockets: this machine alone, for now (`specs/computers.md`). A computer
    /// that cannot tell answers unsupported.
    fn listening(
        &self,
        workspace: WorkspaceId,
    ) -> BoxFuture<'_, Result<Vec<Listening>, ComputerError>> {
        let _ = workspace;
        Box::pin(std::future::ready(Err(ComputerError::Unsupported(
            "this computer cannot tell where its commands listen".into(),
        ))))
    }

    /// Opens a terminal of the person's own on `computer` for `owner` (a thread, as the caller
    /// names it) (`terminal/open`): their login shell, at `folder`, on a pseudo-terminal of
    /// `size`, outside every sandbox, in a session of its own that its computer ends with
    /// everything in it. It stays open until it is closed, its computer is destroyed or goes
    /// away, or nothing uses it for [`IDLE`]: no read that found something printed, and no
    /// write. A computer keeps at most [`MAX_TERMINALS`] open and [`MAX_OWNED_TERMINALS`] for
    /// one owner, and refuses one more. A computer that opens no terminals answers unsupported.
    fn open_terminal<'a>(
        &'a self,
        computer: ComputerId,
        folder: &'a str,
        owner: &'a str,
        size: TerminalSize,
    ) -> BoxFuture<'a, Result<TerminalId, ComputerError>> {
        let _ = (computer, folder, owner, size);
        Box::pin(std::future::ready(Err(no_terminals())))
    }

    /// The terminals open on `computer` for `owner`, oldest first, ended ones too
    /// (`terminal/list`); none where it opens none.
    fn terminals<'a>(
        &'a self,
        computer: ComputerId,
        owner: &'a str,
    ) -> BoxFuture<'a, Result<Vec<TerminalInfo>, ComputerError>> {
        let _ = (computer, owner);
        Box::pin(std::future::ready(Ok(Vec::new())))
    }

    /// Types `input` into a terminal of `owner`'s (`terminal/write`), at most
    /// [`TERMINAL_CHUNK`] bytes. Each call on a terminal names its owner, which its computer
    /// checks: another owner's is not found.
    fn write_terminal<'a>(
        &'a self,
        computer: ComputerId,
        (terminal, owner): (TerminalId, &'a str),
        input: &'a Bytes,
    ) -> BoxFuture<'a, Result<(), ComputerError>> {
        let _ = (computer, terminal, owner, input);
        Box::pin(std::future::ready(Err(no_terminals())))
    }

    /// What a terminal of `owner`'s printed from offset `from` (`terminal/read`), at most
    /// [`TERMINAL_CHUNK`] bytes, waiting up to `wait` (at most [`MAX_WAIT`]) for something to
    /// read when nothing is there yet (a read that finds nothing does not use it); a `from`
    /// past what it printed reads from its end. Never forgets it: a terminal whose shell ended
    /// stays until it is closed.
    fn read_terminal<'a>(
        &'a self,
        computer: ComputerId,
        (terminal, owner): (TerminalId, &'a str),
        from: u64,
        wait: Duration,
    ) -> BoxFuture<'a, Result<Screenful, ComputerError>> {
        let _ = (computer, terminal, owner, from, wait);
        Box::pin(std::future::ready(Err(no_terminals())))
    }

    /// Gives a terminal of `owner`'s another size (`terminal/resize`), which its shell is told
    /// of.
    fn resize_terminal<'a>(
        &'a self,
        computer: ComputerId,
        (terminal, owner): (TerminalId, &'a str),
        size: TerminalSize,
    ) -> BoxFuture<'a, Result<(), ComputerError>> {
        let _ = (computer, terminal, owner, size);
        Box::pin(std::future::ready(Err(no_terminals())))
    }

    /// Closes a terminal of `owner`'s (`terminal/close`): hangs it up, as a terminal's hang-up
    /// does, ends what is left in its session, and forgets it.
    fn close_terminal<'a>(
        &'a self,
        computer: ComputerId,
        (terminal, owner): (TerminalId, &'a str),
    ) -> BoxFuture<'a, Result<(), ComputerError>> {
        let _ = (computer, terminal, owner);
        Box::pin(std::future::ready(Err(no_terminals())))
    }

    /// The worktrees and scratch folders it keeps for threads (`computer/worktrees`); none where
    /// it keeps none.
    fn worktrees(
        &self,
        computer: ComputerId,
    ) -> BoxFuture<'_, Result<Vec<KeptWorktree>, ComputerError>> {
        let _ = computer;
        Box::pin(std::future::ready(Ok(Vec::new())))
    }

    /// Frees the worktree at `path`, when what it holds can be rebuilt: nothing uncommitted,
    /// every commit on a remote, no file git ignores but build outputs, and no workspace open on
    /// it, all looked at again now (`computer/free_worktree`). Its branch stays. A scratch folder
    /// with no workspace open on it goes whatever it holds: removed when empty, else moved to the
    /// Trash.
    fn free_worktree<'a>(
        &'a self,
        computer: ComputerId,
        path: &'a str,
    ) -> BoxFuture<'a, Result<Freed, ComputerError>> {
        let _ = (computer, path);
        Box::pin(std::future::ready(Ok(Freed::Kept {
            why: "this computer keeps no worktrees".to_owned(),
        })))
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

    /// What changed in the workspace's folder since `since`, a point in its history as
    /// [`WorkspaceInfo::at`] named one, or since where its history stands now when that is
    /// `None` or gone (`files/changes`): every file that differs, whatever changed it, new
    /// files not ignored among them. `None` for a folder that keeps no history. Asking never
    /// wakes a computer that sleeps: asleep, it answers unsupported.
    fn changes<'a>(
        &'a self,
        workspace: WorkspaceId,
        since: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Option<Changes>, ComputerError>> {
        let _ = (workspace, since);
        Box::pin(std::future::ready(Err(ComputerError::Unsupported(
            "this computer cannot say what changed in its folders".into(),
        ))))
    }

    /// Commits the workspace's changes to a branch and pushes it to the repository's `origin`
    /// (`workspace/save`), when it has one. A folder that is not a repository is refused.
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

/// What a computer that opens no terminals answers.
fn no_terminals() -> ComputerError {
    ComputerError::Unsupported("this computer opens no terminals".into())
}
