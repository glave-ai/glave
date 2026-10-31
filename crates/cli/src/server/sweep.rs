//! The daily sweep of blobs (`specs/storage.md`, "Blobs"): every organization on the store, one
//! at a time, each in its own scope and under a lease, so two servers on one store never sweep
//! one organization at once. A sweep that fails, or a lease another holds, waits for the next
//! day; nothing waits on it, one organization's failure leaves the rest swept, and a sweep that
//! outruns its lease is stopped, so one organization never holds up the others for long.

use std::time::Duration;

use anyhow::Context as _;
use glave_ports::{BlobStore, Holder, Leased, Leases, OrgId, Swept, spoken};
use tokio::time::timeout;

/// The longest one organization's sweep runs, holding its lease for as long, and so how long one
/// a server left unfinished keeps others off. Stopped then, what it had not reached waits for the
/// next day; it deletes only what no thread refers to, so stopping it loses nothing.
const SWEEPING: Duration = Duration::from_hours(1);

/// Deletes the blobs no thread refers to, and bytes no record names, in each of `organizations`
/// whose sweep no other server holds, as `holder`.
pub(super) async fn every_organization<B: BlobStore>(
    organizations: Vec<OrgId>,
    blobs: &B,
    leases: &dyn Leases,
    holder: Holder,
) {
    for org in organizations {
        match one_organization(blobs, leases, holder, org).await {
            Ok(Some(swept)) => {
                tracing::info!(%org, blobs = swept.blobs, orphans = swept.orphans, "blobs swept");
            }
            Ok(None) => tracing::debug!(%org, "another server is sweeping these blobs"),
            Err(error) => {
                let error = format!("{error:#}");
                tracing::warn!(%org, %error, "the blobs could not be swept");
            }
        }
    }
}

/// Sweeps `org`, holding its lease as `holder`: what went, or none while another holds it.
async fn one_organization<B: BlobStore>(
    blobs: &B,
    leases: &dyn Leases,
    holder: Holder,
    org: OrgId,
) -> anyhow::Result<Option<Swept>> {
    let taken = leases
        .acquire(org, Leased::BlobSweep, holder, SWEEPING)
        .await
        .context("could not take its lease")?;
    let Some(lease) = taken else {
        return Ok(None);
    };
    let swept = timeout(SWEEPING, blobs.sweep(org)).await;
    if let Err(error) = leases.release(&lease).await {
        tracing::warn!(%org, %error, "a sweep's lease could not be let go; it lapses");
    }
    let swept = swept.with_context(|| format!("it was not done within {}", spoken(SWEEPING)))?;
    Ok(Some(swept?))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use glave_event::{Sha256, ThreadId};
    use glave_ports::testing::ManualClock;
    use glave_ports::{
        BoxFuture, InMemoryBlobs, InMemoryLeases, Lease, LeaseError, Pieces, ProjectId, Scope,
        block_on,
    };
    use jiff::Timestamp;
    use uuid::Uuid;

    use super::*;

    fn org(n: u128) -> OrgId {
        OrgId::from_uuid(Uuid::from_u128(n))
    }

    /// Keeps a blob in `org` whose thread let go of it, so only a sweep is left to take it.
    async fn released(blobs: &InMemoryBlobs, org: OrgId) -> (Scope, Sha256) {
        let scope = Scope::glaves_own(org, ProjectId::from_uuid(Uuid::from_u128(9)));
        let thread = ThreadId::new(Timestamp::UNIX_EPOCH, [1; 10]).unwrap();
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

    fn clock() -> Arc<ManualClock> {
        Arc::new(ManualClock::at_millis(1_000_000))
    }

    fn holder(n: u128) -> Holder {
        Holder::from_uuid(Uuid::from_u128(n))
    }

    #[test]
    fn every_organization_is_swept_and_its_lease_let_go() {
        block_on!(async {
            let (blobs, leases) = (InMemoryBlobs::new(), InMemoryLeases::new(clock()));
            let ours = released(&blobs, org(1)).await;
            let theirs = released(&blobs, org(2)).await;
            every_organization(vec![org(1), org(2)], &blobs, &leases, holder(1)).await;
            assert!(!there(&blobs, &ours).await, "the server's own organization");
            assert!(
                !there(&blobs, &theirs).await,
                "one it serves beside its own"
            );
            let taken: Option<Lease> = leases
                .acquire(org(2), Leased::BlobSweep, holder(2), SWEEPING)
                .await
                .unwrap();
            assert!(taken.is_some(), "let go once it is done");
        });
    }

    #[test]
    fn an_organization_another_server_is_sweeping_is_left_to_it() {
        block_on!(async {
            let (blobs, leases) = (InMemoryBlobs::new(), InMemoryLeases::new(clock()));
            let held = released(&blobs, org(1)).await;
            let free = released(&blobs, org(2)).await;
            leases
                .acquire(org(1), Leased::BlobSweep, holder(2), SWEEPING)
                .await
                .unwrap();
            every_organization(vec![org(1), org(2)], &blobs, &leases, holder(1)).await;
            assert!(there(&blobs, &held).await, "held by another server");
            assert!(!there(&blobs, &free).await, "the rest are still swept");
        });
    }

    /// Leases that can't be taken in the organization `broken`, and as `inner` otherwise.
    struct Failing {
        inner: InMemoryLeases,
        broken: OrgId,
    }

    impl Leases for Failing {
        fn acquire(
            &self,
            org: OrgId,
            on: Leased,
            holder: Holder,
            ttl: Duration,
        ) -> BoxFuture<'_, Result<Option<Lease>, LeaseError>> {
            if org == self.broken {
                return Box::pin(async {
                    Err(LeaseError::Storage("the store is away".to_owned()))
                });
            }
            self.inner.acquire(org, on, holder, ttl)
        }

        fn renew<'a>(
            &'a self,
            lease: &'a Lease,
            ttl: Duration,
        ) -> BoxFuture<'a, Result<Option<Lease>, LeaseError>> {
            self.inner.renew(lease, ttl)
        }

        fn release<'a>(&'a self, lease: &'a Lease) -> BoxFuture<'a, Result<(), LeaseError>> {
            self.inner.release(lease)
        }
    }

    #[test]
    fn one_organization_s_failure_leaves_the_rest_swept() {
        block_on!(async {
            let blobs = InMemoryBlobs::new();
            let leases = Failing {
                inner: InMemoryLeases::new(clock()),
                broken: org(1),
            };
            let failed = released(&blobs, org(1)).await;
            let rest = released(&blobs, org(2)).await;
            every_organization(vec![org(1), org(2)], &blobs, &leases, holder(1)).await;
            assert!(
                there(&blobs, &failed).await,
                "its sweep waits for the next day"
            );
            assert!(!there(&blobs, &rest).await);
        });
    }
}
