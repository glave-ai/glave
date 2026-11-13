//! A person's computer sleeping and woken again on one server, on leases in memory with time
//! paused and a cluster nothing is asked of (#1387): what one wake leaves behind, the renewal
//! of its lease, its word, or a sleep or a keep found late, never acts on a later wake; a call
//! that comes as the computer sleeps waits for that sleep to let go of the lease, then wakes
//! it; and a turn takes a free lease, but never as the computer is put to sleep.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use glave_ports::computer::{ComputerError, Limits, WorkspaceInfo};
use glave_ports::{
    BoxFuture, Holder, InMemoryLeases, Lease, LeaseError, Leased, Leases, OrgId, SystemClock,
    block_on,
};

use super::super::super::Kubernetes;
use super::super::lease::{self, Leasing};
use super::super::{
    People, Person, State, holds, planned_home, renew_while_awake, slept, start, woken,
};
use super::{alone, leasing, someone};

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

/// `person`'s computer's state once no wake goes on.
async fn settled(person: &Person) -> State {
    let mut state = person.state.subscribe();
    state.wait_for(State::settled).await.unwrap().clone()
}

/// Wakes `person`'s computer as a wake does, its lease taken and kept, awake with `home`: the
/// number of that wake.
async fn wake_up(person: &Arc<Person>, home: &WorkspaceInfo) -> u64 {
    let (keeps, home) = (Arc::clone(person), home.clone());
    start(person, move |wake| async move {
        let held = keeps.leasing.take(keeps.org, keeps.id).await?;
        let held = held.ok_or_else(|| ComputerError::Failed("another server holds it".into()))?;
        renew_while_awake(&keeps, held, wake);
        Ok(home)
    });
    match settled(person).await {
        State::Awake(wake, _) => wake,
        now => panic!("not woken: {now:?}"),
    }
}

/// The order of #1387: a call found the computer gone on its own, removed what was left, let
/// go of its lease and woke it again; the last wake's renewal came due while the new wake was
/// still reaching it, found its lease let go of, and stopped the new wake.
#[test]
fn the_renewal_left_from_the_last_wake_never_stops_the_next() {
    block_on!(async {
        tokio::time::pause();
        let slow = slow(Duration::ZERO, Duration::ZERO);
        let people = people(leasing(&(Arc::clone(&slow) as Arc<dyn Leases>)));
        let (org, id, key) = someone();
        let person = people.person(org, id).unwrap();
        let home = planned_home(&key).unwrap();
        let first = wake_up(&person, &home).await;
        slept(&person, first, &home).await;
        start(&person, |_| std::future::pending());
        tokio::time::sleep(lease::RENEW * 2).await;
        let now = person.state.borrow().clone();
        assert!(matches!(now, State::Waking(_)), "still waking: {now:?}");
        let renewed = *slow.renewed.lock().unwrap();
        assert_eq!(renewed, 0, "nothing renewed for a wake that ended");
    });
}

/// Leases in memory whose renewing and letting go take `renewing` and `releasing`, as a store
/// across a network takes a moment, and how often they were renewed and let go of.
struct Slow {
    leases: InMemoryLeases,
    renewing: Duration,
    releasing: Duration,
    renewed: Mutex<usize>,
    released: Mutex<usize>,
}

