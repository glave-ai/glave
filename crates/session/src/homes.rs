//! A person's own computer woken as they come back (`specs/computers.md`, "A person's computer
//! in the cloud", 6): opening the app, or a chat, starts the wake, so it is usually awake by the
//! time a turn needs it. A turn that needs it before then waits for the same wake, and every
//! turn or task keeps it from sleeping while it runs.

use glave_event::PrincipalId;
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider, OrgId};

use crate::protocol::Request;
use crate::server::Server;

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Starts waking `caller`'s computer when `request` is what a client sends as the person
    /// opens the app (their threads listed) or a chat (watched), where people have computers of
    /// their own. Never waits, and never fails the request: a wake that fails is tried again
    /// by the turn that needs it, which says why.
    pub(crate) fn wake_for(&self, caller: &Caller, request: &Request) {
        let opens = matches!(
            request,
            Request::ThreadList { .. }
                | Request::ThreadListWatch { .. }
                | Request::ThreadWatch { .. }
        );
        let (Some(homes), Some(person), true) =
            (&self.inner.homes, caller.person.or(self.person()), opens)
        else {
            return;
        };
        if let Err(error) = homes.wake(self.inner.org, person) {
            tracing::warn!(error = error.message(), "could not start waking a computer");
        }
    }

    /// Keeps the computer of `person` (the server's own when none) in `org` from sleeping while
    /// what it answers is kept, where people have computers of their own: for a turn or task.
    pub(crate) fn hold_home(
        &self,
        org: OrgId,
        person: Option<PrincipalId>,
    ) -> Option<Box<dyn Send>> {
        let homes = self.inner.homes.as_ref()?;
        let person = person.or(self.person())?;
        homes
            .hold(org, person)
            .map_err(|error| {
                tracing::warn!(error = error.message(), "could not keep a computer awake");
            })
            .ok()
    }
}
