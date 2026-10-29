//! The directory in memory: the in-memory adapter of the directory port, for tests and a
//! server alone with an in-memory store.

use std::collections::BTreeMap;
use std::future::ready;
use std::sync::{Mutex, MutexGuard};

use glave_event::PrincipalId;

use crate::directory::{Directory, DirectoryError, IdentityKey};
use crate::scope::OrgId;
use crate::tools::BoxFuture;

#[derive(Default)]
struct Kept {
    identities: BTreeMap<IdentityKey, PrincipalId>,
    /// Each person's organizations, the first joined first.
    memberships: BTreeMap<PrincipalId, Vec<OrgId>>,
}

/// The directory in memory. Nothing survives the process.
#[derive(Default)]
pub struct InMemoryDirectory {
    kept: Mutex<Kept>,
}

impl InMemoryDirectory {
    fn kept(&self) -> Result<MutexGuard<'_, Kept>, DirectoryError> {
        self.kept.lock().map_err(|_| {
            DirectoryError::Storage("the in-memory directory's lock is poisoned".into())
        })
    }
}

impl Directory for InMemoryDirectory {
    fn link<'a>(
        &'a self,
        identity: &'a IdentityKey,
        person: PrincipalId,
    ) -> BoxFuture<'a, Result<PrincipalId, DirectoryError>> {
        let linked = self
            .kept()
            .map(|mut kept| *kept.identities.entry(identity.clone()).or_insert(person));
        Box::pin(ready(linked))
    }

    fn find<'a>(
        &'a self,
        identity: &'a IdentityKey,
    ) -> BoxFuture<'a, Result<Option<PrincipalId>, DirectoryError>> {
        let found = self
            .kept()
            .map(|kept| kept.identities.get(identity).copied());
        Box::pin(ready(found))
    }

    fn join(&self, person: PrincipalId, org: OrgId) -> BoxFuture<'_, Result<(), DirectoryError>> {
        let joined = self.kept().map(|mut kept| {
            let orgs = kept.memberships.entry(person).or_default();
            if !orgs.contains(&org) {
                orgs.push(org);
            }
        });
        Box::pin(ready(joined))
    }

    fn organizations(
        &self,
        person: PrincipalId,
    ) -> BoxFuture<'_, Result<Vec<OrgId>, DirectoryError>> {
        let orgs = self
            .kept()
            .map(|kept| kept.memberships.get(&person).cloned().unwrap_or_default());
        Box::pin(ready(orgs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_on;

    #[test]
    fn the_in_memory_directory_passes_the_directory_conformance_suite() {
        let directory = InMemoryDirectory::default();
        assert_eq!(block_on!(crate::conformance::directory(&directory)), Ok(()));
    }
}
