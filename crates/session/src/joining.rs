//! Invited people making a glave account (`specs/model.md`, "Identity", "Invites"): where the
//! organization has no company login and the server takes invites by passkey (`joins_by`), an
//! invite's link makes one. Opening it begins making the first passkey of a new account; making
//! it checks the passkey, spends the invite, recording the person as joined with its role in
//! one append, and only then keeps the account and its organization, so a code someone else
//! spent meanwhile makes no account. The server keeps nothing meanwhile: the passkey request
//! the browser holds carries what it is for, sealed with it (`Making`).

use glave_event::{InviteAccepted, PersonJoined, PrincipalId, Profile, Secret};
use glave_identity::signin::{Accounts, Proof, kept};
use glave_ports::{BlobStore, EventLog, ModelProvider, OrgId};
use serde::{Deserialize, Serialize};

use crate::invites::gone;
use crate::protocol::{ApiError, Reply, code};
use crate::server::Server;
use crate::signups::Signing;
use crate::words::Sentence;

/// What a request to make a new account's passkey, or one to get back in, is for, carried
/// sealed in the request to its answer.
#[derive(Serialize, Deserialize)]
#[serde(tag = "by", rename_all = "snake_case")]
pub(crate) enum Making {
    /// An invite's link.
    Invite(Joining),
    /// An emailed link, or one whoever runs the server handed over.
    Link(Signing),
}

impl Making {
    /// As the request carries it.
    pub(crate) fn carried(&self) -> Result<serde_json::Value, ApiError> {
        serde_json::to_value(self).map_err(|e| ApiError::new(code::INTERNAL, e.to_string()))
    }
}

/// The first passkey of a new account, being made.
#[derive(Serialize, Deserialize)]
pub(crate) struct Joining {
    person: PrincipalId,
    org: OrgId,
    /// The invite code's hash.
    code: String,
    /// The address the invite's code went to alone, never back to whoever made it: whoever
    /// holds the code holds its mailbox.
    mailbox: Option<String>,
}

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
        self.joins_by()?;
        let accounts = self.accounts()?;
        let org = self.invited_to(code.expose());
        let presented = kept(code.expose());
        let now = self.inner.harness.clock().now();
        let local = self.inner.harness.person();
        // Read apart: a code names any organization, real or not, until it opens an invite.
        let opened = self
            .people_apart(org)
            .await?
            .opens(&presented, now, local)
            .cloned()
            .ok_or_else(gone)?;
        let mailbox = opened.given.mailbox().map(str::to_owned);
        let (person, browsers) = accounts.made().await?;
        let making = Making::Invite(Joining {
            person,
            org,
            code: presented,
            mailbox,
        });
        let ceremony =
            browsers.create_options_carrying(Proof::SignedIn, Some(making.carried()?))?;
        Ok(crate::browsers::options(ceremony))
    }

    /// What the passkey request `ceremony` is for, when it is one to make a new account's
    /// passkey, or one to get back in, begun here and not expired.
    pub(crate) fn making(&self, ceremony: &str) -> Option<Making> {
        let carried = self.inner.accounts.as_deref()?.carried(ceremony)?;
        serde_json::from_value(carried).ok()
    }

    /// Answers `passkey/create` for an account being made with the request `ceremony`: its
    /// first passkey, the browser signed in with it, and the person joined by the invite.
    pub(crate) async fn join(
        &self,
        (ceremony, joining): (&str, Joining),
        credential: &serde_json::Value,
        name: Option<&str>,
        person_name: Option<String>,
    ) -> Result<Reply, ApiError> {
        let accounts = self.accounts()?;
        let browsers = accounts.browsers(joining.person).await?;
        // Checked, then the invite spent, and only then is anything of the account kept: a
        // code spent meanwhile leaves nothing behind, and the request is spent with it.
        let checked = browsers.checked(ceremony, credential, name)?;
        self.spend_joining(&joining, person_name).await?;
        // The address the invite's code went to alone, which they opened, is theirs for getting
        // back in, unless another account has it. One whose code its maker was shown too proves
        // nothing of the mailbox: it stays what the organization knows them by.
        if let Some(email) = &joining.mailbox {
            accounts.claim_email(email, joining.person).await?;
        }
        let session = browsers.keep(checked).await?;
        accounts.join(joining.person, joining.org).await?;
        Ok(self.signed_in(session, &browsers))
    }

    /// Records the person of `joining` as joined, with the role of the invite its code opens
    /// now, spending it; refused when it opens none any more.
    async fn spend_joining(&self, joining: &Joining, name: Option<String>) -> Result<(), ApiError> {
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
                    name: name.clone(),
                    email: opened.given.email().map(str::to_owned),
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
}
