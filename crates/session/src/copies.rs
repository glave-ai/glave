//! Separate copies of a project's folder (`specs/session.md`, "Where a thread works"): whether
//! a new thread works on one, a git worktree of its own, unless `thread/start` says, and whether
//! a new chat may be offered one, which `project/get` says.

use std::path::Path;
use std::sync::atomic::Ordering;

use glave_ports::{BlobStore, Caller, EventLog, ModelProvider, Scope};

use crate::protocol::{Copies, ProjectFolder};
use crate::server::Server;

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Whether a new thread of `caller`'s in the folder `found` (and whether that is on its
    /// person's own computer) works on a separate copy unless told: the project's own choice for
    /// the person here, nearer than the server's; one on a person's own computer is not read on
    /// the server's disk.
    pub(crate) async fn copies_by_default(
        &self,
        caller: &Caller,
        found: Option<&(String, bool)>,
    ) -> bool {
        let own = self.inner.worktree.load(Ordering::Relaxed);
        match found {
            Some((folder, false)) if self.is_theirs(caller) => self
                .inner
                .harness
                .settled(Path::new(folder))
                .await
                .worktree
                .unwrap_or(own),
            _ => own,
        }
    }

    /// What `project/get` tells `caller` of a new chat's copies, whose `folder` it read: one may
    /// be offered where that folder is on this server's computer, in a git repository with a
    /// commit to start one from; and whether it works on one unless told, read afresh as a new
    /// thread reads it.
    /// None for a folder on the person's own computer, which this never wakes, nor for one
    /// that isn't there anymore.
    pub(crate) async fn copies(
        &self,
        caller: &Caller,
        folder: Option<&ProjectFolder>,
    ) -> Option<Copies> {
        let checkout = &folder?.checkout;
        if checkout.computer != self.computer() {
            return None;
        }
        let workspace = self.inner.harness.workspace();
        if !workspace.copies(&checkout.path).await.unwrap_or(false) {
            return None;
        }
        // What the person chose meanwhile applies, as it does to the thread they start.
        self.reread().await;
        let found = (checkout.path.clone(), false);
        let by_default = self.copies_by_default(caller, Some(&found)).await;
        Some(Copies { by_default })
    }
}
