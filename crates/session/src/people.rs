//! The organization's people and their roles (`specs/model.md`, "Roles"), kept in its people
//! ledger: one stream of the organization's own, beside every project's. Whoever signs in
//! through the identity provider is recorded there the first time and whenever their token
//! says something new of them (`person.joined`); a role given is `role.set`, the latest
//! counting. The local person is the organization's owner, always, and never recorded. Every
//! admin method is checked here, against the caller's role, before anything is read.

use std::collections::BTreeMap;
use std::sync::{MutexGuard, PoisonError};

use glave_event::{
    At, AuthMethod, Body, EventId, Header, OrgRole, Payload, PersonJoined, Principal, PrincipalId,
    RawEvent, RoleSet, Seq, encode,
};
use glave_ports::stream::{Reserved, append_retrying, catch_up};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider, Scope};

use crate::invites::Invites;
use crate::ledger::in_order;
use crate::protocol::{ApiError, PersonInfo, Reply, code};
use crate::server::Server;
use crate::words::Sentence;

/// People listed at once unless the caller says, and at most.
const LISTED: usize = 100;
const MOST_LISTED: usize = 1_000;

/// What a role lets someone do. Admin methods check these, never a role's name, so a role
/// made later of another set of them needs no change to any check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Permit {
    /// List the organization's people and give them Admin or Member.
    People,
    /// Make someone an Owner, or change an Owner's role.
    Owners,
    /// Make invites to the organization, list them and revoke them.
    Invites,
}

/// What each of the three roles permits.
pub(crate) fn permits(role: OrgRole) -> &'static [Permit] {
    match role {
        OrgRole::Owner => &[Permit::People, Permit::Owners, Permit::Invites],
        OrgRole::Admin => &[Permit::People, Permit::Invites],
        OrgRole::Member => &[],
    }
}

/// A person as the ledger has them: their latest record, when they first signed in, and the
/// role they were given last, if any.
#[derive(Debug, Clone)]
struct Kept {
    joined: PersonJoined,
    at: At,
    role: Option<OrgRole>,
}

/// What the people ledger says so far: one entry per person, however many times they signed
/// in, so it grows with the organization's people and not with its events.
#[derive(Debug, Default)]
pub(crate) struct People {
    /// The `seq` the next event will take.
    next: u64,
    known: BTreeMap<PrincipalId, Kept>,
    /// The invites made and not yet spent or revoked, expired ones included until dropped.
    pub(crate) invites: Invites,
}

impl People {
    fn next_seq(&self) -> u64 {
        self.next.max(1)
    }

    /// Takes in the event at the next `seq`; any other is already in, or not yet.
    fn apply(&mut self, raw: RawEvent) {
        let Some((at, body)) = in_order(&mut self.next, raw) else {
            return;
        };
        match body {
            Body::PersonJoined(joined) => {
                let person = joined.person;
                if let Some(kept) = self.known.get_mut(&person) {
                    kept.joined = joined;
                } else {
                    let kept = Kept {
                        joined,
                        at,
                        role: None,
                    };
                    self.known.insert(person, kept);
                }
            }
            Body::RoleSet(set) => {
                if let Some(kept) = self.known.get_mut(&set.person) {
                    kept.role = Some(set.role);
                }
            }
            Body::InviteCreated(created) => self.invites.made(created, at),
            Body::InviteRevoked(revoked) => self.invites.spent(revoked.invite),
            Body::InviteAccepted(accepted) => {
                self.invites.spent(accepted.invite);
                if let Some(kept) = self.known.get_mut(&accepted.person) {
                    kept.role = Some(kept.role.unwrap_or_default().max(accepted.role));
                }
            }
            _ => {}
        }
    }

    /// Whether the ledger has `joined` as it is.
    fn knows(&self, joined: &PersonJoined) -> bool {
        self.known
            .get(&joined.person)
            .is_some_and(|kept| kept.joined == *joined)
    }

    /// The role of `person`, who signed in from elsewhere: a Member until given another.
    pub(crate) fn role(&self, person: PrincipalId) -> OrgRole {
        self.known
            .get(&person)
            .and_then(|kept| kept.role)
            .unwrap_or_default()
    }

    pub(crate) fn info(&self, person: PrincipalId) -> Option<PersonInfo> {
        self.known.get(&person).map(|kept| PersonInfo {
            id: person,
            name: kept.joined.profile.name.clone(),
            email: kept.joined.profile.email.clone(),
            subject: kept.joined.subject.clone(),
            groups: kept.joined.profile.groups.clone(),
            role: kept.role.unwrap_or_default(),
            joined: kept.at,
        })
    }
}

/// What the ledger keeps of `caller`, when they signed in through an identity provider.
pub(crate) fn joined_of(caller: &Caller) -> Option<PersonJoined> {
    let auth = caller.auth.as_ref()?;
    if auth.method != AuthMethod::Oidc {
        return None;
    }
    Some(PersonJoined {
        person: caller.person?,
        issuer: auth.issuer.clone()?,
        subject: auth.subject.clone()?,
        profile: caller.profile.clone().unwrap_or_default(),
    })
}

fn admins_only() -> ApiError {
    ApiError::new(code::NOT_ALLOWED, Sentence::AdminsOnly)
}

