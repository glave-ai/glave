//! The daily sweep of blobs (`specs/storage.md`, "Blobs"): every organization on the store, one
//! at a time, each in its own scope and under a lease, so two servers on one store never sweep
//! one organization at once. A sweep that fails, or a lease another holds, waits for the next
//! day; nothing waits on it, and one organization's failure leaves the rest swept.

use std::time::Duration;

use glave_ports::{BlobStore, Holder, Leased, Leases, OrgId, OsEntropy, Swept, SystemClock};

/// How long one organization's sweep holds its lease, and so how long one a server left
/// unfinished keeps the others off. A sweep running longer overlaps the next, which is harmless:
/// each deletes only what no thread refers to.
const SWEEPING: Duration = Duration::from_hours(1);

/// Deletes the blobs no thread refers to, and bytes no record names, in every organization on
/// the store whose sweep no other server holds.
pub(super) async fn every_organization<B: BlobStore>(blobs: &B, leases: &dyn Leases) {
    let holder = match Holder::new(&SystemClock, &OsEntropy) {
        Ok(holder) => holder,
        Err(error) => {
            tracing::warn!(%error, "the blobs could not be swept");
            return;
        }
    };
    let organizations = match blobs.organizations().await {
        Ok(organizations) => organizations,
        Err(error) => {
            tracing::warn!(%error, "the blobs could not be swept");
            return;
        }
    };
    for org in organizations {
        match one_organization(blobs, leases, holder, org).await {
            Ok(Some(swept)) => {
                tracing::info!(%org, blobs = swept.blobs, orphans = swept.orphans, "blobs swept");
            }
            Ok(None) => tracing::debug!(%org, "another server is sweeping these blobs"),
            Err(error) => tracing::warn!(%org, %error, "the blobs could not be swept"),
        }
    }
}

/// Sweeps `org`, holding its lease as `holder`: what went, or none while another holds it.
async fn one_organization<B: BlobStore>(
    blobs: &B,
    leases: &dyn Leases,
    holder: Holder,
    org: OrgId,
) -> Result<Option<Swept>, String> {
    let taken = leases
        .acquire(org, Leased::BlobSweep, holder, SWEEPING)
        .await
        .map_err(|error| error.to_string())?;
    let Some(lease) = taken else {
        return Ok(None);
    };
    let swept = blobs.sweep(org).await.map_err(|error| error.to_string());
    if let Err(error) = leases.release(&lease).await {
        tracing::warn!(%org, %error, "a sweep's lease could not be let go; it lapses");
    }
    swept.map(Some)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use glave_event::{Sha256, ThreadId};
    use glave_ports::testing::ManualClock;
    use glave_ports::{InMemoryBlobs, InMemoryLeases, Lease, Pieces, ProjectId, Scope, block_on};
    use uuid::Uuid;

    use super::*;

    fn org(n: u128) -> OrgId {
        OrgId::from_uuid(Uuid::from_u128(n))
    }

    /// Keeps a blob in `org` whose thread let go of it, so only a sweep is left to take it.
    async fn released(blobs: &InMemoryBlobs, org: OrgId) -> (Scope, Sha256) {
        let scope = Scope::glaves_own(org, ProjectId::from_uuid(Uuid::from_u128(9)));
        let thread = ThreadId::new(jiff::Timestamp::UNIX_EPOCH, [1; 10]).unwrap();
        let bytes = Box::new(Pieces::from(b"a logo replaced".to_vec()));
        let kept = blobs
            .put(&scope, Some(thread), "image/png", 1024, bytes)
            .await
            .unwrap();
        blobs.release(&scope, thread).await.unwrap();
        (scope, kept.sha256)
    }

    async fn there(blobs: &InMemoryBlobs, (scope, sha256): &(Scope, Sha256)) -> bool {
        blobs.get(scope, sha256).await.unwrap().is_some()
    }

    fn stores() -> (InMemoryBlobs, InMemoryLeases) {
        let clock = Arc::new(ManualClock::at_millis(1_000_000));
        (InMemoryBlobs::new(), InMemoryLeases::new(clock))
    }

    #[test]
    fn every_organization_on_the_store_is_swept_and_its_lease_let_go() {
        block_on!(async {
            let (blobs, leases) = stores();
            let ours = released(&blobs, org(1)).await;
            let theirs = released(&blobs, org(2)).await;
            every_organization(&blobs, &leases).await;
            assert!(!there(&blobs, &ours).await, "the server's own organization");
            assert!(
                !there(&blobs, &theirs).await,
                "one it serves beside its own"
            );
            let other = Holder::new(&SystemClock, &OsEntropy).unwrap();
            let taken: Option<Lease> = leases
                .acquire(org(2), Leased::BlobSweep, other, SWEEPING)
                .await
                .unwrap();
            assert!(taken.is_some(), "let go once it is done");
        });
    }

    #[test]
    fn an_organization_another_server_is_sweeping_is_left_to_it() {
        block_on!(async {
            let (blobs, leases) = stores();
            let held = released(&blobs, org(1)).await;
            let free = released(&blobs, org(2)).await;
            let other = Holder::new(&SystemClock, &OsEntropy).unwrap();
            leases
                .acquire(org(1), Leased::BlobSweep, other, SWEEPING)
                .await
                .unwrap();
            every_organization(&blobs, &leases).await;
            assert!(there(&blobs, &held).await, "held by another server");
            assert!(!there(&blobs, &free).await, "the rest are still swept");
        });
    }
}
