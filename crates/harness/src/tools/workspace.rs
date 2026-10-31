//! The workspace the tools act on: a binding to one workspace on a computer, reached through
//! the computer port, so the same tools work on this machine or across the wire. The
//! computer confines every path and runs the commands; the binding adds what the harness
//! decides: the sandbox each command runs in, where its git asks for credentials, the paths
//! the file tools may not change in it, and what the model has read, so writes and edits
//! never act on a stale view.

mod folders;

pub use folders::{Inside, Looked};

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use glave_event::Sha256;
use glave_ports::computer::{
    Bytes, Command, CommandId, ComputerError, Computers, Ending, FileWrite, Finished,
    GitCredentials, Glob, Globbed, Listed, Listing, OnTimeout, Output, Sandbox, Search, Searched,
    Sources, Stat, WorkspaceInfo, is_protected, is_protected_refusal, refuse_protected,
};
use glave_ports::{Cancellation, digest};

use crate::cancel::Cancel;

/// Seconds the sandbox has to run its first command, which prints one word: it starts in
/// well under a second, and one that has not by then is stuck (on a named pipe, say), so
/// glave says so at once rather than after a command's whole timeout.
const SANDBOX_STARTS_WITHIN: u64 = 10;

/// One workspace on one computer, the sandbox its commands run in, and what the model has
/// seen there.
pub struct Workspace {
    computers: Arc<dyn Computers>,
    info: WorkspaceInfo,
    root: PathBuf,
    sandbox: Option<Sandbox>,
    /// Whether a command whose sandbox has no network may ask the person to run with it, or
    /// outside the sandbox, which has it; on by default.
    asks_network: bool,
    /// Whether its commands work for a plan (plan mode): they may never ask to leave the
    /// sandbox, whatever it has. Off by default.
    planning: bool,
    /// Where commands' git and forge tools ask for credentials, at each use; none by default.
    credentials: Option<Arc<dyn GitCredentials>>,
    temp: Option<PathBuf>,
    /// Whether its computer may be asleep, as a person's may: asking it anything before a call
    /// needs it would wake it.
    asleep_at_times: bool,
    /// What the model last read of each file, by its path in the workspace.
    reads: Mutex<HashMap<String, Sha256>>,
}

/// What a command is told where a person's computer can't run the sandbox it's set to.
const NO_SANDBOX_THERE: &str = "the sandbox can't start on this person's computer, so the command \
     did not run. Whoever runs this server has to run people's computers where it can start, or \
     set them to go without it.";

impl fmt::Debug for Workspace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Workspace")
            .field("info", &self.info)
            .field("sandbox", &self.sandbox)
            .finish_non_exhaustive()
    }
}

impl Workspace {
    /// Binds the tools to `workspace`, opened on `computers`.
    pub fn new(computers: Arc<dyn Computers>, workspace: WorkspaceInfo) -> Self {
        Self {
            root: PathBuf::from(&workspace.root),
            temp: workspace.temp.as_ref().map(PathBuf::from),
            computers,
            info: workspace,
            sandbox: None,
            asks_network: true,
            planning: false,
            credentials: None,
            asleep_at_times: false,
            reads: Mutex::new(HashMap::new()),
        }
    }

    /// The same binding on a computer that may be asleep, as a person's may: a sandbox is taken
    /// without asking it, so no sandbox wakes it, and the first command to reach it finds out,
    /// refused by a computer that cannot enforce it.
    #[must_use]
    pub(crate) fn asleep_at_times(mut self) -> Self {
        self.asleep_at_times = true;
        self
    }

    /// Lets a command whose sandbox has no network ask the person to run with it, or outside
    /// the sandbox, which has it, or not (`asks`): an organization's policy can forbid it. The
    /// worktrees and scratch folders made from this workspace keep it.
    #[must_use]
    pub fn asking_network(mut self, asks: bool) -> Self {
        self.asks_network = asks;
        self
    }

    /// Whether a command whose sandbox has no network may ask for it.
    pub(crate) fn asks_network(&self) -> bool {
        self.asks_network
    }

    /// The same binding for a thread in plan mode, or not (`planning`): there no command may
    /// ask to leave the sandbox.
    #[must_use]
    pub(crate) fn planning(mut self, planning: bool) -> Self {
        self.planning = planning;
        self
    }

    /// Whether its commands work for a plan, and may never ask to leave the sandbox.
    pub(crate) fn plans(&self) -> bool {
        self.planning
    }

