//! Separate copies of a project's folder (`specs/session.md`, "Where a thread works"): whether
//! a new thread works on one, a git worktree of its own, unless `thread/start` says, and whether
//! a new chat may be offered one, which `project/get` says.

use std::path::Path;
use std::sync::atomic::Ordering;

use glave_ports::{BlobStore, Caller, EventLog, ModelProvider, Scope};

use crate::protocol::Copies;
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

    /// What `project/get` tells `caller` of a new chat's copies in `scope`: one may be offered
    /// where its folder is in a git repository with a commit to start one from, on this server's
    /// computer, and its threads work here rather than on workers; and whether it works on one
    /// unless told. A folder that isn't there anymore has none.
    pub(crate) async fn copies(&self, caller: &Caller, scope: Scope) -> Option<Copies> {
        let on_workers = self
            .inner
            .worker
            .as_ref()
            .is_some_and(|(chosen, _)| *chosen == scope.project());
        if on_workers {
            return None;
        }
        let found = self.folder_to_start_in(scope.located()).await.ok()??;
        let (folder, false) = &found else {
            return None;
        };
        let workspace = self.inner.harness.workspace();
        if !workspace.copies(folder).await.unwrap_or(false) {
            return None;
        }
        let by_default = self.copies_by_default(caller, Some(&found)).await;
        Some(Copies { by_default })
    }
}
