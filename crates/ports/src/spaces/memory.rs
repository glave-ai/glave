//! Spaces held in memory: the in-memory adapter of the spaces port, for tests and for runs
//! that keep nothing.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};

use glave_event::Principal;

use super::{FolderKey, Space, SpaceError, SpaceKind, Spaces};
use crate::scope::{OrgId, SpaceId};
use crate::tools::BoxFuture;

#[derive(Debug, Default)]
struct Held {
    spaces: BTreeMap<(OrgId, SpaceId), Space>,
    folders: BTreeMap<(OrgId, String), SpaceId>,
}

/// Spaces held in memory. Nothing survives the process.
#[derive(Debug, Default)]
pub struct MemorySpaces {
    held: Mutex<Held>,
}

impl MemorySpaces {
    pub fn new() -> Self {
        Self::default()
    }

    fn held(&self) -> Result<MutexGuard<'_, Held>, SpaceError> {
        self.held
            .lock()
            .map_err(|_| SpaceError::Storage("the in-memory spaces' lock is poisoned".into()))
    }

    fn create_now(&self, org: OrgId, space: &Space) -> Result<(), SpaceError> {
        Space::check_name(&space.name)?;
        let mut held = self.held()?;
        insert(&mut held, org, space)
    }

    fn list_now(
        &self,
        org: OrgId,
        member: &Principal,
        limit: usize,
    ) -> Result<Vec<Space>, SpaceError> {
        let held = self.held()?;
        Ok(held
            .spaces
            .iter()
            .filter(|((owner, _), space)| *owner == org && space.admits(member))
            .map(|(_, space)| space.clone())
            .take(limit)
            .collect())
    }

    fn update_now(&self, org: OrgId, space: &Space) -> Result<(), SpaceError> {
        Space::check_name(&space.name)?;
        let mut held = self.held()?;
        let kept = held
            .spaces
            .get_mut(&(org, space.id))
            .ok_or(SpaceError::NotFound(space.id))?;
        kept.name.clone_from(&space.name);
        kept.members.clone_from(&space.members);
        kept.updated_at = space.updated_at;
        Ok(())
    }

    fn personal_now(&self, org: OrgId, new: &Space) -> Result<Space, SpaceError> {
        if new.kind != SpaceKind::Personal {
            return Err(SpaceError::Invalid(
                "a personal space must be personal".into(),
            ));
        }
        let mut held = self.held()?;
        if let Some(found) = personal_of(&held, org, &new.owner) {
            return Ok(found);
        }
        insert(&mut held, org, new)?;
        Ok(new.clone())
    }

    fn folder_now(&self, org: OrgId, keys: &[FolderKey], new: &Space) -> Result<Space, SpaceError> {
        let mut held = self.held()?;
        let found = keys
            .iter()
            .find_map(|key| held.folders.get(&(org, key.stored())).copied());
        let space = if let Some(id) = found {
            held.spaces
                .get(&(org, id))
                .cloned()
                .ok_or(SpaceError::NotFound(id))?
        } else {
            Space::check_name(&new.name)?;
            insert(&mut held, org, new)?;
            new.clone()
        };
        for key in keys {
            held.folders.entry((org, key.stored())).or_insert(space.id);
        }
        Ok(space)
    }
}

fn personal_of(held: &Held, org: OrgId, owner: &Principal) -> Option<Space> {
    held.spaces
        .iter()
        .find(|((at, _), space)| {
            *at == org && space.kind == SpaceKind::Personal && space.owner == *owner
        })
        .map(|(_, space)| space.clone())
}

fn insert(held: &mut Held, org: OrgId, space: &Space) -> Result<(), SpaceError> {
    if held.spaces.contains_key(&(org, space.id)) {
        return Err(SpaceError::Conflict(format!(
            "space {} exists already",
            space.id
        )));
    }
    if space.kind == SpaceKind::Personal && personal_of(held, org, &space.owner).is_some() {
        return Err(SpaceError::Conflict(
            "this person has a personal space already".into(),
        ));
    }
    held.spaces.insert((org, space.id), space.clone());
    Ok(())
}

impl Spaces for MemorySpaces {
    fn create<'a>(&'a self, org: OrgId, space: &'a Space) -> BoxFuture<'a, Result<(), SpaceError>> {
        Box::pin(std::future::ready(self.create_now(org, space)))
    }

    fn get(&self, org: OrgId, space: SpaceId) -> BoxFuture<'_, Result<Option<Space>, SpaceError>> {
        let found = self
            .held()
            .map(|held| held.spaces.get(&(org, space)).cloned());
        Box::pin(std::future::ready(found))
    }

    fn list<'a>(
        &'a self,
        org: OrgId,
        member: &'a Principal,
        limit: usize,
    ) -> BoxFuture<'a, Result<Vec<Space>, SpaceError>> {
        Box::pin(std::future::ready(self.list_now(org, member, limit)))
    }

    fn update<'a>(&'a self, org: OrgId, space: &'a Space) -> BoxFuture<'a, Result<(), SpaceError>> {
        Box::pin(std::future::ready(self.update_now(org, space)))
    }

    fn personal<'a>(
        &'a self,
        org: OrgId,
        new: &'a Space,
    ) -> BoxFuture<'a, Result<Space, SpaceError>> {
        Box::pin(std::future::ready(self.personal_now(org, new)))
    }

    fn folder<'a>(
        &'a self,
        org: OrgId,
        keys: &'a [FolderKey],
        new: &'a Space,
    ) -> BoxFuture<'a, Result<Space, SpaceError>> {
        Box::pin(std::future::ready(self.folder_now(org, keys, new)))
    }
}

#[cfg(test)]
mod tests {
    use super::MemorySpaces;
    use crate::conformance;

    #[test]
    fn memory_spaces_pass_the_spaces_conformance_suite() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a current-thread runtime always builds");
        assert_eq!(
            runtime.block_on(conformance::spaces(&MemorySpaces::new())),
            Ok(())
        );
    }
}
