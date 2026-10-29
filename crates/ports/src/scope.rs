//! The scope every storage call carries: which organization and which project.

use std::fmt;

use glave_event::{Principal, PrincipalId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::projects::Project;

/// Identifies an organization. A personal computer has exactly one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OrgId(Uuid);

impl OrgId {
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    pub fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl fmt::Display for OrgId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Identifies a project: the boundary for memory, history, files and policy.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(transparent)]
pub struct ProjectId(Uuid);

impl ProjectId {
    /// The organization's own place among its projects' ([`Scope::of_organization`]): no
    /// project's id, since projects' are made from the clock.
    const ORGANIZATION: Self = Self(Uuid::from_u128(0x0000_0000_0000_7000_8000_0000_0000_0000));

    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    pub fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl fmt::Display for ProjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Where a storage call reads and writes. There is no storage call without one, so code
/// cannot reach across organizations or projects; and there is no scope without a reason to
/// hold it, so code cannot read where no one was checked (`specs/invariants.md`, 17). It is
/// made only two ways, each easy to find:
///
/// - [`Scope::of_owner`], for what a person asks: only once they are found to own the
///   project, read from the store in their organization. The session API's one check makes it.
/// - [`Scope::glaves_own`], for glave's own work that no one asks for at that moment (the
///   scheduler, sweeps, recovery, the ledgers it keeps, the host opening its own folder), and
///   for tests. Every call is one to audit.
///
/// A scope found from a bare thread id is a [`Located`], never a scope: it must be checked
/// before anything is read with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Scope {
    org: OrgId,
    project: ProjectId,
}

impl Scope {
    /// The scope of `project`, in the organization `org` it was read from, for `person` when
    /// they own it. `None` otherwise.
    pub fn of_owner(org: OrgId, project: &Project, person: PrincipalId) -> Option<Self> {
        (project.owner == Principal::person(person)).then_some(Self {
            org,
            project: project.id,
        })
    }

    /// The scope of `project` in `org` for glave's own work, which no person's ownership
    /// decides.
    pub fn glaves_own(org: OrgId, project: ProjectId) -> Self {
        Self { org, project }
    }

    /// The scope of the organization's own streams, which are no project's: its people ledger
    /// (`specs/model.md`, "Roles"). glave's own, like [`Scope::glaves_own`]: the session reads
    /// it for a caller only to check their role, and answers it only to an admin.
    pub fn of_organization(org: OrgId) -> Self {
        Self {
            org,
            project: ProjectId::ORGANIZATION,
        }
    }

    pub fn org(self) -> OrgId {
        self.org
    }

    pub fn project(self) -> ProjectId {
        self.project
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "org {} / project {}", self.org, self.project)
    }
}

/// Where the store found a thread by its bare id (`EventLog::locate`): which organization and
/// project hold it. A hint, not a scope: that the caller owns `project` must be checked, making
/// a [`Scope`], before anything of the thread is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Located {
    pub org: OrgId,
    pub project: ProjectId,
}

/// A scope is read back only where glave wrote it itself: the memory server reading the
/// context its own harness sends with each request ([`crate::memories::MemoryContext`]). So
/// it is glave's own.
impl<'de> Deserialize<'de> for Scope {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Written {
            org: OrgId,
            project: ProjectId,
        }
        let written = Written::deserialize(deserializer)?;
        Ok(Self::glaves_own(written.org, written.project))
    }
}

#[cfg(test)]
mod tests {
    use glave_event::At;

    use super::*;
    use crate::projects::ProjectKind;

    fn person(n: u128) -> PrincipalId {
        PrincipalId::try_from(Uuid::from_u128(
            0x0192_f5c0_0000_7000_8000_0000_0000_0000 + n,
        ))
        .unwrap()
    }

    #[test]
    fn only_the_owner_gets_a_scope_and_it_is_the_projects_in_its_organization() {
        let at = At::try_from("2026-10-01T00:00:00.000Z".to_owned()).unwrap();
        let project = Project {
            id: ProjectId::from_uuid(Uuid::from_u128(2)),
            kind: ProjectKind::Project,
            name: "garden".into(),
            owner: Principal::person(person(1)),
            created_at: at,
            updated_at: at,
        };
        let org = OrgId::from_uuid(Uuid::from_u128(1));
        let expected = Scope::glaves_own(org, project.id);
        assert_eq!(Scope::of_owner(org, &project, person(1)), Some(expected));
        assert_eq!(Scope::of_owner(org, &project, person(2)), None);
        let written = serde_json::to_value(expected).unwrap();
        assert_eq!(serde_json::from_value::<Scope>(written).unwrap(), expected);
    }
}
