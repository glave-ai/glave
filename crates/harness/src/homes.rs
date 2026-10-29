//! People's own computers (`specs/computers.md`, "A person's computer in the cloud"): where a
//! deployment gives each person a computer of their own, a thread for them works there, in
//! their home, rather than on the harness's computer. Their folders, worktrees and scratch
//! folders are opened on it as they would be here.

use std::sync::Arc;

use glave_event::{ComputerKind, PrincipalId};
use glave_ports::computer::{ComputerError, Home};
use glave_ports::{BlobStore, BoxFuture, EventLog, ModelProvider, OrgId, Scope};

use crate::approval::Approver;
use crate::harness::{Harness, HarnessError};
use crate::tools::workspace::Workspace;

/// Each person's own computer, woken on demand. The harness cannot reach a cluster itself (its
/// crate depends on no adapter), so the host hands it this.
pub trait Homes: Send + Sync {
    /// Starts waking the computer of `person` in `org`, unless it is awake or waking, and
    /// returns at once: for when they open the app, so it is awake by the time a turn needs it.
    fn wake(&self, org: OrgId, person: PrincipalId) -> Result<(), ComputerError>;

    /// The computer of `person` in `org`, once it is awake, and the workspace of their home on
    /// it.
    fn home(&self, org: OrgId, person: PrincipalId) -> BoxFuture<'_, Result<Home, ComputerError>>;
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Where the threads of `person` in `scope` open their folders, worktrees and scratch
    /// folders, and the kind of computer that is: their own computer's home, woken if it
    /// sleeps, where people have computers of their own; else the harness's own workspace.
    pub(crate) async fn home_of(
        &self,
        scope: Scope,
        person: Option<PrincipalId>,
    ) -> Result<(Arc<Workspace>, ComputerKind), HarnessError> {
        let (Some(homes), Some(person)) = (&self.homes, person) else {
            return Ok((Arc::clone(&self.workspace), ComputerKind::Local));
        };
        let awake = homes.home(scope.org(), person).await?;
        let home = Workspace::new(awake.computers, awake.home);
        // Their git sign-ins reach their computer as they reach this one: asked at each use.
        let home = match self.workspace.credentials() {
            Some(credentials) => home.with_credentials(Arc::clone(credentials)),
            None => home,
        };
        Ok((Arc::new(home), ComputerKind::Person))
    }
}
