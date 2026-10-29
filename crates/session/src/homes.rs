//! A person's own computer woken as they come back (`specs/computers.md`, "A person's computer
//! in the cloud", 6): opening the app, or a chat, starts the wake, so it is usually awake by the
//! time a turn needs it. A turn that needs it before then waits for the same wake.

use glave_ports::{BlobStore, Caller, EventLog, ModelProvider};

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
        if let Err(error) = homes.wake(self.org_of(caller), person) {
            tracing::warn!(error = error.message(), "could not start waking a computer");
        }
    }
}
