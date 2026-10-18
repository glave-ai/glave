//! The host's own settings, read again (`specs/one-server.md`, "Its settings"): a server that
//! runs for days notices when the person edits them, at the start of each turn and thread, and
//! when the models are listed or the server discovered, and runs what they choose from then on.
//! The host decides what changed; the server takes the models, how threads run and the
//! person's own tools it answers with.

use std::sync::atomic::Ordering;
use std::sync::{Arc, PoisonError};

use glave_harness::{ApprovalMode, Defaults};
use glave_ports::{BlobStore, BoxFuture, EventLog, ModelProvider, ToolSource};

use crate::providers::Models;
use crate::server::Server;

/// What changed in the host's settings since it last read them.
#[derive(Default)]
pub struct Changed {
    /// The models the server runs from now on.
    pub models: Option<Models>,
    /// The person's own tools from now on, such as their MCP servers'.
    pub own: Option<Vec<Arc<dyn ToolSource>>>,
    /// How threads run from now on where neither they nor their projects choose.
    pub runs: Option<Runs>,
}

/// How threads run where neither they nor their projects choose: from each one's next turn,
/// or, for where a thread works, from the next new one.
pub struct Runs {
    /// What every thread's turns run with: the approval mode, the judge, the sandbox and the
    /// rest. The judge comes with the models it names, so it never names one that is gone.
    pub defaults: Defaults,
    /// The approval modes the policy leaves a thread to choose.
    pub modes: Vec<ApprovalMode>,
    /// Whether a new thread in a project with a folder works in a worktree of its own.
    pub worktree: bool,
}

/// Runs what changed, as the server takes it: synchronously, so the host knows once it
/// returns that nothing new goes to what it replaced.
pub type Apply<'a> = dyn Fn(Changed) + Send + Sync + 'a;

/// What reads the host's settings again: the person's own files, by their content.
pub trait Rereads: Send + Sync {
    /// Reads the settings again when they changed since the last read, hands `apply` what
    /// changed, then lets go of what that replaced. Nothing changes when they did not, or
    /// when they no longer read: the last that did stay, and the host says why in its log.
    fn reread<'a>(&'a self, apply: &'a Apply<'a>) -> BoxFuture<'a, ()>;
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Reads the host's settings again, if it has any to read, and runs what changed.
    pub(crate) async fn reread(&self) {
        let Some(rereads) = self.inner.rereads.as_deref() else {
            return;
        };
        // One change at a time, as when model accounts change, so the models offered are
        // those of the last.
        let _changing = self.inner.changing_models.lock().await;
        let apply = |changed: Changed| {
            if let Some(models) = changed.models {
                self.offer(models);
            }
            if let Some(own) = changed.own {
                self.inner.harness.set_own_tools(own);
            }
            if let Some(runs) = changed.runs {
                let modes = crate::modes::choosable(runs.defaults.approval, &runs.modes);
                *self
                    .inner
                    .modes
                    .write()
                    .unwrap_or_else(PoisonError::into_inner) = modes;
                self.inner.worktree.store(runs.worktree, Ordering::Relaxed);
                self.inner.harness.set_defaults(runs.defaults);
            }
        };
        rereads.reread(&apply).await;
    }
}
