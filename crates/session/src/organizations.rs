//! The organization a request works in, when its caller names one (`specs/session.md`, "Who
//! is calling"): someone who belongs to several names it in the `Glave-Organization` header,
//! checked against the directory before anything is read. One they are not in is refused
//! exactly as one that does not exist, so the answer says nothing of other organizations.

use glave_event::AuthMethod;
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider, OrgId};
use uuid::Uuid;

use crate::protocol::{ApiError, code};
use crate::server::Server;
use crate::words::Sentence;

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
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
        let account = caller
            .auth
            .as_ref()
            .is_some_and(|auth| auth.method == AuthMethod::Account);
        if !account {
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

/// The one answer to an organization the caller is not in, whether or not it exists.
fn not_in() -> ApiError {
    ApiError::new(code::NOT_ALLOWED, Sentence::NotInOrganization)
}