pub(crate) fn no_such_person(person: PrincipalId) -> ApiError {
    ApiError::new(
        code::NO_PERSON,
        Sentence::NoSuchPerson {
            person: person.to_string(),
        },
    )
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// The caller's role in the organization: the local person is its owner; anyone signed in
    /// from elsewhere has the role the ledger last gave them, else Member.
    pub(crate) async fn role_of(&self, caller: &Caller) -> Result<OrgRole, ApiError> {
        let Some(person) = caller.person else {
            return Ok(OrgRole::Owner);
        };
        self.catch_up_people().await?;
        Ok(self.people().role(person))
    }

    /// Refuses `caller` what their role does not permit, before anything of it is read, and
    /// answers their role when it does.
    pub(crate) async fn permitted(
        &self,
        caller: &Caller,
        needed: Permit,
    ) -> Result<OrgRole, ApiError> {
        let role = self.role_of(caller).await?;
        if permits(role).contains(&needed) {
            Ok(role)
        } else {
            Err(match needed {
                Permit::People | Permit::Invites => admins_only(),
                Permit::Owners => ApiError::new(code::NOT_ALLOWED, Sentence::OwnersOnly),
            })
        }
    }

    /// Records `caller` in the people ledger when they signed in through the identity provider
    /// and it does not have them as they are now: the first time, or when their token says
    /// something new of them.
    pub(crate) async fn seen(&self, caller: &Caller) -> Result<(), ApiError> {
        let Some(joined) = joined_of(caller) else {
            return Ok(());
        };
        if self.people().knows(&joined) {
            return Ok(());
        }
        self.record_person(|people| Ok((!people.knows(&joined)).then(|| joined.clone())))
            .await
    }

    /// Answers `people/list`: the people after `after`, in id order, at most `limit`. An
    /// `after` naming no one here is refused, as a cursor from another list.
    pub(crate) async fn list_people(
        &self,
        after: Option<PrincipalId>,
        limit: Option<usize>,
    ) -> Result<Reply, ApiError> {
        self.catch_up_people().await?;
        let limit = limit.unwrap_or(LISTED).clamp(1, MOST_LISTED);
        let held = self.people();
        if let Some(after) = after
            && !held.known.contains_key(&after)
        {
            return Err(no_such_person(after));
        }
        let people = held
            .known
            .keys()
            .filter(|person| after.is_none_or(|after| **person > after))
            .take(limit)
            .filter_map(|person| held.info(*person))
            .collect();
        Ok(Reply::People { people })
    }

    /// Answers `role/set`: gives `person` `role`, when the caller may. Only an Owner makes an
    /// Owner or changes one's role.
    pub(crate) async fn set_role(
        &self,
        caller: &Caller,
        person: PrincipalId,
        role: OrgRole,
    ) -> Result<Reply, ApiError> {
        let theirs = permits(self.permitted(caller, Permit::People).await?);
        let by = Principal::person(self.person_of(caller)?);
        self.record_person(|people| {
            if !people.known.contains_key(&person) {
                return Err(no_such_person(person));
            }
            let owners = role == OrgRole::Owner || people.role(person) == OrgRole::Owner;
            if owners && !theirs.contains(&Permit::Owners) {
                return Err(ApiError::new(code::NOT_ALLOWED, Sentence::OwnersOnly));
            }
            Ok(Some(RoleSet {
                person,
                role,
                by: by.clone(),
            }))
        })
        .await?;
        let person = self
            .people()
            .info(person)
            .ok_or_else(|| no_such_person(person))?;
        Ok(Reply::Person { person })
    }

    /// Reads whatever was added to the people ledger since this server last looked.
    pub(crate) async fn catch_up_people(&self) -> Result<(), ApiError> {
        // Glave's own: the organization's people ledger, answered only as the methods here say.
        let scope = Scope::of_organization(self.inner.org);
        let after = Seq::new(self.people().next_seq().saturating_sub(1)).ok();
        let log = self.inner.harness.log();
        catch_up(log, &scope, Reserved::People.id(), after, |page| {
            let mut held = self.people();
            for raw in page {
                held.apply(raw);
            }
        })
        .await?;
        Ok(())
    }

    /// Appends what `make` makes of the ledger as it is then, after anything others wrote
    /// meanwhile: nothing when it makes nothing.
    pub(crate) async fn record_person<T: Payload>(
        &self,
        make: impl Fn(&People) -> Result<Option<T>, ApiError>,
    ) -> Result<(), ApiError> {
        let ledger = Reserved::People.id();
        // Glave's own: the organization's people ledger.
        let scope = Scope::of_organization(self.inner.org);
        let log = self.inner.harness.log();
        let make = &make;
        let appended = append_retrying(log, &scope, ledger, || async move {
            self.catch_up_people().await?;
            let (payload, next) = {
                let held = self.people();
                (make(&held)?, held.next_seq())
            };
            let Some(payload) = payload else {
                return Ok(None);
            };
            let now = self.inner.harness.clock().now();
            let header = Header {
                id: EventId::new(now, self.inner.harness.entropy().bytes()?)?,
                thread: ledger,
                seq: Seq::new(next)?,
                at: At::from(now),
            };
            Ok::<_, ApiError>(Some((vec![encode(header, &payload)?], ())))
        })
        .await?;
        let mut held = self.people();
        for raw in appended.into_iter().flat_map(|(events, ())| events) {
            held.apply(raw);
        }
        Ok(())
    }

    pub(crate) fn people(&self) -> MutexGuard<'_, People> {
        self.inner
            .people
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}
