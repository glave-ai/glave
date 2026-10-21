//! Who reads and writes which memories (`specs/memory.md`, "Scopes"): a project's own, and the
//! person's own in a project that shares it. Decided from the caller and the thread's project,
//! never from the model, and applied by the store to every read and write.

use std::collections::BTreeMap;

use glave_event::{MemoryScope, PrincipalId};
use glave_ports::memories::{MemoryError, ProjectMemory};
use glave_ports::{ProjectId, Scope};

/// Where a person's own memory lives: the personal project the host provides for them, in their
/// organization (the CLI: `projects::personal`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Personal {
    pub scope: Scope,
    pub person: PrincipalId,
}

/// The rules one deployment's memory runs by.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rules {
    /// The person's memory; `None` when personal memory is off (by choice or by policy).
    pub personal: Option<Personal>,
    /// Each project's setting, where it differs from `default`.
    pub projects: BTreeMap<ProjectId, ProjectMemory>,
    pub default: ProjectMemory,
    /// Memories not saved or edited for this many days are forgotten by glave.
    pub retention_days: Option<u32>,
}

impl Rules {
    /// The setting of `project`.
    pub fn setting(&self, project: ProjectId) -> ProjectMemory {
        self.projects.get(&project).copied().unwrap_or(self.default)
    }

    /// The person's memory, when `person` may use it from `scope`: never across organizations,
    /// never another person's. `None` stands for the local person.
    fn personal_for(&self, person: Option<PrincipalId>, scope: Scope) -> Option<Scope> {
        let personal = self.personal?;
        let theirs = person.is_none_or(|person| person == personal.person);
        (theirs && personal.scope.org() == scope.org()).then_some(personal.scope)
    }

    /// The memories a thread in `scope` reads, each with the ledger that holds it: the
    /// person's first, in a shared project, then the project's.
    pub fn readable(&self, person: Option<PrincipalId>, scope: Scope) -> Vec<(MemoryScope, Scope)> {
        let mut readable = Vec::new();
        match self.setting(scope.project()) {
            ProjectMemory::Off => {}
            ProjectMemory::Isolated => readable.push((MemoryScope::Project, scope)),
            ProjectMemory::Shared => {
                if let Some(personal) = self.personal_for(person, scope) {
                    readable.push((MemoryScope::Personal, personal));
                }
                readable.push((MemoryScope::Project, scope));
            }
        }
        readable
    }

    /// The ledger a memory of `which` lives in, for a caller in `scope`; refused when the project
    /// does not allow it.
    pub(crate) fn place(
        &self,
        person: Option<PrincipalId>,
        scope: Scope,
        which: MemoryScope,
    ) -> Result<Scope, MemoryError> {
        let setting = self.setting(scope.project());
        match (which, setting) {
            (_, ProjectMemory::Off) => {
                Err(MemoryError::Off("memory is off in this project".to_owned()))
            }
            (MemoryScope::Project, _) => Ok(scope),
            (MemoryScope::Personal, ProjectMemory::Isolated) => Err(MemoryError::Refused(
                "this project is isolated: personal memory is neither read nor written here; \
                 save it to the project instead"
                    .to_owned(),
            )),
            (MemoryScope::Personal, ProjectMemory::Shared) => {
                self.personal_for(person, scope).ok_or_else(|| {
                    MemoryError::Refused(
                        "personal memory is off, or belongs to someone else, here; save it to \
                         the project instead"
                            .to_owned(),
                    )
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use glave_ports::OrgId;
    use uuid::Uuid;

    use super::*;

    fn scope(org: u128, project: u128) -> Scope {
        Scope::glaves_own(
            OrgId::from_uuid(Uuid::from_u128(org)),
            ProjectId::from_uuid(Uuid::from_u128(project)),
        )
    }

    fn person(n: u128) -> PrincipalId {
        PrincipalId::try_from(Uuid::from_u128(
            0x0192_f5c0_0000_7000_8000_0000_0000_0000 | n,
        ))
        .unwrap()
    }

    fn rules() -> Rules {
        let mut rules = Rules {
            personal: Some(Personal {
                scope: scope(1, 100),
                person: person(1),
            }),
            ..Rules::default()
        };
        rules
            .projects
            .insert(scope(1, 3).project(), ProjectMemory::Isolated);
        rules
            .projects
            .insert(scope(1, 4).project(), ProjectMemory::Off);
        rules
    }

    #[test]
    fn a_shared_project_reads_the_persons_memory_and_its_own() {
        let rules = rules();
        assert_eq!(
            rules.readable(Some(person(1)), scope(1, 2)),
            [
                (MemoryScope::Personal, scope(1, 100)),
                (MemoryScope::Project, scope(1, 2))
            ]
        );
        assert_eq!(
            rules.place(None, scope(1, 2), MemoryScope::Personal),
            Ok(scope(1, 100))
        );
    }

    #[test]
    fn an_isolated_project_never_reads_or_writes_personal_memory() {
        let rules = rules();
        assert_eq!(
            rules.readable(Some(person(1)), scope(1, 3)),
            [(MemoryScope::Project, scope(1, 3))]
        );
        assert!(matches!(
            rules.place(Some(person(1)), scope(1, 3), MemoryScope::Personal),
            Err(MemoryError::Refused(_))
        ));
    }

    #[test]
    fn off_reads_nothing_and_nothing_crosses_organizations_or_people() {
        let rules = rules();
        assert_eq!(rules.readable(Some(person(1)), scope(1, 4)).len(), 0);
        assert!(
            rules
                .place(None, scope(1, 4), MemoryScope::Project)
                .is_err()
        );
        assert_eq!(
            rules.readable(Some(person(1)), scope(2, 2)),
            [(MemoryScope::Project, scope(2, 2))],
            "another organization never reads this person's memory"
        );
        assert_eq!(
            rules.readable(Some(person(2)), scope(1, 2)),
            [(MemoryScope::Project, scope(1, 2))],
            "another person never reads it"
        );
    }
}
