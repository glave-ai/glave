//! Taking someone out of an organization (`specs/model.md`, "Identity"): one admin action that
//! records `person.removed`, takes their glave account's membership out of the directory, and
//! tells every watch here a sign-in may have ended, so theirs in the organization end at once.
//! Their next request there is refused, and a company login lets them back only by an invite.

use glave_event::{OrgRole, PersonRemoved, Principal, PrincipalId};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider};

use crate::people::{Permit, no_such_person, permits};
use crate::protocol::{ApiError, Reply, code};
use crate::server::Server;
use crate::words::Sentence;

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Answers `person/remove`: `person` out of the caller's organization, when the caller may.
    /// Only an Owner removes an Owner, and never the last of an organization whose owner is no
    /// one on the server's computer. Asked again for someone removed, it finishes what a
    /// failure left undone.
    pub(crate) async fn remove_person(
        &self,
        caller: &Caller,
        person: PrincipalId,
    ) -> Result<Reply, ApiError> {
        let theirs = permits(self.permitted(caller, Permit::People).await?);
        let by = Principal::person(self.person_of(caller)?);
        let org = self.org_of(caller);
        let alone = org != self.inner.own;
        self.record_person(org, |people| {
            // Removed already: nothing to record, and what follows is done again.
            if people.removed(person) {
                return Ok(None);
            }
            if !people.has(person) {
                return Err(no_such_person(person));
            }
            if people.role(person) == OrgRole::Owner && !theirs.contains(&Permit::Owners) {
                return Err(ApiError::new(code::NOT_ALLOWED, Sentence::OwnersOnly));
            }
            if people.last_owner(person, alone) {
                return Err(ApiError::new(code::NOT_ALLOWED, Sentence::LastOwner));
            }
            Ok(Some(PersonRemoved {
                person,
                by: by.clone(),
            }))
        })
        .await?;
        // Their watches here end now, before anything else can fail.
        self.inner.hub.sign_ins_changed();
        if let Some(accounts) = self.inner.accounts.as_deref() {
            accounts.leave(person, org).await?;
        }
        Ok(Reply::Done {})
    }
}
