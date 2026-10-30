//! Invites to the organization (`specs/model.md`, "Invites"): a one-time link that gives a
//! role. Kept in the organization's people ledger beside its people, each as the SHA-256 of its
//! code alone, so an invite is found only in the organization that made it, and every server on
//! one store sees the same ones. Opening one spends it, appended after whatever another server
//! wrote meanwhile, so a code works once however many servers are asked at once.

use std::collections::BTreeMap;

use glave_event::{
    At, InviteAccepted, InviteCreated, InviteId, InviteRevoked, OrgRole, PersonJoined, Principal,
    PrincipalId, Profile, Secret,
};
use glave_identity::signin::{kept, one_time_code, same_kept};
use glave_ports::{BlobStore, Caller, EventLog, Mail, ModelProvider, OrgId};
use jiff::{SignedDuration, Timestamp};
use uuid::Uuid;

use crate::mailing::just_now;
use crate::organizations::with_account;
use crate::people::{People, Permit, joined_of, no_such_person, permits};
use crate::protocol::{ApiError, InviteInfo, JoinWay, PersonInfo, Reply, ShownInvite, code};
use crate::server::Server;
use crate::words::{Language, Sentence, say};

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

    /// Whether one still works at `now`, as `invite/list` would list it.
    pub(crate) fn wait(&self, now: Timestamp) -> bool {
        self.waiting.values().any(|waiting| waiting.works(now))
    }

    /// Forgets those expired at `now`: nothing can spend them, so nothing read later needs them.
    fn drop_expired(&mut self, now: Timestamp) {
        self.waiting.retain(|_, waiting| waiting.works(now));
    }
}

impl Waiting {
    /// Whether it still works at `now`: it stops at its expiry, opened or not.
    fn works(&self, now: Timestamp) -> bool {
        now < self.created.expires.timestamp()
    }

    fn info(&self) -> InviteInfo {
        InviteInfo {
            id: self.created.invite,
            role: self.created.role,
            by: self.created.by.clone(),
            made: self.made,
            expires: self.created.expires,
            email: self.created.email.clone(),
        }
    }
}

