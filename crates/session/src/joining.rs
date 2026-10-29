//! Invited people making a glave account (`specs/model.md`, "Identity", "Invites"): where the
//! organization has no company login and the server is reached at a public origin, an invite's
//! link makes a passkey. Opening it begins making the first passkey of a new account; making
//! it keeps the account, finds the person by their passkeys from then on, and spends the
//! invite, recording them as joined with its role in one append, so a code someone else spent
//! meanwhile joins no one.

use std::sync::{MutexGuard, PoisonError};

use glave_event::{InviteAccepted, PersonJoined, PrincipalId, Profile, Secret};
use glave_identity::signin::{Accounts, Proof, kept};
use glave_ports::{BlobStore, EventLog, ModelProvider, OrgId};
use jiff::{SignedDuration, Timestamp};

use crate::invites::gone;
use crate::protocol::{ApiError, Reply, code};
use crate::server::Server;
use crate::words::Sentence;

/// How long making the first passkey may take, as any passkey request.
const LIFETIME: SignedDuration = SignedDuration::from_mins(5);

/// Accounts being made at once; a new one past this forgets the oldest.
const MOST: usize = 16;

/// The first passkey of a new account, being made.
pub(crate) struct Joining {
    /// The passkey request's id.
    ceremony: String,
    person: PrincipalId,
    org: OrgId,
    /// The invite code's hash.
    code: String,
    began: Timestamp,
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
        let org = self.inner.own;
        let presented = kept(code.expose());
        let now = self.inner.harness.clock().now();
        self.catch_up_people(org).await?;
        let local = self.inner.harness.person();
        if self
            .people(org, |people| people.opens(&presented, now, local))
            .is_none()
        {
            return Err(gone());
        }
        let (person, browsers) = accounts.made().await?;
        let ceremony = browsers.create_options(Proof::Vouched)?;
        let mut joining = self.joinings();
        joining
            .0
            .retain(|it| now.duration_since(it.began) <= LIFETIME);
        if joining.0.len() >= MOST {
            joining.0.remove(0);
        }
        joining.0.push(Joining {
            ceremony: ceremony.id.clone(),
            person,
            org,
            code: presented,
            began: now,
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
        let (_, session) = browsers
            .create_vouched(&joining.ceremony, credential, name)
            .await?;
        accounts.found_by_its_passkeys(joining.person).await?;
        self.spend_joining(&joining).await?;
        accounts.join(joining.person, joining.org).await?;
        Ok(self.signed_in(session, &browsers))
    }

    /// Records the person of `joining` as joined, with the role of the invite its code opens
    /// now, spending it; refused when it opens none any more.
    async fn spend_joining(&self, joining: &Joining) -> Result<(), ApiError> {
        let now = self.inner.harness.clock().now();
        let local = self.inner.harness.person();
        self.record_people(joining.org, |people, next| {
            let (invite, role) = people.opens(&joining.code, now, local).ok_or_else(gone)?;
            let joined = PersonJoined {
                person: joining.person,
                issuer: None,
                subject: None,
                profile: Profile::default(),
            };
            let accepted = InviteAccepted {
                invite,
                person: joining.person,
                role,
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
