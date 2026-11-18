//! Whom glave acts as when it acts for someone on its own, as a schedule's run does: never in
//! the form of the person on its computer for anyone else, and only for someone their
//! organization's people ledger lists as there now.

use glave_event::{Auth, AuthMethod, Principal, PrincipalId, PrincipalKind};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider, OrgId};

use crate::protocol::{ApiError, code};
use crate::server::Server;
use crate::words::Sentence;

/// Where someone glave acts for stands in their organization, as its people ledger says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Membership {
    Member,
    Removed,
    /// Not in the ledger as read: never taken for a member.
    Unknown,
}

/// Who glave acts as for `person` on its own, in `org`, as a schedule's run does: the person on
/// this computer as they call, for none; anyone else unmistakably as themselves, signed in by no
/// one (`on_behalf`), so nothing that only the person on this computer may do is theirs.
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

/// The person `by` names, as a schedule and its runs record them: always named, so a server
/// whose own person is someone else acts for the one who made it. Refused for anything but a
/// person with an id, none included, whom glave could act for only as someone else.
pub(crate) fn whose(by: Option<&Principal>) -> Result<Option<PrincipalId>, ApiError> {
    match by {
        Some(Principal {
            kind: PrincipalKind::Person,
            id: Some(person),
        }) => Ok(Some(*person)),
        _ => Err(ApiError::new(code::NOT_ALLOWED, Sentence::MakerUnknown)),
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

    /// Where `caller`, someone glave acts for on its own, stands in their organization, as its
    /// people ledger says now, read whole and apart from what this server keeps of people
    /// ledgers, which may forget one midway through a read: the server's own person always a
    /// member. Refused while it can't be read; one it doesn't list is unknown, never taken for
    /// still there.
    pub(crate) async fn membership(&self, caller: &Caller) -> Result<Membership, ApiError> {
        let (Some(person), Some(org)) = (caller.person, caller.org) else {
            return Ok(Membership::Member);
        };
        let people = self.people_apart(org).await?;
        Ok(if people.removed(person) {
            Membership::Removed
        } else if people.has(person) {
            Membership::Member
        } else {
            Membership::Unknown
        })
    }
}
