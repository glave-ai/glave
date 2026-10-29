//! Invited people making a glave account (`specs/model.md`, "Identity", "Invites"): where the
//! organization has no company login and the server is reached at a public origin, an invite's
//! link makes a passkey. Opening it begins making the first passkey of a new account; making
//! it checks the passkey, spends the invite, recording the person as joined with its role in
//! one append, and only then keeps the account and its organization, so a code someone else
//! spent meanwhile makes no account.

use std::sync::{MutexGuard, PoisonError};

use glave_event::{InviteAccepted, PersonJoined, PrincipalId, Profile, Secret};
use glave_identity::signin::{Accounts, Proof, kept};
use glave_ports::{BlobStore, EventLog, ModelProvider, OrgId};

use crate::invites::gone;
use crate::protocol::{ApiError, Reply, code};
use crate::server::Server;
use crate::words::Sentence;

/// Accounts being made at once; a new one past this forgets the oldest. Each passkey request
/// expires on its own, as any does.
const MOST: usize = 16;

/// The first passkey of a new account, being made.
pub(crate) struct Joining {
    /// The passkey request's id.
    ceremony: String,
    person: PrincipalId,
    org: OrgId,
    /// The invite code's hash.
    code: String,
}

/// The accounts being made on this server.
#[derive(Default)]
pub(crate) struct Joinings(Vec<Joining>);

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// The glave accounts of this server, where it keeps them.
    pub(crate) fn accounts(&self) -> Result<&Accounts, ApiError> {
        self.inner
            .accounts
            .as_deref()
            .ok_or_else(|| ApiError::new(code::NOT_ALLOWED, Sentence::NoAccounts))
    }

    /// Answers `passkey/create_options` with an invite's `code`: begins making the first passkey
    /// of a new account, when the code opens an invite now.
    pub(crate) async fn join_options(&self, code: &Secret) -> Result<Reply, ApiError> {
        let accounts = self.accounts()?;
        let org = self.invited_to(code.expose());
        let presented = kept(code.expose());
        let now = self.inner.harness.clock().now();
        self.catch_up_people(org).await?;
        let local = self.inner.harness.person();
        if self.people(org, |people| people.opens(&presented, now, local).is_none()) {
            return Err(gone());
        }
        let (person, browsers) = accounts.made().await?;
        let ceremony = browsers.create_options(Proof::SignedIn)?;
        let mut joining = self.joinings();
        if joining.0.len() >= MOST {
            joining.0.remove(0);
        }
        joining.0.push(Joining {
            ceremony: ceremony.id.clone(),
            person,
            org,
            code: presented,
        });
        Ok(crate::browsers::options(ceremony))
    }

    /// The account being made with the passkey request `ceremony`, taken, if one is.
    pub(crate) fn joining(&self, ceremony: &str) -> Option<Joining> {
        let mut joining = self.joinings();
        let at = joining.0.iter().position(|it| it.ceremony == ceremony)?;
        Some(joining.0.remove(at))
    }

    /// Answers `passkey/create` for an account being made: its first passkey, the browser
    /// signed in with it, and the person joined by the invite.
    pub(crate) async fn join(
        &self,
        joining: Joining,
        credential: &serde_json::Value,
        name: Option<&str>,
    ) -> Result<Reply, ApiError> {
        let accounts = self.accounts()?;
        let browsers = accounts.browsers(joining.person).await?;
        // Checked, then the invite spent, and only then is anything of the account kept: a
        // code spent meanwhile leaves nothing behind.
        let checked = browsers.checked(&joining.ceremony, credential, name)?;
        self.spend_joining(&joining).await?;
        let session = browsers.keep(checked).await?;
        accounts.join(joining.person, joining.org).await?;
        Ok(self.signed_in(session, &browsers))
    }

    /// Records the person of `joining` as joined, with the role of the invite its code opens
    /// now, spending it; refused when it opens none any more.
    async fn spend_joining(&self, joining: &Joining) -> Result<(), ApiError> {
        let now = self.inner.harness.clock().now();
        let local = self.inner.harness.person();
        self.record_people(joining.org, |people, next| {
            let opened = people.opens(&joining.code, now, local).ok_or_else(gone)?;
            // The address the invite's link was emailed to, which they opened: theirs.
            let joined = PersonJoined {
                person: joining.person,
                issuer: None,
                subject: None,
                profile: Profile {
                    email: opened.email.clone(),
                    ..Profile::default()
                },
            };
            let accepted = InviteAccepted {
                invite: opened.invite,
                person: joining.person,
                role: opened.role,
            };
            let after = next.saturating_add(1);
            Ok(Some(vec![
                self.people_event(next, &joined)?,
                self.people_event(after, &accepted)?,
            ]))
        })
        .await
    }

    fn joinings(&self) -> MutexGuard<'_, Joinings> {
        self.inner
            .joinings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}
