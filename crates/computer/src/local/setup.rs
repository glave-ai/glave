//! How this machine's computers are set up, before they are shared: where they keep
//! worktrees, scratch folders and new projects, the id the host goes by, glave's own program,
//! what their commands may not read and find first on `PATH`, and where they are recorded.

use std::path::PathBuf;
use std::sync::Arc;

#[cfg(doc)]
use glave_ports::computer::Sources;
use glave_ports::computer::{ComputerId, DiskSpace};
use glave_process::Processes;

use super::{LocalComputers, Measure};
use crate::disk::Largest;

impl LocalComputers {
    /// Keeps threads' worktrees under `folder`, one folder per repository, so a computer here
    /// can open [`Sources::Worktree`]. They outlive the computer: removing them is the
    /// person's choice.
    #[must_use]
    pub fn with_worktrees(mut self, folder: impl Into<PathBuf>) -> Self {
        self.worktrees = Some(folder.into());
        self
    }

    /// Says of what uses its disk what `largest` holds, which its [`crate::Keeper`] fills.
    #[must_use]
    pub fn with_largest(mut self, largest: Largest) -> Self {
        self.largest = largest;
        self
    }

    /// Says its disk holds what `measure` answers, for a test that can't fill one.
    #[must_use]
    pub fn with_disk_measure(
        mut self,
        measure: impl Fn() -> DiskSpace + Send + Sync + 'static,
    ) -> Self {
        self.measure = Some(Measure(Arc::new(measure)));
        self
    }

    /// Makes threads' scratch folders under `folder`, so a computer here can open
    /// [`Sources::Scratch`]. Like worktrees, they outlive the computer.
    #[must_use]
    pub fn with_scratch(mut self, folder: impl Into<PathBuf>) -> Self {
        self.scratch = Some(folder.into());
        self
    }

    /// Makes new projects' folders under `folder`, so a computer here can open
    /// [`Sources::Project`]. They are the person's: nothing glave does removes them.
    #[must_use]
    pub fn with_projects(mut self, folder: impl Into<PathBuf>) -> Self {
        self.projects = Some(folder.into());
        self
    }

    /// Opens this machine as the host computer under `id`, one kept across runs, so whatever
    /// names a computer (a thread's binding, a project's checkout) names this one the same way
    /// each time.
    #[must_use]
    pub fn known_as(mut self, id: ComputerId) -> Self {
        self.host = Some(id);
        self
    }

    /// Names `program` as glave's own: a program (usually this one, from
    /// `std::env::current_exe`) that calls [`crate::sandbox_helper`] first thing in `main`.
    /// Linux needs it to confine `read-only` commands with Landlock where bubblewrap cannot
    /// start; without it, such a machine refuses sandboxed commands.
    #[must_use]
    pub fn with_helper(mut self, program: impl Into<PathBuf>) -> Self {
        self.helper = Some(program.into());
        self
    }

    /// Records the commands its workspaces start in `processes`' ledger while they run, so a
    /// glave that starts after this one crashed stops them (`glave_process`). Without it they
    /// are still stopped with their workspace, only not recorded.
    #[must_use]
    pub fn with_processes(mut self, processes: Processes) -> Self {
        self.processes = processes;
        self
    }

    /// Hides `places` from its sandboxed commands too, as the home folder's credential stores
    /// are, whether they exist yet or not (on Linux, from the next command once they do): the
    /// evals hide this machine's own crate downloads, so an attempt starts as a fresh computer
    /// would.
    #[must_use]
    pub fn with_hidden(mut self, places: impl IntoIterator<Item = PathBuf>) -> Self {
        self.hidden = places.into_iter().collect();
        self
    }

    /// Puts `folder`, which holds `glave` (see [`crate::glave_on_path`]), first on every
    /// command's `PATH`, and keeps it readable in every sandbox though it sits in glave's own
    /// folder, which they hide: a command's `glave` is then the one running it.
    #[must_use]
    pub fn with_glave_on_path(mut self, folder: impl Into<PathBuf>) -> Self {
        self.glave_on_path = Some(folder.into());
        self
    }

    /// Whether a command still runs here in the background: for a computer that ends when
    /// idle ([`crate::Idle`]).
    pub fn busy(&self) -> bool {
        self.background.running()
    }
}
