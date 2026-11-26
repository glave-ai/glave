//! The daily housekeeping of memory in every organization on the store (`specs/memory.md`,
//! "Forgetting"): in each, under a lease, every project that has memories has what is past the
//! retention forgotten, what is due erased and its key rotated after a forget, whoever's it is
//! and whether it is in use, so a forget anywhere leaves backups within a day (#1486). One at a time, as the blob sweep goes
//! (`sweep.rs`): an organization another server holds, or one that fails, waits for the next day.

use std::time::Duration;

use glave_memory::Memories;
use glave_ports::{EventLog, Holder, Leased, Leases, OrgId};

use super::leased;

/// The longest one organization's erasure runs, holding its lease for as long. Stopped then,
/// what it had not reached waits for the next day.
const ERASING: Duration = Duration::from_hours(1);

/// Erases what is due in each of `organizations` whose erasure no other server holds, as
/// `holder`.
pub(super) async fn every_organization<L: EventLog + 'static>(
    organizations: Vec<OrgId>,
    memories: &Memories<L>,
    leases: &dyn Leases,
    holder: Holder,
) {
    let job = (holder, Leased::MemoryErasure, ERASING);
    let work = |org| memories.housekeep_in(org);
    leased::every_organization(organizations, leases, job, work, |org, done| match done {
        Ok(Some(done)) => {
            let (forgotten, erased, rotated) = (done.forgotten, done.erased, done.rotated);
            tracing::info!(%org, forgotten, erased, rotated, "memory housekept");
        }
        Ok(None) => tracing::debug!(%org, "another server is erasing this memory"),
        // The store failing: what is due waits on someone acting.
        Err(error) => {
            let error = format!("{error:#}");
            tracing::error!(%org, %error, "memory could not be erased");
        }
    })
    .await;
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use glave_memory::Rules;
    use glave_ports::testing::ManualClock;
    use glave_ports::{InMemoryLeases, OsEntropy, SystemClock, block_on};
    use uuid::Uuid;

    use super::*;
    use crate::data::Data;

    fn org(n: u128) -> OrgId {
        OrgId::from_uuid(Uuid::from_u128(n))
    }

    fn holder(n: u128) -> Holder {
        Holder::from_uuid(Uuid::from_u128(n))
    }

    /// Each organization is erased under its lease, let go of once done; one another server is
    /// erasing is left to it.
    #[test]
    fn each_organization_is_erased_under_its_lease_and_one_held_elsewhere_is_left() {
        block_on!(async {
            let store = Data::in_memory().unwrap();
            let memories = Memories::new(
                store.log(),
                store.eraser(),
                Rules::default(),
                Box::new(SystemClock),
                Box::new(OsEntropy),
            );
            let leases = InMemoryLeases::new(Arc::new(ManualClock::at_millis(1_000_000)));
            let taken = |org: OrgId, by: Holder| {
                let leases = &leases;
                async move {
                    leases
                        .acquire(org, Leased::MemoryErasure, by, ERASING)
                        .await
                        .unwrap()
                        .is_some()
                }
            };
            assert!(taken(org(2), holder(2)).await, "another server erasing");
            every_organization(vec![org(1), org(2)], &memories, &leases, holder(1)).await;
            assert!(taken(org(1), holder(3)).await, "let go once it is done");
            assert!(
                !taken(org(2), holder(3)).await,
                "left to the server holding it"
            );
        });
    }
}
