//! What a thread runs with where neither it nor its project chooses (`specs/one-server.md`,
//! "Its settings"): the person's own choices, as the host read them last. The host replaces
//! them when the person edits their settings, and each turn takes them as it starts, so a
//! running turn keeps what it started with and the next one runs with the new.

use std::sync::{Arc, PoisonError};

use glave_event::ApprovalMode;
use glave_ports::computer::Sandbox;
use glave_ports::settings::{Hooks, Judge};
use glave_ports::{BlobStore, EventLog, ModelProvider, ToolSource};

use crate::approval::Approver;
use crate::harness::Harness;

/// The person's choices every thread runs with unless it, or its project, chooses otherwise.
#[derive(Clone)]
pub struct Defaults {
    /// How much a thread's turns ask before acting, when it chose no mode.
    pub approval: ApprovalMode,
    /// Who judges flagged actions in auto mode: its model must be one the provider serves.
    pub judge: Judge,
    /// Whether a model titles each thread after its first exchange.
    pub titles: bool,
    /// The sandbox commands on this computer run in; none runs them unconfined.
    pub sandbox: Option<Sandbox>,
    /// Commands run at points of the turns of the person the harness works for.
    pub hooks: Hooks,
    /// `web_fetch`, offered right after the built-in tools unless a thread's project sets up
    /// its own; none when it is off.
    pub web: Option<Arc<dyn ToolSource>>,
}

impl std::fmt::Debug for Defaults {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Defaults")
            .field("approval", &self.approval)
            .field("judge", &self.judge)
            .field("titles", &self.titles)
            .field("sandbox", &self.sandbox)
            .field("hooks", &self.hooks)
            .field("web", &self.web.is_some())
            .finish()
    }
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// What threads run with now where neither they nor their projects choose.
    pub fn defaults(&self) -> Arc<Defaults> {
        Arc::clone(&self.defaults.read().unwrap_or_else(PoisonError::into_inner))
    }

    /// Runs every thread's next turn with `defaults`: a turn already running keeps those it
    /// started with.
    pub fn set_defaults(&self, defaults: Defaults) {
        *self
            .defaults
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Arc::new(defaults);
    }

    /// The defaults as `change` changes them, from each thread's next turn on.
    pub(crate) fn change_defaults(&self, change: impl FnOnce(&mut Defaults)) {
        let mut defaults = Defaults::clone(&self.defaults());
        change(&mut defaults);
        self.set_defaults(defaults);
    }
}
