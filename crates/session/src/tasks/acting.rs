//! Whom glave acts as when it acts for someone on its own, as a schedule's run does: never in
//! the form of the person on its computer for anyone else, and only for someone their
//! organization's people ledger lists as there now.

use glave_event::{Auth, AuthMethod, Principal, PrincipalId, PrincipalKind};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider, OrgId};

use crate::people::People;
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

/// An organization's people ledger, read whole once, by the first of a tick's runs there that
/// needs it.
pub(crate) type PeopleOnce = tokio::sync::OnceCell<People>;

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// The person `by` names, as a schedule and its runs record them: named from
    /// `schedule.created` v4 on; none, which older ones left implied, is the server's own person
    /// where it has one. Refused for anything but a person with an id, whom glave could act for
    /// only as someone else.
    pub(crate) fn maker_of(&self, by: Option<&Principal>) -> Result<PrincipalId, ApiError> {
        let unknown = || ApiError::new(code::NOT_ALLOWED, Sentence::MakerUnknown);
        match by {
            None => self.inner.person.ok_or_else(unknown),
            Some(Principal {
                kind: PrincipalKind::Person,
                id: Some(person),
            }) => Ok(*person),
            Some(_) => Err(unknown()),
        }
    }

    /// Who glave acts as for `person` on its own, in `org`: [`acting_for`] anyone but the
    /// server's own person, who stays the person on this computer.
    pub(crate) fn acting_as(&self, person: Option<PrincipalId>, org: OrgId) -> Caller {
        acting_for(self.someone_else(person).and_then(|by| by.id), org)
    }

    /// Where `person`, someone glave acts for on its own, stands in `org`, as its people ledger
    /// says now, read whole and apart from what this server keeps of people ledgers, which may
    /// forget one midway through a read, once for all who need it (`people`): the server's own
    /// person always a member of the server's own organization, and of any other only as its
    /// ledger says. Refused while it can't be read; one it doesn't list is unknown, never taken
    /// for still there.
    pub(crate) async fn membership(
        &self,
        person: PrincipalId,
        org: OrgId,
        people: &PeopleOnce,
    ) -> Result<Membership, ApiError> {
        if org == self.inner.own && Some(person) == self.inner.person {
            return Ok(Membership::Member);
        }
        let people = people.get_or_try_init(|| self.people_apart(org)).await?;
        Ok(if people.removed(person) {
            Membership::Removed
        } else if people.has(person) {
            Membership::Member
        } else {
            Membership::Unknown
        })
    }
}
