//! The organization a request works in, when its caller names one (`specs/session.md`, "Who
//! is calling"): someone who belongs to several names it in the `Glave-Organization` header,
//! checked against the directory before anything is read. One they are not in is refused
//! exactly as one that does not exist, so the answer says nothing of other organizations.

use glave_event::{AuthMethod, OrgRole};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider, OrgId};
use uuid::Uuid;

use crate::protocol::{ApiError, OrganizationInfo, Reply, code};
use crate::server::Server;
use crate::words::Sentence;

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Answers `organization/list`: the organizations the caller belongs to, in the order they
    /// joined, so their own, the one they signed up into, first (for the person on the server's
    /// computer, the server's own alone), each as its people ledger says it is to them.
    pub(crate) async fn list_organizations(&self, caller: &Caller) -> Result<Reply, ApiError> {
        let current = self.org_of(caller);
        let theirs = match (caller.person, self.inner.accounts.as_deref()) {
            (Some(person), Some(accounts)) if with_account(caller) => {
                accounts.organizations(person).await?
            }
            _ => vec![current],
        };
        let mut organizations = Vec::new();
        for org in theirs {
            self.catch_up_people(org).await?;
            let (people, own, role, owner) = self.people(org, |people| {
                let counted = people
                    .count()
                    .saturating_add(usize::from(org == self.inner.own));
                // The person on the server's computer, whose own the server's is, is no one
                // in its ledger.
                let (own, role) = match caller.person {
                    Some(person) => (people.founded_by(person), people.role(person)),
                    None => (org == self.inner.own, OrgRole::Owner),
                };
                let owner = people
                    .first_owner_but(caller.person)
                    .and_then(|owner| people.info(owner))
                    .and_then(|owner| owner.name.or(owner.email));
                (counted, own, role, owner)
            });
            let name = match (own, owner) {
                (true, _) => Sentence::OwnOrganization,
                (false, Some(owner)) => Sentence::OwnersTeam { owner },
                (false, None) => Sentence::SomeTeam,
            };
            organizations.push(OrganizationInfo {
                id: org,
                name: name.into(),
                people,
                own,
                role,
                current: org == current,
            });
        }
        Ok(Reply::Organizations { organizations })
    }

    /// `caller`, working in the organization `named`, when their request names one: one they
    /// belong to, else refused. The organization the transport found for them is kept when it
    /// names none, and asked for again only when it names another. Only a glave account
    /// belongs to more than the one it signs in to; the local person and anyone a company
    /// login signs in are in the server's own alone.
    pub(crate) async fn in_organization(
        &self,
        mut caller: Caller,
        named: Option<&str>,
    ) -> Result<Caller, ApiError> {
        let Some(named) = named else {
            return Ok(caller);
        };
        let named = Uuid::try_parse(named.trim())
            .map(OrgId::from_uuid)
            .map_err(|_| not_in())?;
        if named == self.org_of(&caller) {
            return Ok(caller);
        }
        if !with_account(&caller) {
            return Err(not_in());
        }
        let (Some(person), Some(accounts)) = (caller.person, self.inner.accounts.as_deref()) else {
            return Err(not_in());
        };
        let theirs = accounts
            .organizations(person)
            .await
            .map_err(|_| ApiError::new(code::INTERNAL, Sentence::RecordsFailed))?;
        if !theirs.contains(&named) {
            return Err(not_in());
        }
        caller.org = Some(named);
        Ok(caller)
    }
}

/// The one answer to an organization the caller is not in, whether or not it exists: its own
/// code, so a client stops naming it.
fn not_in() -> ApiError {
    ApiError::new(code::NOT_IN, Sentence::NotInOrganization)
}

/// Whether `caller` signed in with a glave account, the only kind of caller who may belong to
/// several organizations.
pub(crate) fn with_account(caller: &Caller) -> bool {
    caller
        .auth
        .as_ref()
        .is_some_and(|auth| auth.method == AuthMethod::Account)
}