    /// Lets every command's git and forge tools ask `credentials` for a credential each time
    /// they need one (`specs/computers.md`, rule 8), as do the worktrees and scratch folders
    /// made from this workspace.
    #[must_use]
    pub fn with_credentials(mut self, credentials: Arc<dyn GitCredentials>) -> Self {
        self.credentials = Some(credentials);
        self
    }

    /// Where commands ask for credentials, if anywhere.
    pub(crate) fn credentials(&self) -> Option<&Arc<dyn GitCredentials>> {
        self.credentials.as_ref()
    }

    /// The same binding's credentials, and what its commands may ask for, given to `other`,
    /// made from it.
    fn sharing(&self, other: Self) -> Self {
        let other = Self {
            asks_network: self.asks_network,
            planning: self.planning,
            asleep_at_times: self.asleep_at_times,
            ..other
        };
        match &self.credentials {
            Some(credentials) => other.with_credentials(Arc::clone(credentials)),
            None => other,
        }
    }

    /// Runs every command in `sandbox`. Fails if the computer cannot enforce it, so a
    /// requested sandbox is never quietly skipped, and within 10 seconds when it starts no
    /// command; on success, learns the temp folder its commands get, which the shell
    /// assessment treats as theirs to write. On a computer that may be asleep nothing is asked
    /// (`asleep_at_times`).
    pub async fn with_sandbox(mut self, sandbox: &Sandbox) -> Result<Self, ComputerError> {
        if self.asleep_at_times {
            self.sandbox = Some(sandbox.clone());
            return Ok(self);
        }
        let mut probe = Command::new(r#"printf '%s' "$TMPDIR""#);
        probe.sandbox = Some(sandbox.clone());
        probe.timeout_secs = Some(SANDBOX_STARTS_WITHIN);
        let temp = Collected::default();
        let finished = self
            .computers
            .run(self.info.id, &probe, &temp, &Cancel::new())
            .await?;
        let temp = temp.text();
        if let Ending::TimedOut { after_secs } = finished.ending {
            return Err(ComputerError::Failed(format!(
                "a command in the sandbox did not start within {}",
                glave_ports::spoken(std::time::Duration::from_secs(after_secs))
            )));
        }
        if finished.ending != (Ending::Exited { code: 0 }) || temp.is_empty() {
            return Err(ComputerError::Failed(format!(
                "a command in the sandbox did not run: {temp}"
            )));
        }
        self.temp = Some(PathBuf::from(temp));
        self.sandbox = Some(sandbox.clone());
        Ok(self)
    }

    /// A worktree of the repository around the folder at `path`, on `branch`, on the same
    /// computer and in the same sandbox: the one already there for that branch, or a new one
    /// made from the folder's current commit. `owner` is kept beside it, for whoever cleans it
    /// up later.
    pub async fn worktree(
        &self,
        path: &str,
        branch: &str,
        owner: Option<&str>,
    ) -> Result<Self, ComputerError> {
        self.beside(&Sources::Worktree {
            path: path.to_owned(),
            branch: branch.to_owned(),
            owner: owner.map(str::to_owned),
        })
        .await
    }

    /// A scratch folder of its own named `name`, on the same computer and in the same sandbox:
    /// the one already there, or a new, empty one. `owner` is kept beside it, as for a worktree.
    pub async fn scratch(&self, name: &str, owner: Option<&str>) -> Result<Self, ComputerError> {
        self.beside(&Sources::Scratch {
            name: name.to_owned(),
            owner: owner.map(str::to_owned),
        })
        .await
    }

    /// The folder at `path` on the same computer, worked on in place, in the same sandbox and
    /// with the same credentials, such as a project's.
    pub async fn folder(&self, path: &str) -> Result<Self, ComputerError> {
        self.beside(&Sources::Folder {
            path: path.to_owned(),
        })
        .await
    }

    /// `sources` opened as another workspace on the same computer, in the same sandbox and
    /// with the same credentials.
    async fn beside(&self, sources: &Sources) -> Result<Self, ComputerError> {
        let info = self.computers.open(self.info.computer, sources).await?;
        let opened = self.sharing(Self::new(Arc::clone(&self.computers), info));
        match &self.sandbox {
            Some(sandbox) => opened.with_sandbox(sandbox).await,
            None => Ok(opened),
        }
    }

    /// The same binding, with what the model has read there, its commands in `sandbox`, or in
    /// none: a project's trusted settings can turn the sandbox off for its threads.
    pub(crate) async fn within(&self, sandbox: Option<&Sandbox>) -> Result<Self, ComputerError> {
        let same = self.sharing(Self::new(Arc::clone(&self.computers), self.info.clone()));
        let reads = self
            .reads
            .lock()
            .map(|reads| reads.clone())
            .unwrap_or_default();
        let same = Self {
            reads: Mutex::new(reads),
            ..same
        };
        match sandbox {
            Some(sandbox) => same.with_sandbox(sandbox).await,
            None => Ok(same),
        }
    }

    /// The computers this workspace is on.
    pub(crate) fn computers(&self) -> &Arc<dyn Computers> {
        &self.computers
    }

    /// Runs `script` for glave itself, not the model: its output whole, and how it ended.
    pub(crate) async fn quietly(&self, script: &str) -> Result<(Finished, String), ComputerError> {
        let output = Collected::default();
        let finished = self
            .run(
                script.to_owned(),
                (120, OnTimeout::Stop),
                &output,
                &Cancel::new(),
            )
            .await?;
        Ok((finished, output.text()))
    }

    /// The sandbox commands run in, if one was asked for.
    pub(crate) fn sandbox(&self) -> Option<&Sandbox> {
        self.sandbox.as_ref()
    }

    /// The workspace root, as its computer names it.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Which workspace this is, on which computer.
    pub fn info(&self) -> &WorkspaceInfo {
        &self.info
    }

    /// The home folder commands see, if known.
    pub(crate) fn home(&self) -> Option<&Path> {
        self.info.home.as_deref().map(Path::new)
    }

    /// The temp folder commands see, if known.
    pub(crate) fn temp(&self) -> Option<&Path> {
        self.temp.as_deref()
    }

    /// What is at a path the model gave, confined to the workspace. A refusal is a sentence
    /// for the model.
    pub(crate) async fn stat(&self, path: &str) -> Result<Stat, String> {
        self.computers
            .stat(self.info.id, path)
            .await
            .map_err(|error| explain(&error))
    }

    /// What is at a path the model wants to change. In a sandbox, paths that make code run
    /// later (git hooks and config, glave's settings) are refused here first, before the
    /// model is asked to read a file it may not change anyway. Symlinks are resolved first, so
    /// a planted link is caught. The computer refuses them again on the write itself
    /// ([`Self::write`]), and that refusal is the one that holds while a command races.
    pub(crate) async fn stat_for_change(&self, path: &str) -> Result<Stat, String> {
        let stat = self.stat(path).await?;
        if self.sandbox.is_some() && is_protected(&self.root, &self.root.join(&stat.path)) {
            return Err(refuse_protected(&stat.path).message().to_owned());
        }
        Ok(stat)
    }

    pub(crate) async fn read(&self, path: &str) -> Result<Bytes, ComputerError> {
        self.computers.read(self.info.id, path).await
    }

    /// Writes `content` to `path`, a path [`Self::stat_for_change`] showed. In a sandbox the
    /// computer refuses a protected path on the path it opens, so a folder a command swapped
    /// for a link since then cannot slip one in. A failure is a sentence for the model: that
    /// refusal as the stat check words it, or what went wrong.
    pub(crate) async fn write(&self, path: &str, content: &[u8]) -> Result<(), String> {
        let file = FileWrite {
            protect: self.sandbox.is_some(),
            ..FileWrite::new(path, Bytes::new(content))
        };
        let written = self
            .computers
            .write(self.info.id, std::slice::from_ref(&file))
            .await;
        written.map_err(|error| {
            if is_protected_refusal(&error) {
                error.message().to_owned()
            } else {
                format!("Could not write {path}: {}", error.message())
            }
        })
    }

    pub(crate) async fn list(&self, listing: &Listing) -> Result<Listed, ComputerError> {
        self.computers.list(self.info.id, listing).await
    }

    pub(crate) async fn glob(&self, glob: &Glob) -> Result<Globbed, String> {
        self.computers
            .glob(self.info.id, glob)
            .await
            .map_err(|error| explain(&error))
    }

    pub(crate) async fn search(&self, search: &Search) -> Result<Searched, String> {
        self.computers
            .search(self.info.id, search)
            .await
            .map_err(|error| explain(&error))
    }

    /// Runs `command` in this workspace, in its sandbox if it has one, with its credentials if
    /// it has them, doing at its timeout what `on_timeout` says.
    pub(crate) async fn run(
        &self,
        script: String,
        (timeout_secs, on_timeout): (u64, OnTimeout),
        output: &dyn Output,
        cancel: &dyn Cancellation,
    ) -> Result<Finished, ComputerError> {
        let command = Command {
            script,
            timeout_secs: Some(timeout_secs),
            sandbox: self.sandbox.clone(),
            on_timeout,
        };
        let span = crate::telemetry::command();
        let running = match &self.credentials {
            Some(credentials) => self.computers.run_with_credentials(
                self.info.id,
                &command,
                output,
                cancel,
                credentials.as_ref(),
            ),
            None => self.computers.run(self.info.id, &command, output, cancel),
        };
        let finished = tracing::Instrument::instrument(running, span.clone()).await;
        let ended = finished.as_ref().ok();
        crate::telemetry::command_ended(&span, ended.map(|done| (&done.ending, done.millis)));
        finished.map_err(|error| self.said_plainly(error))
    }

    /// `error`, a command's: where a person's computer refuses the sandbox its server set it to,
    /// that is a mistake in the server's settings no one at the computer can act on, so it says
    /// so to them, and what the computer said goes in the server's log.
    fn said_plainly(&self, error: ComputerError) -> ComputerError {
        if !self.asleep_at_times || self.sandbox.is_none() {
            return error;
        }
        match error {
            ComputerError::Unsupported(why) => {
                tracing::warn!(%why, "a person's computer cannot run the sandbox it is set to");
                ComputerError::Unsupported(NO_SANDBOX_THERE.into())
            }
            other => other,
        }
    }

    /// Starts `script` in this workspace, in its sandbox if it has one, and keeps it running in
    /// the background. Its git and forge tools get no credentials there.
    pub(crate) async fn start(&self, script: String) -> Result<CommandId, ComputerError> {
        let command = Command {
            sandbox: self.sandbox.clone(),
            ..Command::new(script)
        };
        self.computers.start(self.info.id, &command).await
    }

    /// Records that the model has seen `content` as the current state of `path`.
    pub(crate) fn remember(&self, path: &str, content: &[u8]) {
        if let Ok(mut reads) = self.reads.lock() {
            reads.insert(path.to_owned(), digest(content));
        }
    }

    /// Refuses to change a file the model has not read, or that changed since it did.
    /// `current` is the file's digest now; `None` when it is too large to know.
    pub(crate) fn check_fresh(&self, path: &str, current: Option<&Sha256>) -> Result<(), String> {
        let reads = self
            .reads
            .lock()
            .map_err(|_| "The workspace's read record is unavailable.".to_owned())?;
        match reads.get(path) {
            None => Err(format!("Read {path} before changing it.")),
            Some(seen) if Some(seen) == current => Ok(()),
            Some(_) => Err(format!(
                "{path} changed since you last read it. Read it again before changing it."
            )),
        }
    }
}

/// A computer's refusal as the model reads it: the computer's own sentence where it wrote one
/// for this (paths outside the workspace, bad patterns), and what went wrong otherwise.
pub(crate) fn explain(error: &ComputerError) -> String {
    match error {
        ComputerError::NotFound(_) | ComputerError::Refused(_) | ComputerError::Failed(_) => {
            error.message().to_owned()
        }
        ComputerError::Unsupported(_) | ComputerError::Unreachable(_) => {
            format!("The computer could not do this: {error}")
        }
    }
}

/// Collects a short command's output.
#[derive(Default)]
pub(crate) struct Collected(Mutex<String>);

impl Collected {
    pub(crate) fn text(&self) -> String {
        self.0.lock().map(|text| text.clone()).unwrap_or_default()
    }
}

impl Output for Collected {
    fn write<'a>(&'a self, text: &'a str) -> glave_ports::BoxFuture<'a, ()> {
        if let Ok(mut all) = self.0.lock() {
            all.push_str(text);
        }
        Box::pin(std::future::ready(()))
    }
}

#[cfg(test)]
impl Workspace {
    /// This machine's folder `root` as a workspace, as the CLI opens it.
    pub(crate) async fn open(
        root: impl AsRef<Path>,
        environment: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
    ) -> Result<Self, ComputerError> {
        let (computers, info) = glave_computer::LocalComputers::here(root, environment).await?;
        Ok(Self::new(computers, info))
    }
}

#[cfg(test)]
mod tests;
