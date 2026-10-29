//! The organization's people and their roles (`specs/model.md`, "Roles"), kept in its people
//! ledger: one stream of the organization's own, beside every project's. Whoever signs in
//! through the identity provider is recorded there the first time and whenever their token
//! says something new of them (`person.joined`); a role given is `role.set`, the latest
//! counting. The local person is the organization's owner, always, and never recorded. Every
//! admin method is checked here, against the caller's role, before anything is read.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::PoisonError;

use glave_event::{
    At, AuthMethod, Body, EventId, Header, OrgRole, Payload, PersonJoined, Principal, PrincipalId,
    RawEvent, RoleSet, Seq, encode,
};
use glave_ports::stream::{Reserved, append_retrying, catch_up};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider, OrgId, Scope};

use crate::invites::Invites;
use crate::ledger::in_order;
use crate::protocol::{ApiError, PersonInfo, Reply, Request, code};
use crate::server::Server;
use crate::words::Sentence;

/// Organizations whose people ledgers a server keeps in memory at once.
const PEOPLES: usize = 256;
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
    /// Those removed and not back by an invite since.
    removed: BTreeSet<PrincipalId>,
    /// The invites made and not yet spent or revoked, expired ones included until dropped.
    pub(crate) invites: Invites,
}

/// What an admin method that changes someone knows of its caller: what their role permits,
/// who they are, their organization, and whether no local person owns it.
pub(crate) struct Acting {
    pub(crate) theirs: &'static [Permit],
    pub(crate) by: Principal,
    pub(crate) org: OrgId,
    pub(crate) alone: bool,
}

/// What each organization's people ledger says, as far as this server has read it. Past
/// [`PEOPLES`] organizations, the others are forgotten and read again when next asked for.
#[derive(Debug, Default)]
pub(crate) struct Peoples(HashMap<OrgId, People>);

impl Peoples {
    /// Whether `person` was removed from `org`, as far as this server has read.
    pub(crate) fn removed(&self, org: OrgId, person: PrincipalId) -> bool {
        self.0
            .get(&org)
            .is_some_and(|people| people.removed(person))
    }

    fn of(&mut self, org: OrgId) -> &mut People {
        if self.0.len() >= PEOPLES && !self.0.contains_key(&org) {
            self.0.clear();
        }
        self.0.entry(org).or_default()
    }
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
                self.removed.remove(&person);
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
            Body::PersonRemoved(gone) => {
                self.known.remove(&gone.person);
                self.removed.insert(gone.person);
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

    /// How many people the ledger has.
    pub(crate) fn count(&self) -> usize {
        self.known.len()
    }

    /// Whether `person` was removed, and has not come back by an invite since.
    pub(crate) fn removed(&self, person: PrincipalId) -> bool {
        self.removed.contains(&person)
    }

    /// Whether `person` is in the ledger now.
    pub(crate) fn has(&self, person: PrincipalId) -> bool {
        self.known.contains_key(&person)
    }

    /// Whether `person` is the organization's last Owner, where it has no owner but those the
    /// ledger names (`alone`: no local person owns it).
    pub(crate) fn last_owner(&self, person: PrincipalId, alone: bool) -> bool {
        let owners = self
            .known
            .values()
            .filter(|kept| kept.role == Some(OrgRole::Owner))
            .count();
        alone && self.role(person) == OrgRole::Owner && owners <= 1
    }

    /// Whether anyone in it joined with a glave account.
    pub(crate) fn has_accounts(&self) -> bool {
        self.known.values().any(|kept| kept.joined.issuer.is_none())
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
            joined: Some(kept.at),
            local: false,
            you: false,
            roles: Vec::new(),
            removable: false,
        })
    }

    /// `info` as the admin `acting` sees it: whether it is them, and what they may change of
    /// it, as `role/set` and `person/remove` would let them.
    fn as_seen_by(&self, mut info: PersonInfo, acting: &Acting) -> PersonInfo {
        info.you = Some(info.id) == acting.by.id;
        let changeable = !info.you
            && !info.local
            && (info.role != OrgRole::Owner || acting.theirs.contains(&Permit::Owners))
            && !self.last_owner(info.id, acting.alone);
        info.roles = match (changeable, acting.theirs.contains(&Permit::Owners)) {
            (false, _) => Vec::new(),
            (true, true) => vec![OrgRole::Owner, OrgRole::Admin, OrgRole::Member],
            (true, false) => vec![OrgRole::Admin, OrgRole::Member],
        };
        info.removable = changeable;
        info
    }
}

/// The person on the server's computer, `local`, as the organization's people are listed: its
/// owner always, never recorded.
fn local_info(local: PrincipalId) -> PersonInfo {
    PersonInfo {
        id: local,
        name: None,
        email: None,
        subject: None,
        groups: Vec::new(),
        role: OrgRole::Owner,
        joined: None,
        local: true,
        you: false,
        roles: Vec::new(),
        removable: false,
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
        issuer: Some(auth.issuer.clone()?),
        subject: Some(auth.subject.clone()?),
        profile: caller.profile.clone().unwrap_or_default(),
    })
}

