//! Leases held in memory, on an injected clock: the in-memory adapter of the leases port, for
//! a server alone on its store and for tests.

use std::collections::HashMap;
use std::future::ready;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use jiff::{SignedDuration, Timestamp};

use crate::env::Clock;
use crate::leases::{Holder, Lease, LeaseError, Leased, Leases, ttl_millis};
use crate::scope::OrgId;
use crate::tools::BoxFuture;

/// Each lease held: its holder, and when it lapses.
type Held = HashMap<(OrgId, Leased), (Holder, Timestamp)>;

/// Leases held in memory, timed by `clock`. Nothing survives the process.
pub struct InMemoryLeases {
    clock: Arc<dyn Clock>,
    held: Mutex<Held>,
}

impl InMemoryLeases {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            held: Mutex::new(HashMap::new()),
        }
    }

    fn held(&self) -> Result<MutexGuard<'_, Held>, LeaseError> {
        self.held
            .lock()
            .map_err(|_| LeaseError::Storage("the in-memory leases' lock is poisoned".into()))
    }

    /// When a lease taken now for `ttl` lapses, and now.
    fn lapsing(&self, ttl: Duration) -> (Timestamp, Timestamp) {
        let now = self.clock.now();
        let expires = now
            .checked_add(SignedDuration::from_millis(ttl_millis(ttl)))
            .unwrap_or(Timestamp::MAX);
        (expires, now)
    }

    fn acquire_now(
        &self,
        org: OrgId,
        on: Leased,
        holder: Holder,
        ttl: Duration,
    ) -> Result<Option<Lease>, LeaseError> {
        let (expires, now) = self.lapsing(ttl);
        let mut held = self.held()?;
        let taken = match held.get(&(org, on)) {
            Some((other, until)) => *other == holder || *until <= now,
            None => true,
        };
        if !taken {
            return Ok(None);
        }
        held.insert((org, on), (holder, expires));
        Ok(Some(Lease {
            org,
            on,
            holder,
            expires,
        }))
    }

    fn renew_now(&self, lease: &Lease, ttl: Duration) -> Result<Option<Lease>, LeaseError> {
        let (expires, _) = self.lapsing(ttl);
        let mut held = self.held()?;
        match held.get_mut(&(lease.org, lease.on)) {
            Some((holder, until)) if *holder == lease.holder => {
                *until = expires;
                Ok(Some(Lease { expires, ..*lease }))
            }
            _ => Ok(None),
        }
    }

    fn release_now(&self, lease: &Lease) -> Result<(), LeaseError> {
        let mut held = self.held()?;
        if held
            .get(&(lease.org, lease.on))
            .is_some_and(|(holder, _)| *holder == lease.holder)
        {
            held.remove(&(lease.org, lease.on));
        }
        Ok(())
    }
}

impl Leases for InMemoryLeases {
    fn acquire(
        &self,
        org: OrgId,
        on: Leased,
        holder: Holder,
        ttl: Duration,
    ) -> BoxFuture<'_, Result<Option<Lease>, LeaseError>> {
        Box::pin(ready(self.acquire_now(org, on, holder, ttl)))
    }

    fn renew<'a>(
        &'a self,
        lease: &'a Lease,
        ttl: Duration,
    ) -> BoxFuture<'a, Result<Option<Lease>, LeaseError>> {
        Box::pin(ready(self.renew_now(lease, ttl)))
    }

    fn release<'a>(&'a self, lease: &'a Lease) -> BoxFuture<'a, Result<(), LeaseError>> {
        Box::pin(ready(self.release_now(lease)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_on;
    use crate::testing::ManualClock;
    use uuid::Uuid;

    #[test]
    fn in_memory_leases_pass_the_leases_suite() {
        let clock = ManualClock::at_millis(1_791_000_000_000);
        let leases = InMemoryLeases::new(Arc::new(clock));
        assert_eq!(block_on!(crate::conformance::leases(&leases)), Ok(()));
    }

    #[test]
    fn a_lease_is_held_until_its_time_on_the_clock_and_renewing_moves_it() {
        let clock = ManualClock::at_millis(1_791_000_000_000);
        let leases = InMemoryLeases::new(Arc::new(clock.clone()));
        let org = OrgId::from_uuid(Uuid::from_u128(1));
        let (first, second) = (
            Holder::from_uuid(Uuid::from_u128(2)),
            Holder::from_uuid(Uuid::from_u128(3)),
        );
        let minute = Duration::from_secs(60);
        block_on!(async {
            let lease = leases.acquire(org, Leased::Scheduler, first, minute).await;
            let lease = lease.unwrap().unwrap();
            clock.advance_seconds(59);
            let taken = leases.acquire(org, Leased::Scheduler, second, minute).await;
            assert_eq!(taken, Ok(None), "held until its minute is up");
            let renewed = leases.renew(&lease, minute).await.unwrap().unwrap();
            assert_eq!(renewed.expires, clock.now() + SignedDuration::from_secs(60));
            clock.advance_seconds(59);
            let taken = leases.acquire(org, Leased::Scheduler, second, minute).await;
            assert_eq!(taken, Ok(None), "renewing gave it another minute");
            clock.advance_seconds(1);
            let taken = leases.acquire(org, Leased::Scheduler, second, minute).await;
            assert!(taken.unwrap().is_some(), "lapsed, it is anyone's");
            assert_eq!(leases.renew(&renewed, minute).await, Ok(None));
        });
    }
}