impl People {
    /// The invite whose code's hash is `presented` and that opens at `now`: live, and giving
    /// no more than its maker may give now.
    pub(crate) fn opens(
        &self,
        presented: &str,
        now: Timestamp,
        local: Option<PrincipalId>,
    ) -> Option<&InviteCreated> {
        self.invites
            .waiting
            .values()
            .find(|it| {
                same_kept(&it.created.hash, presented)
                    && it.works(now)
                    && self.may_give(&it.created.by, it.created.role, local)
            })
            .map(|it| &it.created)
    }

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
pub(crate) fn gone() -> ApiError {
    ApiError::new(code::NO_INVITE, Sentence::InviteGone)
}

/// What parts an invite's code: the id of the organization it is to, then its secret.
const TO: char = '.';

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
        email: Option<&str>,
    ) -> Result<Reply, ApiError> {
        // Someone else signs in through the company login, or with a glave account here.
        self.joins_by()?;
        let theirs = permits(self.permitted(caller, Permit::Invites).await?);
        if role == OrgRole::Owner && !theirs.contains(&Permit::Owners) {
            return Err(ApiError::new(code::NOT_ALLOWED, Sentence::OwnersOnly));
        }
        let sending = match email {
            Some(to) => Some(self.sender(to)?),
            None => None,
        };
        if sending.is_some() {
            self.count_asked(caller).await?;
        }
        let org = self.org_of(caller);
        let code = Secret::new(format!(
            "{}{TO}{}",
            org.as_uuid().simple(),
            one_time_code()?.expose()
        ));
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
            email: sending.as_ref().map(|(_, _, to)| to.clone()),
        };
        self.record_person(org, |people| {
            let waiting = people.invites.waiting.values();
            if waiting.clone().filter(|it| it.works(now)).count() >= MOST_WAITING {
                return Err(ApiError::new(code::NOT_ALLOWED, Sentence::TooManyInvites));
            }
            // One email a minute to an address, from any server on the store.
            let mailed = waiting.into_iter().any(|it| {
                it.created.email.is_some()
                    && it.created.email == created.email
                    && just_now(it.made.timestamp(), now)
            });
            if mailed {
                return Err(ApiError::new(code::NOT_AVAILABLE, Sentence::MailedJustNow));
            }
            Ok(Some(created.clone()))
        })
        .await?;
        let invite = self.people(org, |people| {
            people.invites.drop_expired(now);
            let waiting = &people.invites.waiting;
            waiting.get(&created.invite).map(Waiting::info)
        });
        if let Some((mailer, origin, to)) = sending {
            let link = format!("{origin}/#invite={}", code.expose());
            let mail = Mail {
                to,
                subject: say(Language::English, &Sentence::InviteMailSubject),
                text: format!(
                    "{}\n\n{link}\n\n{}\n",
                    say(Language::English, &Sentence::InviteMail),
                    say(Language::English, &Sentence::InviteMailIgnore),
                ),
            };
            if self.send(&*mailer, &mail).await.is_err() {
                // An invite whose link reached no one is not left waiting.
                self.revoke_invite(caller, created.invite).await?;
                return Err(ApiError::new(code::NOT_AVAILABLE, Sentence::InviteNotSent));
            }
        }
        Ok(Reply::Invited {
            invite: invite.ok_or_else(gone)?,
            code,
        })
    }

    /// The organization an invite's `code` is to: the id it begins with, before [`TO`]; a code
    /// that names none is to the server's own, and opens nothing elsewhere.
    pub(crate) fn invited_to(&self, code: &str) -> OrgId {
        code.split_once(TO)
            .and_then(|(org, _)| Uuid::try_parse(org).ok())
            .map_or(self.inner.own, OrgId::from_uuid)
    }

    /// How someone not signed in joins this server's organizations by an invite: through its
    /// company login, or by making a passkey where it keeps glave accounts; refused where no
    /// one else can.
    fn joins_by(&self) -> Result<JoinWay, ApiError> {
        if self.inner.identity_provider {
            Ok(JoinWay::Company)
        } else if self.inner.accounts.is_some() {
            Ok(JoinWay::Passkey)
        } else {
            Err(ApiError::new(code::NOT_ALLOWED, Sentence::NoOrganization))
        }
    }

    /// Whether `caller` may make invites here: the server takes them, they signed in, and
    /// their role permits inviting.
    pub(crate) async fn may_invite(&self, caller: &Caller) -> Result<bool, ApiError> {
        if self.joins_by().is_err() || caller.auth.is_none() {
            return Ok(false);
        }
        Ok(permits(self.role_of(caller).await?).contains(&Permit::Invites))
    }

    /// Answers `invite/show`: what the invite `code` opens now, for whoever holds it, and how
    /// they join with it: signed out, by a passkey or the company login; signed in, by
    /// accepting it, unless they are in its organization already. Refused alike as any code
    /// that opens nothing. The organization it names is read apart, never kept.
    pub(crate) async fn show_invite(&self, caller: &Caller, code: &str) -> Result<Reply, ApiError> {
        let way = self.joins_by()?;
        let org = self.invited_to(code);
        let presented = kept(code);
        let now = self.inner.harness.clock().now();
        let local = self.inner.harness.person();
        let people = self.people_apart(org).await?;
        let opened = people.opens(&presented, now, local).ok_or_else(gone)?;
        let by = opened
            .by
            .id
            .filter(|maker| Some(*maker) != local)
            .and_then(|maker| people.info(maker))
            .and_then(|maker| maker.name.or(maker.email));
        let joining = match (caller.auth.is_some(), caller.person) {
            (false, _) => way,
            // The person on the server's computer is in its own organization alone.
            (true, None) if org == self.inner.own => JoinWay::Member,
            (true, Some(person)) if people.has(person) && !people.removed(person) => {
                JoinWay::Member
            }
            (true, _) => JoinWay::Accept,
        };
        Ok(Reply::InviteShown {
            shown: ShownInvite {
                organization: org,
                role: opened.role,
                by,
                joining,
            },
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
                .is_some_and(|it| it.works(now));
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

    /// Answers `invite/accept`: spends the invite `code` opens, raising the caller's role to
    /// its: someone signed in through the identity provider, in their organization; someone
    /// with a glave account, in the organization it is to, which they join.
    pub(crate) async fn accept_invite(
        &self,
        caller: &Caller,
        code: &str,
    ) -> Result<Reply, ApiError> {
        let org = self.invited_to(code);
        let joined = match (joined_of(caller), caller.person) {
            (Some(joined), _) if org == self.org_of(caller) => joined,
            // Another organization's code opens nothing for someone a company login signed in.
            (Some(_), _) => return Err(gone()),
            (None, Some(person)) if with_account(caller) && self.inner.accounts.is_some() => {
                PersonJoined {
                    person,
                    issuer: None,
                    subject: None,
                    profile: Profile::default(),
                }
            }
            (None, _) => return Err(ApiError::new(code::NOT_ALLOWED, Sentence::InviteSignIn)),
        };
        let person = joined.person;
        let presented = kept(code);
        let now = self.inner.harness.clock().now();
        let local = self.inner.harness.person();
        self.record_people(org, |people, next| {
            let opened = people.opens(&presented, now, local).ok_or_else(gone)?;
            let accepted = InviteAccepted {
                invite: opened.invite,
                person,
                role: opened.role,
            };
            // Someone removed, or not yet recorded, joins with it; a glave account with the
            // address the invite was emailed to, as one an invite makes does.
            if people.has(person) {
                return Ok(Some(vec![self.people_event(next, &accepted)?]));
            }
            let mut joined = joined.clone();
            if joined.issuer.is_none() {
                joined.profile.email.clone_from(&opened.email);
            }
            let after = next.saturating_add(1);
            Ok(Some(vec![
                self.people_event(next, &joined)?,
                self.people_event(after, &accepted)?,
            ]))
        })
        .await?;
        if let (true, Some(accounts)) = (with_account(caller), self.inner.accounts.as_deref()) {
            accounts.join(person, org).await?;
        }
        let person = self
            .people(org, |people| people.info(person))
            .map(|info| PersonInfo { you: true, ..info })
            .ok_or_else(|| no_such_person(person))?;
        Ok(Reply::Person { person })
    }
}
