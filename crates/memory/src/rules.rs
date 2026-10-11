//! Who reads and writes which memories (`specs/memory.md`, "Scopes"): a space's own, and the
//! person's own in a space that shares it. Decided from the caller and the thread's space,
//! never from the model, and applied by the store to every read and write.

use std::collections::BTreeMap;

use glave_event::{MemoryScope, PrincipalId};
use glave_harness::MemoryError;
use glave_ports::{Scope, SpaceId};

/// What a space allows (`specs/memory.md`): its own memory, the person's too, or none.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SpaceMemory {
    /// Threads read and write the person's memory and the space's.
    #[default]
    Shared,
    /// Threads read and write the space's memory only; the person's never enters.
    Isolated,
    /// Threads neither read nor write memory.
    Off,
}

impl SpaceMemory {
    pub fn named(name: &str) -> Option<Self> {
        match name.trim() {
            "shared" => Some(Self::Shared),
            "isolated" => Some(Self::Isolated),
            "off" => Some(Self::Off),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shared => "shared",
            Self::Isolated => "isolated",
            Self::Off => "off",
        }
    }
}

/// Where a person's own memory lives: the personal space the host provides for them, in their
/// organization. Until spaces are first-class, the host derives it (the CLI: `personal_space`).
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
    /// Each space's setting, where it differs from `default`.
    pub spaces: BTreeMap<SpaceId, SpaceMemory>,
    pub default: SpaceMemory,
    /// Memories not saved or edited for this many days are forgotten by glave.
    pub retention_days: Option<u32>,
}

impl Rules {
    /// The setting of `space`.
    pub fn setting(&self, space: SpaceId) -> SpaceMemory {
        self.spaces.get(&space).copied().unwrap_or(self.default)
    }

    /// The person's memory, when `person` may use it from `scope`: never across organizations,
    /// never another person's. `None` stands for the local person.
    fn personal_for(&self, person: Option<PrincipalId>, scope: Scope) -> Option<Scope> {
        let personal = self.personal?;
        let theirs = person.is_none_or(|person| person == personal.person);
        (theirs && personal.scope.org == scope.org).then_some(personal.scope)
    }

    /// The memories a thread in `scope` reads, each with the ledger that holds it: the
    /// person's first, in a shared space, then the space's.
    pub(crate) fn readable(
        &self,
        person: Option<PrincipalId>,
        scope: Scope,
    ) -> Vec<(MemoryScope, Scope)> {
        let mut readable = Vec::new();
        match self.setting(scope.space) {
            SpaceMemory::Off => {}
            SpaceMemory::Isolated => readable.push((MemoryScope::Space, scope)),
            SpaceMemory::Shared => {
                if let Some(personal) = self.personal_for(person, scope) {
                    readable.push((MemoryScope::Personal, personal));
                }
                readable.push((MemoryScope::Space, scope));
            }
        }
        readable
    }

    /// The ledger a memory of `which` lives in, for a caller in `scope`; refused when the space
    /// does not allow it.
    pub(crate) fn place(
        &self,
        person: Option<PrincipalId>,
        scope: Scope,
        which: MemoryScope,
    ) -> Result<Scope, MemoryError> {
        let setting = self.setting(scope.space);
        match (which, setting) {
            (_, SpaceMemory::Off) => {
                Err(MemoryError::Off("memory is off in this space".to_owned()))
            }
            (MemoryScope::Space, _) => Ok(scope),
            (MemoryScope::Personal, SpaceMemory::Isolated) => Err(MemoryError::Refused(
                "this space is isolated: personal memory is neither read nor written here; \
                 save it to the space instead"
                    .to_owned(),
            )),
            (MemoryScope::Personal, SpaceMemory::Shared) => {
                self.personal_for(person, scope).ok_or_else(|| {
                    MemoryError::Refused(
                        "personal memory is off, or belongs to someone else, here; save it to \
                         the space instead"
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

    fn scope(org: u128, space: u128) -> Scope {
        Scope {
            org: OrgId::from_uuid(Uuid::from_u128(org)),
            space: SpaceId::from_uuid(Uuid::from_u128(space)),
        }
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
            .spaces
            .insert(scope(1, 3).space, SpaceMemory::Isolated);
        rules.spaces.insert(scope(1, 4).space, SpaceMemory::Off);
        rules
    }

    #[test]
    fn a_shared_space_reads_the_persons_memory_and_its_own() {
        let rules = rules();
        assert_eq!(
            rules.readable(Some(person(1)), scope(1, 2)),
            [
                (MemoryScope::Personal, scope(1, 100)),
                (MemoryScope::Space, scope(1, 2))
            ]
        );
        assert_eq!(
            rules.place(None, scope(1, 2), MemoryScope::Personal),
            Ok(scope(1, 100))
        );
    }

    #[test]
    fn an_isolated_space_never_reads_or_writes_personal_memory() {
        let rules = rules();
        assert_eq!(
            rules.readable(Some(person(1)), scope(1, 3)),
            [(MemoryScope::Space, scope(1, 3))]
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
        assert!(rules.place(None, scope(1, 4), MemoryScope::Space).is_err());
        assert_eq!(
            rules.readable(Some(person(1)), scope(2, 2)),
            [(MemoryScope::Space, scope(2, 2))],
            "another organization never reads this person's memory"
        );
        assert_eq!(
            rules.readable(Some(person(2)), scope(1, 2)),
            [(MemoryScope::Space, scope(1, 2))],
            "another person never reads it"
        );
    }
}
