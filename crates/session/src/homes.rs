//! A person's own computer kept awake while a turn or task works on it (`specs/computers.md`,
//! "A person's computer in the cloud", 4). Nothing here wakes it: the first tool call that
//! needs it does (decision 6), so opening the app, a chat, or a turn that only talks never
//! does.

use glave_event::PrincipalId;
use glave_ports::{BlobStore, EventLog, ModelProvider, OrgId};

use crate::server::Server;

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Keeps the computer of `person` in `org` from sleeping while what it answers is kept,
    /// where people have computers of their own: for a turn or task of a thread for them, whose
    /// work runs there (the harness's `home_of` takes the same person).
    pub(crate) fn hold_home(
        &self,
        org: OrgId,
        person: Option<PrincipalId>,
    ) -> Option<Box<dyn Send>> {
        let (homes, person) = (self.inner.homes.as_ref()?, person?);
        homes
            .hold(org, person)
            .map_err(|error| {
                tracing::warn!(error = error.message(), "could not keep a computer awake");
            })
            .ok()
    }
}
