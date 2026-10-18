//! The host's own settings, read again (`specs/one-server.md`, "Its settings"): a server that
//! runs for days notices when the person edits them, at the start of each turn and when the
//! models are listed, and runs what they choose from then on. The host decides what changed;
//! the server takes the models and the person's own tools it answers with.

use std::sync::Arc;

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
        };
        rereads.reread(&apply).await;
    }
}