fn slow(renewing: Duration, releasing: Duration) -> Arc<Slow> {
    Arc::new(Slow {
        leases: InMemoryLeases::new(Arc::new(SystemClock)),
        renewing,
        releasing,
        renewed: Mutex::new(0),
        released: Mutex::new(0),
    })
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
        Box::pin(async move {
            tokio::time::sleep(self.renewing).await;
            *self.renewed.lock().unwrap() += 1;
            self.leases.renew(lease, ttl).await
        })
    }

    fn release<'a>(&'a self, lease: &'a Lease) -> BoxFuture<'a, Result<(), LeaseError>> {
        Box::pin(async move {
            tokio::time::sleep(self.releasing).await;
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
        let slow = slow(Duration::ZERO, Duration::from_secs(1));
        let leases: Arc<dyn Leases> = Arc::clone(&slow) as Arc<dyn Leases>;
        let people = people(leasing(&leases));
        let (org, id, key) = someone();
        let person = people.person(org, id).unwrap();
        let home = planned_home(&key).unwrap();
        let first = wake_up(&person, &home).await;
        let found_asleep = async {
            let mut state = person.state.subscribe();
            let asleep = state.wait_for(|state| matches!(state, State::Asleep)).await;
            drop(asleep.unwrap());
            wake_up(&person, &home).await;
        };
        tokio::join!(
            slept(&person, first, &home),
            slept(&person, first, &home),
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
/// keep of its lease, leaves the computer awake from the next wake, on the same computer, and
/// that wake's lease, alone.
#[test]
fn a_sleep_or_a_keep_for_a_wake_before_leaves_the_next_wake_alone() {
    block_on!(async {
        tokio::time::pause();
        let leases: Arc<dyn Leases> = Arc::new(InMemoryLeases::new(Arc::new(SystemClock)));
        let people = people(leasing(&leases));
        let (org, id, key) = someone();
        let person = people.person(org, id).unwrap();
        let home = planned_home(&key).unwrap();
        let first = wake_up(&person, &home).await;
        slept(&person, first, &home).await;
        let next = wake_up(&person, &home).await;
        slept(&person, first, &home).await;
        let late = person.leasing.take(org, id).await.unwrap().unwrap();
        renew_while_awake(&person, late, first);
        let now = person.state.borrow().clone();
        assert!(
            matches!(now, State::Awake(wake, _) if wake == next),
            "{now:?}"
        );
        let kept = person.held.lock().unwrap().map(|(wake, _)| wake);
        assert_eq!(kept, Some(next), "the next wake's lease is kept as its own");
        let taken = leasing(&leases).take(org, id).await.unwrap();
        assert!(
            taken.is_none() && holds(&person),
            "and is still this server's: {taken:?}"
        );
    });
}

/// A renewal already renewing as the computer sleeps and is woken again finds the lease let go
/// of, which was its own wake's, so it stops nothing; and that wake's word, should it come
/// late, leaves the next wake alone too.
#[test]
fn a_renewal_or_a_word_that_comes_late_leaves_the_next_wake_alone() {
    block_on!(async {
        tokio::time::pause();
        let slow = slow(Duration::from_secs(1), Duration::ZERO);
        let people = people(leasing(&(Arc::clone(&slow) as Arc<dyn Leases>)));
        let (org, id, key) = someone();
        let person = people.person(org, id).unwrap();
        let home = planned_home(&key).unwrap();
        let first = wake_up(&person, &home).await;
        // Its renewal came due, and is renewing as the computer sleeps and is woken again.
        tokio::time::sleep(lease::RENEW + Duration::from_millis(500)).await;
        slept(&person, first, &home).await;
        start(&person, |_| std::future::pending());
        tokio::time::sleep(Duration::from_secs(1)).await;
        let now = person.state.borrow().clone();
        assert!(matches!(now, State::Waking(_)), "still waking: {now:?}");
        let late = home.clone();
        woken(Arc::clone(&person), first, async { Ok(late) }).await;
        let now = person.state.borrow().clone();
        assert!(matches!(now, State::Waking(_)), "still waking: {now:?}");
    });
}

/// A turn's heartbeat takes the free lease of a computer awake, so one in use has a holder, but
/// never as it is put to sleep, where the lease it took would outlive the sleep.
#[test]
fn a_turn_takes_a_free_lease_but_never_as_the_computer_is_put_to_sleep() {
    block_on!(async {
        tokio::time::pause();
        // Asked how it is every 15 seconds.
        let people = people(alone()).idle_after(1);
        let (org, id, key) = someone();
        let person = people.person(org, id).unwrap();
        // Awake with no lease of this server's, as when another server held it as it was
        // reached.
        let home = planned_home(&key).unwrap();
        start(&person, move |_| std::future::ready(Ok(home)));
        let now = settled(&person).await;
        assert!(matches!(now, State::Awake(..)), "{now:?}");
        let turn = people.hold(org, id).unwrap();
        let sleeping = person.sleeping.lock().await;
        tokio::time::sleep(Duration::from_secs(20)).await;
        assert!(!holds(&person), "not taken as it is put to sleep");
        drop(sleeping);
        tokio::time::sleep(Duration::from_secs(15)).await;
        assert!(holds(&person), "taken once it is not");
        drop(turn);
    });
}