fn admins_only() -> ApiError {
    ApiError::new(code::NOT_ALLOWED, Sentence::AdminsOnly)
}

/// Refuses an admin's change of themselves: another admin changes their role, or takes them
/// out.
pub(crate) fn refuse_self(by: &Principal, person: PrincipalId) -> Result<(), ApiError> {
    if by.id == Some(person) {
        return Err(ApiError::new(code::NOT_ALLOWED, Sentence::NotYourself));
    }
    Ok(())
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
        let org = self.org_of(caller);
        self.catch_up_people(org).await?;
        Ok(self.people(org, |people| people.role(person)))
    }

    /// Whether `caller`'s organization is shown (`specs/model.md`, "Identity"): where people
    /// sign in through a company login, or, to someone signed in, where it holds more than one
    /// person, the local person counting in the server's own.
    pub(crate) async fn organization_shown(&self, caller: &Caller) -> Result<bool, ApiError> {
        if self.inner.identity_provider {
            return Ok(true);
        }
        if caller.auth.is_none() {
            return Ok(false);
        }
        let org = self.org_of(caller);
        self.catch_up_people(org).await?;
        let local = usize::from(org == self.inner.own);
        Ok(self
            .people(org, |people| people.count())
            .saturating_add(local)
            > 1)
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

    /// Refuses `caller` everything in their organization once they were removed from it, but
    /// opening an invite, which brings them back. Else records them in the people ledger when
    /// they signed in through the company login and it does not have them as they are now:
    /// the first time, or when their token says something new of them.
    pub(crate) async fn seen(&self, caller: &Caller, request: &Request) -> Result<(), ApiError> {
        let Some(person) = caller.person else {
            return Ok(());
        };
        let org = self.org_of(caller);
        // Removed on another server on this store since this one last looked.
        self.catch_up_people(org).await?;
        if self.people(org, |people| people.removed(person)) {
            return match request {
                Request::InviteShow { .. } | Request::InviteAccept { .. } => Ok(()),
                _ => Err(ApiError::new(code::NOT_ALLOWED, Sentence::Removed)),
            };
        }
        let Some(joined) = joined_of(caller) else {
            return Ok(());
        };
        if self.people(org, |people| people.knows(&joined)) {
            return Ok(());
        }
        self.record_person(org, |people| {
            // Removed meanwhile, as this request was answered: still out.
            if people.removed(person) {
                return Err(ApiError::new(code::NOT_ALLOWED, Sentence::Removed));
            }
            Ok((!people.knows(&joined)).then(|| joined.clone()))
        })
        .await
    }

    /// Answers `people/list`: the people after `after`, in id order, at most `limit`. An
    /// `after` naming no one here is refused, as a cursor from another list.
    pub(crate) async fn list_people(
        &self,
        caller: &Caller,
        after: Option<PrincipalId>,
        limit: Option<usize>,
    ) -> Result<Reply, ApiError> {
        let org = self.org_of(caller);
        let acting = self.acting(caller).await?;
        // The local person's role is never read from the ledger, so it is read here.
        self.catch_up_people(org).await?;
        let limit = limit.unwrap_or(LISTED).clamp(1, MOST_LISTED);
        // The person on the server's computer owns its own organization, and is listed there.
        let local = self
            .inner
            .harness
            .person()
            .filter(|_| org == self.inner.own);
        let people = self.people(org, |held| {
            if let Some(after) = after
                && !held.has(after)
                && Some(after) != local
            {
                return Err(no_such_person(after));
            }
            let mut ids: BTreeSet<PrincipalId> = held.known.keys().copied().collect();
            ids.extend(local);
            Ok(ids
                .into_iter()
                .filter(|id| after.is_none_or(|after| *id > after))
                .take(limit)
                .filter_map(|id| match local {
                    Some(local) if local == id => Some(local_info(local)),
                    _ => held.info(id),
                })
                .map(|info| held.as_seen_by(info, &acting))
                .collect())
        })?;
        Ok(Reply::People { people })
    }

    /// What an admin method that changes someone knows of `caller` before it acts, once their
    /// role permits it.
    pub(crate) async fn acting(&self, caller: &Caller) -> Result<Acting, ApiError> {
        let theirs = permits(self.permitted(caller, Permit::People).await?);
        let org = self.org_of(caller);
        Ok(Acting {
            theirs,
            by: Principal::person(self.person_of(caller)?),
            org,
            alone: org != self.inner.own,
        })
    }

    /// Answers `role/set`: gives `person` `role`, when the caller may. Only an Owner makes an
    /// Owner or changes one's role.
    pub(crate) async fn set_role(
        &self,
        caller: &Caller,
        person: PrincipalId,
        role: OrgRole,
    ) -> Result<Reply, ApiError> {
        let acting = self.acting(caller).await?;
        let Acting {
            theirs,
            ref by,
            org,
            alone,
        } = acting;
        refuse_self(by, person)?;
        self.refuse_local(org, person)?;
        self.record_person(org, |people| {
            if !people.has(person) {
                return Err(no_such_person(person));
            }
            let owners = role == OrgRole::Owner || people.role(person) == OrgRole::Owner;
            if owners && !theirs.contains(&Permit::Owners) {
                return Err(ApiError::new(code::NOT_ALLOWED, Sentence::OwnersOnly));
            }
            if role != OrgRole::Owner && people.last_owner(person, alone) {
                return Err(ApiError::new(code::NOT_ALLOWED, Sentence::LastOwner));
            }
            Ok(Some(RoleSet {
                person,
                role,
                by: by.clone(),
            }))
        })
        .await?;
        let person = self
            .people(org, |people| {
                people
                    .info(person)
                    .map(|info| people.as_seen_by(info, &acting))
            })
            .ok_or_else(|| no_such_person(person))?;
        Ok(Reply::Person { person })
    }

    /// Refuses to change `person` when they are the person on the server's computer, who
    /// always owns its own organization.
    pub(crate) fn refuse_local(&self, org: OrgId, person: PrincipalId) -> Result<(), ApiError> {
        if org == self.inner.own && self.inner.harness.person() == Some(person) {
            return Err(ApiError::new(code::NOT_ALLOWED, Sentence::LocalOwner));
        }
        Ok(())
    }

    /// Reads whatever was added to `org`'s people ledger since this server last looked.
    pub(crate) async fn catch_up_people(&self, org: OrgId) -> Result<(), ApiError> {
        // Glave's own: the organization's people ledger, answered only as the methods here say.
        let scope = Scope::of_organization(org);
        let next = self.people(org, |people| people.next_seq());
        let after = Seq::new(next.saturating_sub(1)).ok();
        let log = self.inner.harness.log();
        catch_up(log, &scope, Reserved::People.id(), after, |page| {
            self.people(org, |held| {
                for raw in page {
                    held.apply(raw);
                }
            });
        })
        .await?;
        Ok(())
    }

    /// What `org`'s people ledger says, read apart from what this server keeps: for what a
    /// code someone holds names, which may be any organization, so made-up ones crowd out
    /// none this server keeps.
    pub(crate) async fn people_apart(&self, org: OrgId) -> Result<People, ApiError> {
        // Glave's own: the organization's people ledger, answered only as the methods here say.
        let scope = Scope::of_organization(org);
        let log = self.inner.harness.log();
        let mut read = People::default();
        catch_up(log, &scope, Reserved::People.id(), None, |page| {
            for raw in page {
                read.apply(raw);
            }
        })
        .await?;
        Ok(read)
    }

    /// Appends what `make` makes of the ledger as it is then, after anything others wrote
    /// meanwhile: nothing when it makes nothing.
    pub(crate) async fn record_person<T: Payload>(
        &self,
        org: OrgId,
        make: impl Fn(&People) -> Result<Option<T>, ApiError>,
    ) -> Result<(), ApiError> {
        self.record_people(org, |people, next| {
            let payload = make(people)?;
            payload
                .map(|payload| Ok(vec![self.people_event(next, &payload)?]))
                .transpose()
        })
        .await
    }

    /// Appends the events `make` makes of the ledger as it is then, the first at the `seq` it
    /// is handed and each after it at the next, after anything others wrote meanwhile: nothing
    /// when it makes nothing. They are appended together, or none of them.
    pub(crate) async fn record_people(
        &self,
        org: OrgId,
        make: impl Fn(&People, u64) -> Result<Option<Vec<RawEvent>>, ApiError>,
    ) -> Result<(), ApiError> {
        let ledger = Reserved::People.id();
        // Glave's own: the organization's people ledger.
        let scope = Scope::of_organization(org);
        let log = self.inner.harness.log();
        let make = &make;
        let appended = append_retrying(log, &scope, ledger, || async move {
            self.catch_up_people(org).await?;
            let events = self.people(org, |held| make(held, held.next_seq()))?;
            Ok::<_, ApiError>(events.map(|events| (events, ())))
        })
        .await?;
        self.people(org, |held| {
            for raw in appended.into_iter().flat_map(|(events, ())| events) {
                held.apply(raw);
            }
        });
        Ok(())
    }

    /// `payload` as the people ledger's event at `seq`, now.
    pub(crate) fn people_event<T: Payload>(
        &self,
        seq: u64,
        payload: &T,
    ) -> Result<RawEvent, ApiError> {
        let now = self.inner.harness.clock().now();
        let header = Header {
            id: EventId::new(now, self.inner.harness.entropy().bytes()?)?,
            thread: Reserved::People.id(),
            seq: Seq::new(seq)?,
            at: At::from(now),
        };
        Ok(encode(header, payload)?)
    }

    /// `look` at what this server has read of `org`'s people ledger.
    pub(crate) fn people<R>(&self, org: OrgId, look: impl FnOnce(&mut People) -> R) -> R {
        let mut held = self
            .inner
            .people
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        look(held.of(org))
    }
}
