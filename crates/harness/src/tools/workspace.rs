//! The workspace the tools act on: a binding to one workspace on a computer, reached through
//! the computer port, so the same tools work on this machine or across the wire. The
//! computer confines every path and runs the commands; the binding adds what the harness
//! decides: the sandbox each command runs in, the paths the file tools may not change in it,
//! and what the model has read, so writes and edits never act on a stale view.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use glave_event::Sha256;
use glave_ports::{
    Bytes, Cancellation, Command, ComputerError, Computers, FileWrite, Finished, Glob, Globbed,
    Listed, Listing, Output, Sandbox, Search, Searched, Sources, Stat, WorkspaceInfo, digest,
    is_protected, is_protected_refusal, refuse_protected,
};

use crate::cancel::Cancel;

/// One workspace on one computer, the sandbox its commands run in, and what the model has
/// seen there.
pub struct Workspace {
    computers: Arc<dyn Computers>,
    info: WorkspaceInfo,
    root: PathBuf,
    sandbox: Option<Sandbox>,
    temp: Option<PathBuf>,
    /// What the model last read of each file, by its path in the workspace.
    reads: Mutex<HashMap<String, Sha256>>,
}

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
            reads: Mutex::new(HashMap::new()),
        }
    }

    /// Runs every command in `sandbox`. Fails if the computer cannot enforce it, so a
    /// requested sandbox is never quietly skipped; on success, learns the temp folder its
    /// commands get, which the shell assessment treats as theirs to write.
    pub async fn with_sandbox(mut self, sandbox: &Sandbox) -> Result<Self, ComputerError> {
        let mut probe = Command::new(r#"printf '%s' "$TMPDIR""#);
        probe.sandbox = Some(sandbox.clone());
        let temp = Collected::default();
        let finished = self
            .computers
            .run(self.info.id, &probe, &temp, &Cancel::new())
            .await?;
        let temp = temp.text();
        if finished.ending != (glave_ports::Ending::Exited { code: 0 }) || temp.is_empty() {
            return Err(ComputerError::Failed(format!(
                "a command in the sandbox did not run: {temp}"
            )));
        }
        self.temp = Some(PathBuf::from(temp));
        self.sandbox = Some(sandbox.clone());
        Ok(self)
    }

    /// A worktree of the repository around this workspace, on `branch`, on the same computer
    /// and in the same sandbox: the one already there for that branch, or a new one made from
    /// the current commit. `owner` is kept beside it, for whoever cleans it up later.
    pub async fn worktree(&self, branch: &str, owner: Option<&str>) -> Result<Self, ComputerError> {
        let sources = Sources::Worktree {
            path: self.info.root.clone(),
            branch: branch.to_owned(),
            owner: owner.map(str::to_owned),
        };
        let info = self.computers.open(self.info.computer, &sources).await?;
        let worktree = Self::new(Arc::clone(&self.computers), info);
        match &self.sandbox {
            Some(sandbox) => worktree.with_sandbox(sandbox).await,
            None => Ok(worktree),
        }
    }

    /// A scratch folder of its own named `name`, on the same computer and in the same sandbox:
    /// the one already there, or a new, empty one. `owner` is kept beside it, as for a worktree.
    pub async fn scratch(&self, name: &str, owner: Option<&str>) -> Result<Self, ComputerError> {
        let sources = Sources::Scratch {
            name: name.to_owned(),
            owner: owner.map(str::to_owned),
        };
        let info = self.computers.open(self.info.computer, &sources).await?;
        let scratch = Self::new(Arc::clone(&self.computers), info);
        match &self.sandbox {
            Some(sandbox) => scratch.with_sandbox(sandbox).await,
            None => Ok(scratch),
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
            .run(script.to_owned(), 120, &output, &Cancel::new())
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

    /// Runs `command` in this workspace, in its sandbox if it has one.
    pub(crate) async fn run(
        &self,
        script: String,
        timeout_secs: u64,
        output: &dyn Output,
        cancel: &dyn Cancellation,
    ) -> Result<Finished, ComputerError> {
        let command = Command {
            script,
            timeout_secs: Some(timeout_secs),
            sandbox: self.sandbox.clone(),
        };
        let span = crate::telemetry::command();
        let finished = tracing::Instrument::instrument(
            self.computers.run(self.info.id, &command, output, cancel),
            span.clone(),
        )
        .await;
        let ended = finished.as_ref().ok();
        crate::telemetry::command_ended(&span, ended.map(|done| (&done.ending, done.millis)));
        finished
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
        ComputerError::NotFound(message)
        | ComputerError::Refused(message)
        | ComputerError::Failed(message) => message.clone(),
        ComputerError::Unsupported(_) | ComputerError::Unreachable(_) => {
            format!("The computer could not do this: {error}")
        }
    }
}

/// Collects a short command's output.
#[derive(Default)]
struct Collected(Mutex<String>);

impl Collected {
    fn text(&self) -> String {
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
