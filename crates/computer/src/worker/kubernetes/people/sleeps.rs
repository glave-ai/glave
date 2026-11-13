//! A person's computer sleeping and woken again on one server, on leases in memory with time
//! paused and a cluster nothing is asked of (#1387): what one wake leaves behind, the renewal
//! of its lease or a sleep or a keep found late, never acts on a later wake, and a call that
//! comes as the computer sleeps waits for that sleep to let go of the lease, then wakes it.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use glave_ports::computer::{ComputerError, ComputerId, Limits, WorkspaceInfo};
use glave_ports::{
    BoxFuture, Holder, InMemoryLeases, Lease, LeaseError, Leased, Leases, OrgId, SystemClock,
    block_on,
};

use super::super::Kubernetes;
use super::lease::{self, Leasing};
use super::tests::{alone, leasing, someone};
use super::{People, Person, State, holds, planned_home, renew_while_awake, slept, start};

/// People's computers holding their leases as `leasing`, on a cluster nothing is asked of: a
/// test that asked it anything would fail to connect.
fn people(leasing: Leasing) -> People {
    let config = kube::Config::new("http://127.0.0.1:9".parse().unwrap());
    let cluster = Kubernetes {
        client: kube::Client::try_from(config).unwrap(),
        namespace: "unasked".into(),
        runtime_class: None,
        glave: std::path::PathBuf::new(),
        labels: std::collections::BTreeMap::new(),
        hashed: Arc::default(),
    };
    let image =
        "glave-computer@sha256:ce64758a109eb420d874a118f87920e625e12d3634e03b4a5573fd9f6e5d3507";
    People::new(cluster, image, &Limits::default(), 1, leasing).unwrap()
}

/// Wakes `person`'s computer as a wake does, its lease taken and kept, awake with `home`.
async fn woken(person: &Arc<Person>, home: &WorkspaceInfo) {
    let (keeps, home) = (Arc::clone(person), home.clone());
    start(person, move |wake| async move {
        let held = keeps.leasing.take(keeps.org, keeps.id).await?;
        let held = held.ok_or_else(|| ComputerError::Failed("another server holds it".into()))?;
        renew_while_awake(&keeps, held, wake);
        Ok(home)
    });
    let mut state = person.state.subscribe();
    let now = state.wait_for(State::settled).await.unwrap().clone();
    assert!(matches!(now, State::Awake(..)), "woken: {now:?}");
}

/// The home of the computer `key` names as two wakes find it: each wake's computer is its own.
fn homes(key: &str) -> (WorkspaceInfo, WorkspaceInfo) {
    let before = planned_home(key).unwrap();
    let after = WorkspaceInfo {
        computer: ComputerId::from_uuid(crate::local::new_id().unwrap()),
        ..before.clone()
    };
    (before, after)
}

/// The order of #1387: a call found the computer gone on its own, removed what was left, let
/// go of its lease and woke it again; the last wake's renewal came due while the new wake was
/// still reaching it, found its lease let go of, and stopped the new wake.
#[test]
fn the_renewal_left_from_the_last_wake_never_stops_the_next() {
    block_on!(async {
        tokio::time::pause();
        let people = people(alone());
        let (org, id, key) = someone();
        let person = people.person(org, id).unwrap();
        let home = planned_home(&key).unwrap();
        woken(&person, &home).await;
        slept(&person, &home).await;
        start(&person, |_| std::future::pending());
        tokio::time::sleep(lease::RENEW * 2).await;
        let now = person.state.borrow().clone();
        assert!(matches!(now, State::Waking(_)), "still waking: {now:?}");
    });
}

/// Leases in memory whose letting go takes a second, as a store across a network takes a
/// moment, and how often they were let go of.
struct Slow {
    leases: InMemoryLeases,
    released: Mutex<usize>,
}

impl Leases for Slow {
    fn acquire(
        &self,
        org: OrgId,
        on: Leased,
        holder: Holder,
        ttl: Duration,
    ) -> BoxFuture<'_, Result<Option<Lease>, LeaseError>> {
        self.leases.acquire(org, on, holder, ttl)
    }

    fn renew<'a>(
        &'a self,
        lease: &'a Lease,
        ttl: Duration,
    ) -> BoxFuture<'a, Result<Option<Lease>, LeaseError>> {
        self.leases.renew(lease, ttl)
    }

    fn release<'a>(&'a self, lease: &'a Lease) -> BoxFuture<'a, Result<(), LeaseError>> {
        Box::pin(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            *self.released.lock().unwrap() += 1;
            self.leases.release(lease).await
        })
    }
}

/// Calls that come as the computer sleeps: one that finds it gone too waits for that sleep, the
/// one that lets go of the lease, and one that finds it asleep wakes it only once the lease is
/// let go of, so the lease that wake takes stays this server's.
#[test]
fn calls_as_the_computer_sleeps_wait_for_the_one_sleep_to_let_go_then_wake_it() {
    block_on!(async {
        tokio::time::pause();
        let slow = Arc::new(Slow {
            leases: InMemoryLeases::new(Arc::new(SystemClock)),
            released: Mutex::new(0),
        });
        let leases: Arc<dyn Leases> = Arc::clone(&slow) as Arc<dyn Leases>;
        let people = people(leasing(&leases));
        let (org, id, key) = someone();
        let person = people.person(org, id).unwrap();
        let (before, after) = homes(&key);
        woken(&person, &before).await;
        let found_asleep = async {
            let mut state = person.state.subscribe();
            let asleep = state.wait_for(|state| matches!(state, State::Asleep)).await;
            drop(asleep.unwrap());
            woken(&person, &after).await;
        };
        tokio::join!(
            slept(&person, &before),
            slept(&person, &before),
            found_asleep
        );
        assert_eq!(*slow.released.lock().unwrap(), 1, "one sleep let go of it");
        let taken = leasing(&leases).take(org, id).await.unwrap();
        assert!(
            taken.is_none() && holds(&person),
            "the next wake's lease is this server's: {taken:?}"
        );
    });
}

/// What a wake before leaves behind and comes late, a renewal's look putting it to sleep or a
/// keep of its lease, leaves the computer awake from the next wake, and that wake's lease, alone.
#[test]
fn a_sleep_or_a_keep_for_a_wake_before_leaves_the_next_wake_alone() {
    block_on!(async {
        tokio::time::pause();
        let leases: Arc<dyn Leases> = Arc::new(InMemoryLeases::new(Arc::new(SystemClock)));
        let people = people(leasing(&leases));
        let (org, id, key) = someone();
        let person = people.person(org, id).unwrap();
        let (before, after) = homes(&key);
        woken(&person, &before).await;
        slept(&person, &before).await;
        woken(&person, &after).await;
        slept(&person, &before).await;
        let late = person.leasing.take(org, id).await.unwrap().unwrap();
        renew_while_awake(&person, late, 0);
        let now = person.state.borrow().clone();
        assert!(
            matches!(&now, State::Awake(1, home) if home.computer == after.computer),
            "{now:?}"
        );
        let kept = person.held.lock().unwrap().map(|(wake, _)| wake);
        assert_eq!(kept, Some(1), "the next wake's lease is kept as its own");
        let taken = leasing(&leases).take(org, id).await.unwrap();
        assert!(
            taken.is_none() && holds(&person),
            "and is still this server's: {taken:?}"
        );
    });
}
