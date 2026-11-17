//! Whom glave acts as when it acts for someone on its own: never in the form of the person on
//! its computer for anyone else.

use glave_event::{Auth, AuthMethod, PrincipalId};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider, OrgId};

use crate::server::Server;

/// Who glave acts as for `person` on its own, in `org`: the person on this computer as they
/// call, for none; anyone else unmistakably as themselves, signed in by no one (`on_behalf`), so
/// nothing that only the person on this computer may do is theirs.
pub(crate) fn acting_for(person: Option<PrincipalId>, org: OrgId) -> Caller {
    let auth = person.map(|_| Auth {
        method: AuthMethod::OnBehalf,
        issuer: None,
        subject: None,
        session: None,
    });
    Caller {
        person,
        auth,
        org: Some(org),
        ..Caller::default()
    }
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Who glave acts as for `person` on its own, in `org`: [`acting_for`] anyone but the
    /// server's own person, who stays the person on this computer.
    pub(crate) fn acting_as(&self, person: Option<PrincipalId>, org: OrgId) -> Caller {
        acting_for(self.someone_else(person).and_then(|by| by.id), org)
    }
}
