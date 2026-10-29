//! Invites to the organization (`specs/model.md`, "Invites"): a one-time link that gives a
//! role. Kept in the organization's people ledger beside its people, each as the SHA-256 of its
//! code alone, so an invite is found only in the organization that made it, and every server on
//! one store sees the same ones. Opening one spends it, appended after whatever another server
//! wrote meanwhile, so a code works once however many servers are asked at once.

use std::collections::BTreeMap;

use glave_event::{
    At, InviteAccepted, InviteCreated, InviteId, InviteRevoked, OrgRole, Principal, PrincipalId,
};
use glave_identity::signin::{kept, one_time_code, same_kept};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider};
use jiff::{SignedDuration, Timestamp};

use crate::people::{People, Permit, joined_of, no_such_person, permits};
use crate::protocol::{ApiError, InviteInfo, Reply, code};
use crate::server::Server;
use crate::words::Sentence;

/// How long an invite works, opened or not: long enough to reach someone through a chat or an
/// email and be opened after a weekend, short enough that one forgotten in a channel stops.
const LIFETIME: SignedDuration = SignedDuration::from_hours(7 * 24);

/// Invites waiting at once, at most.
const MOST_WAITING: usize = 1_000;

/// An invite made and not spent or revoked.
#[derive(Debug, Clone)]
struct Waiting {
    created: InviteCreated,
    made: At,
}

/// The invites made and not yet spent or revoked, by id, so oldest first.
#[derive(Debug, Default)]
pub(crate) struct Invites {
    waiting: BTreeMap<InviteId, Waiting>,
}

impl Invites {
    pub(crate) fn made(&mut self, created: InviteCreated, made: At) {
        self.waiting
            .insert(created.invite, Waiting { created, made });
    }

    pub(crate) fn spent(&mut self, invite: InviteId) {
        self.waiting.remove(&invite);
    }

    /// Forgets those expired at `now`: nothing can spend them, so nothing read later needs them.
    fn drop_expired(&mut self, now: Timestamp) {
        self.waiting
            .retain(|_, waiting| now < waiting.created.expires.timestamp());
    }
}

impl Waiting {
    fn info(&self) -> InviteInfo {
        InviteInfo {
            id: self.created.invite,
            role: self.created.role,
            by: self.created.by.clone(),
            made: self.made,
            expires: self.created.expires,
        }
    }
}

impl People {
    /// Whether `by` may still give `role` by an invite: the local person, the organization's
    /// owner, always; anyone else as their role now permits.
    fn may_give(&self, by: &Principal, role: OrgRole, local: Option<PrincipalId>) -> bool {
        let theirs = match by.id {
            Some(id) if Some(id) != local => self.role(id),
            _ => OrgRole::Owner,
        };
        let permitted = permits(theirs);
        permitted.contains(&Permit::Invites)
            && (role != OrgRole::Owner || permitted.contains(&Permit::Owners))
    }
}

/// Every code that opens no invite here, whatever the reason, is refused alike.
fn gone() -> ApiError {
    ApiError::new(code::NO_INVITE, Sentence::InviteGone)
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Answers `invite/create`: an invite giving `role`, and its code, this once.
    pub(crate) async fn create_invite(
        &self,
        caller: &Caller,
        role: OrgRole,
    ) -> Result<Reply, ApiError> {
        if !self.inner.identity_provider {
            return Err(ApiError::new(code::NOT_ALLOWED, Sentence::NoOrganization));
        }
        let theirs = permits(self.permitted(caller, Permit::Invites).await?);
        if role == OrgRole::Owner && !theirs.contains(&Permit::Owners) {
            return Err(ApiError::new(code::NOT_ALLOWED, Sentence::OwnersOnly));
        }
        let code = one_time_code()?;
        let now = self.inner.harness.clock().now();
        let expires = now
            .checked_add(LIFETIME)
            .map_err(|e| ApiError::new(code::INTERNAL, e.to_string()))?;
        let created = InviteCreated {
            invite: InviteId::new(now, self.inner.harness.entropy().bytes()?)?,
            hash: kept(code.expose()),
            role,
            by: Principal::person(self.person_of(caller)?),
            expires: At::from(expires),
        };
        let org = self.org_of(caller);
        self.record_person(org, |people| {
            let waiting = people.invites.waiting.values();
            if waiting
                .filter(|it| now < it.created.expires.timestamp())
                .count()
                >= MOST_WAITING
            {
                return Err(ApiError::new(code::NOT_ALLOWED, Sentence::TooManyInvites));
            }
            Ok(Some(created.clone()))
        })
        .await?;
        let invite = self.people(org, |people| {
            people.invites.drop_expired(now);
            let waiting = &people.invites.waiting;
            waiting.get(&created.invite).map(Waiting::info)
        });
        Ok(Reply::Invited {
            invite: invite.ok_or_else(gone)?,
            code,
        })
    }

    /// Answers `invite/list`: the invites waiting in the caller's organization, oldest first.
    pub(crate) async fn list_invites(&self, caller: &Caller) -> Result<Reply, ApiError> {
        let org = self.org_of(caller);
        self.catch_up_people(org).await?;
        let now = self.inner.harness.clock().now();
        let invites = self.people(org, |people| {
            people.invites.drop_expired(now);
            people.invites.waiting.values().map(Waiting::info).collect()
        });
        Ok(Reply::Invites { invites })
    }

    /// Answers `invite/revoke`: `invite`, when it waits, can no longer be opened.
    pub(crate) async fn revoke_invite(
        &self,
        caller: &Caller,
        invite: InviteId,
    ) -> Result<Reply, ApiError> {
        let by = Principal::person(self.person_of(caller)?);
        let now = self.inner.harness.clock().now();
        self.record_person(self.org_of(caller), |people| {
            let waits = people
                .invites
                .waiting
                .get(&invite)
                .is_some_and(|it| now < it.created.expires.timestamp());
            if !waits {
                return Err(ApiError::new(
                    code::NO_INVITE,
                    Sentence::NoSuchInvite {
                        invite: invite.to_string(),
                    },
                ));
            }
            Ok(Some(InviteRevoked {
                invite,
                by: by.clone(),
            }))
        })
        .await?;
        Ok(Reply::Done {})
    }

    /// Answers `invite/accept`: spends the invite `code` opens, for the caller signed in
    /// through the identity provider, raising their role to its.
    pub(crate) async fn accept_invite(
        &self,
        caller: &Caller,
        code: &str,
    ) -> Result<Reply, ApiError> {
        let Some(joined) = joined_of(caller) else {
            return Err(ApiError::new(code::NOT_ALLOWED, Sentence::InviteSignIn));
        };
        let person = joined.person;
        let presented = kept(code);
        let now = self.inner.harness.clock().now();
        let local = self.inner.harness.person();
        let org = self.org_of(caller);
        self.record_person(org, |people| {
            let found = people.invites.waiting.values().find(|it| {
                same_kept(&it.created.hash, &presented)
                    && now < it.created.expires.timestamp()
                    && people.may_give(&it.created.by, it.created.role, local)
            });
            let Some(found) = found else {
                return Err(gone());
            };
            Ok(Some(InviteAccepted {
                invite: found.created.invite,
                person,
                role: found.created.role,
            }))
        })
        .await?;
        let person = self
            .people(org, |people| people.info(person))
            .ok_or_else(|| no_such_person(person))?;
        Ok(Reply::Person { person })
    }
}
