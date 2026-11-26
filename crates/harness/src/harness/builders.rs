//! The builder methods a harness is made with: each `with_*` gives its threads one more thing.

use std::sync::{Arc, PoisonError};

use glave_ports::memories::Memory;
use glave_ports::settings::Hooks;
use glave_ports::{BlobStore, EventLog, ModelProvider, ReadMade, ToolSource};

use super::Harness;
use crate::approval::Approver;
use crate::homes::{CredentialsOf, Homes};
use crate::own_tools::OwnTools;
use crate::project_folders::ProjectFolders;
use crate::skills::Skills;

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Opens each thread that works in a folder of its own with what `projects` reads for that
    /// folder, rather than the settings' instructions, and gives it what its project turns on
    /// at the start of each turn.
    #[must_use]
    pub fn with_project_folders(
        mut self,
        project_folders: Option<Arc<dyn ProjectFolders>>,
    ) -> Self {
        self.project_folders = project_folders;
        self
    }

    /// Gives threads `memory` when there is one: loaded at their start, its tools offered, the
    /// person's controls served (`specs/memory.md`). None leaves them without.
    #[must_use]
    pub fn with_memory(mut self, memory: Option<Arc<dyn Memory>>) -> Self {
        self.memory = memory;
        self
    }

    /// Works each person's threads on their own computer from `homes`, when there is one.
    #[must_use]
    pub fn with_homes(mut self, homes: Option<Arc<dyn Homes>>) -> Self {
        let sandbox = homes.as_ref().and_then(|homes| homes.sandbox());
        self.change_defaults(|defaults| defaults.home_sandbox = sandbox);
        self.homes = homes;
        self
    }

    /// Reads each file a turn made with `reader`, apart from glave, for its title, its parts
    /// and its preview.
    #[must_use]
    pub fn with_reader(mut self, reader: Option<ReadMade>) -> Self {
        self.reader = reader;
        self
    }

    /// Reads the files of one look-over for at most `within`, in all, in place of 30 seconds:
    /// past it, the rest are kept unread.
    #[must_use]
    pub fn reading_within(mut self, within: std::time::Duration) -> Self {
        self.reading_for = within;
        self
    }

    /// Lets the commands on each person's own computer ask `credentials_of` for that person's
    /// credentials, at each use.
    #[must_use]
    pub fn with_credentials_of(mut self, credentials_of: Option<CredentialsOf>) -> Self {
        self.credentials_of = credentials_of;
        self
    }

    /// Offers the tools of `sources` too, after the built-in ones. A tool keeps the first
    /// source that names it.
    #[must_use]
    pub fn with_tools(mut self, sources: Vec<Arc<dyn ToolSource>>) -> Self {
        self.sources.extend(sources);
        self
    }

    /// Offers `skills` to its threads, those on this computer's and glave's own; a thread on a
    /// person's own computer is offered what is found there, with glave's own.
    #[must_use]
    pub fn with_skills(mut self, skills: Option<Skills>) -> Self {
        self.skills = skills.map(Arc::new);
        self
    }

    /// Offers the tools of `own`, the person's own, to the threads of the person the harness
    /// works for alone, each in the places they use it, after its other sources.
    #[must_use]
    pub fn with_own_tools(self, own: OwnTools) -> Self {
        self.set_own_tools(own);
        self
    }

    /// Offers the tools of `own` as the person's own in place of those before, from each
    /// thread's next turn on: a turn already running keeps the tools it started with.
    pub fn set_own_tools(&self, own: OwnTools) {
        *self.own.write().unwrap_or_else(PoisonError::into_inner) = own;
    }

    /// Runs `hooks` at their points of the turns of the person the harness works for.
    #[must_use]
    pub fn with_hooks(self, hooks: Hooks) -> Self {
        self.change_defaults(|defaults| defaults.hooks = hooks);
        self
    }
}
