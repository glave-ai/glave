//! A person's computer lease on leases in memory: one server at a time, another refused in plain
//! words; one that cannot renew in time, or finds another holds it, no longer counts it its own.

use std::sync::Mutex;

use glave_ports::testing::{CountingEntropy, ManualClock};
use glave_ports::{BoxFuture, Clock, InMemoryLeases, LeaseError};

use super::*;

/// Leases whose store stops answering once `down` is set, as one failing over does.
struct Flaky {
    leases: InMemoryLeases,
    down: Mutex<bool>,
}

impl Flaky {
    fn failing(&self) -> Option<LeaseError> {
        self.down
            .lock()
            .unwrap()
            .then(|| LeaseError::Storage("the store is down".into()))
    }
}

impl Leases for Flaky {
    fn acquire(
        &self,
        org: OrgId,
        on: Leased,
        holder: Holder,
        ttl: Duration,
    ) -> BoxFuture<'_, Result<Option<Lease>, LeaseError>> {
        match self.failing() {
            Some(error) => Box::pin(std::future::ready(Err(error))),
            None => self.leases.acquire(org, on, holder, ttl),
        }
    }

    fn renew<'a>(
        &'a self,
        lease: &'a Lease,
        ttl: Duration,
    ) -> BoxFuture<'a, Result<Option<Lease>, LeaseError>> {
        match self.failing() {
            Some(error) => Box::pin(std::future::ready(Err(error))),
            None => self.leases.renew(lease, ttl),
        }
    }

    fn release<'a>(&'a self, lease: &'a Lease) -> BoxFuture<'a, Result<(), LeaseError>> {
        self.leases.release(lease)
    }
}

/// Two servers on one store whose clock is `clock`, and a person of `org`.
fn two_servers(clock: &Arc<ManualClock>) -> (Arc<Flaky>, Leasing, Leasing, OrgId, PrincipalId) {
    let entropy = CountingEntropy::starting_at(1);
    let flaky = Arc::new(Flaky {
        leases: InMemoryLeases::new(Arc::clone(clock) as Arc<dyn Clock>),
        down: Mutex::new(false),
    });
    let server = |holder| Leasing {
        leases: Arc::clone(&flaky) as Arc<dyn Leases>,
        holder,
    };
    let first = server(Holder::new(clock.as_ref(), &entropy).unwrap());
    let second = server(Holder::new(clock.as_ref(), &entropy).unwrap());
    let org = OrgId::from_uuid(uuid::Uuid::from_u128(1));
    let person = PrincipalId::new(clock.now(), [3; 10]).unwrap();
    (flaky, first, second, org, person)
}

#[test]
fn another_server_is_refused_while_one_holds_a_computer_and_takes_it_once_it_lapses() {
    let clock = Arc::new(ManualClock::at_millis(1_791_000_000_000));
    let (_, first, second, org, person) = two_servers(&clock);
    glave_ports::block_on!(async {
        let mut held = first.take(org, person).await.unwrap();
        assert!(first.renew(&mut held).await, "renewed by its holder");
        let refused = second.take(org, person).await.unwrap_err();
        assert_eq!(refused.message(), ELSEWHERE);
        let someone_else = PrincipalId::new(clock.now(), [4; 10]).unwrap();
        assert!(
            second.take(org, someone_else).await.is_ok(),
            "another person's is free"
        );

        clock.advance_seconds(31);
        assert!(
            second.take(org, person).await.is_ok(),
            "lapsed, another takes it"
        );
        assert!(
            !first.renew(&mut held).await,
            "the first no longer counts it its own"
        );
    });
}

#[test]
fn a_server_that_cannot_renew_in_time_stops_counting_the_lease_its_own() {
    let clock = Arc::new(ManualClock::at_millis(1_791_000_000_000));
    let (flaky, first, second, org, person) = two_servers(&clock);
    glave_ports::block_on!(async {
        tokio::time::pause();
        let mut held = first.take(org, person).await.unwrap();
        *flaky.down.lock().unwrap() = true;
        tokio::time::advance(RENEW).await;
        assert!(
            first.renew(&mut held).await,
            "one renewal missed: still its own"
        );
        tokio::time::advance(LEASE).await;
        assert!(
            !first.renew(&mut held).await,
            "past its lapse, never its own"
        );
        let failed = second.take(org, person).await.unwrap_err();
        assert!(failed.message().contains("try again"), "{failed:?}");

        // The store back, and the lease lapsed there too: the second takes it.
        *flaky.down.lock().unwrap() = false;
        clock.advance_seconds(31);
        assert!(second.take(org, person).await.is_ok());
    });
}
