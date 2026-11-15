//! Making organizations (`specs/model.md`, "Identity"): the personal space sign-up makes for a
//! new account, and the organizations anyone signed in with a glave account creates
//! (`organization/create`), to invite people to, however they came: by signing up, a sign-up
//! link or an invite (owner, 2026-11-14). Both are made the same
//! way: what glave made it as (`organization.made`), its first person and their Owner role given
//! by glave, in one append to its people ledger; then, for one created, its name as its brand
//! says it; and last the person's membership, so a failure partway leaves nothing anyone sees.

use glave_event::{
    Accent, BrandSet, OrgRole, OrganizationKind, OrganizationMade, Payload, PersonJoined,
    Principal, Profile, RoleSet,
};
use glave_ports::stream::Reserved;
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider, OrgId, Scope};
use uuid::Builder;

use crate::brand::brand_name;
use crate::protocol::{ApiError, OrganizationInfo, Reply, Told, code};
use crate::server::Server;
use crate::words::Sentence;

/// The most organizations one person may create: enough for the teams anyone works in, and a
/// bound on what one account can make.
const MOST_CREATED: usize = 10;

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// A new organization's id, time-ordered.
    pub(crate) fn new_org_id(&self) -> Result<OrgId, ApiError> {
        let now = self.inner.harness.clock().now();
        let millis = u64::try_from(now.as_millisecond()).unwrap_or_default();
        let random = self.inner.harness.entropy().bytes()?;
        Ok(OrgId::from_uuid(
            Builder::from_unix_timestamp_millis(millis, &random).into_uuid(),
        ))
    }

    /// Makes the new organization `org` of the kind `kind` with `joined` its first person and
    /// Owner, as glave makes them, named `name` where it is given one, and the person one of
    /// its.
    pub(crate) async fn make_organization(
        &self,
        org: OrgId,
        joined: &PersonJoined,
        kind: OrganizationKind,
        name: Option<String>,
    ) -> Result<(), ApiError> {
        let accounts = self.accounts()?;
        let made = OrganizationMade { kind };
        let owner = RoleSet {
            person: joined.person,
            role: OrgRole::Owner,
            by: Principal::glave(),
        };
        self.record_people(org, |_, next| {
            Ok(Some(vec![
                self.people_event(next, &made)?,
                self.people_event(next.saturating_add(1), joined)?,
                self.people_event(next.saturating_add(2), &owner)?,
            ]))
        })
        .await?;
        if let Some(name) = name {
            let brand = BrandSet {
                name: Some(name),
                logo: None,
                accent: Accent::Plain,
                locked: false,
            };
            let scope = Scope::of_organization(org);
            self.keep_latest(&scope, Reserved::Brand.id(), &[BrandSet::TYPE], &brand)
                .await?;
        }
        accounts.join(joined.person, org).await?;
        Ok(())
    }

    /// Answers `organization/create`: a new organization named `name`, the caller its Owner,
    /// for anyone signed in with a glave account, sign-up open here or not. They join
    /// it with what their organization now knows of them; refused past [`MOST_CREATED`].
    pub(crate) async fn create_organization(
        &self,
        caller: &Caller,
        name: &str,
    ) -> Result<Reply, ApiError> {
        let (person, accounts) = self
            .account_of(caller)
            .ok_or_else(|| ApiError::new(code::NOT_AVAILABLE, Sentence::NoNewOrganizations))?;
        let name = brand_name(Some(name.to_owned()))?
            .ok_or_else(|| ApiError::new(code::INVALID_PARAMS, Sentence::OrganizationUnnamed))?;
        let mut created: usize = 0;
        for org in accounts.organizations(person).await? {
            self.catch_up_people(org).await?;
            let theirs = self.people(org, |people| people.created_by(person));
            created = created.saturating_add(usize::from(theirs));
        }
        if created >= MOST_CREATED {
            return Err(ApiError::new(
                code::LIMIT,
                Sentence::TooManyOrganizations { most: MOST_CREATED },
            ));
        }
        let profile = self.people(self.org_of(caller), |people| {
            people
                .joined(person)
                .map(|joined| joined.profile.clone())
                .unwrap_or_default()
        });
        let joined = PersonJoined {
            person,
            issuer: None,
            subject: None,
            profile: Profile {
                groups: Vec::new(),
                ..profile
            },
        };
        let org = self.new_org_id()?;
        self.make_organization(org, &joined, OrganizationKind::Shared, Some(name.clone()))
            .await?;
        Ok(Reply::Organization {
            organization: OrganizationInfo {
                id: org,
                name: Told::from(name),
                people: 1,
                own: false,
                role: OrgRole::Owner,
                current: false,
            },
        })
    }
}
