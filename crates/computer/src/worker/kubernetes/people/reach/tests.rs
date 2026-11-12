//! Reaching a person's computer, on leases in memory and time paused: a served computer is
//! reached, never replaced, however often reaching it fails; another server's wake is waited for
//! as long as it goes on, past the wait for a holder that does nothing; one not awake, served by
//! another protocol, or left waking by a server that went, is woken under the lease.

use std::sync::{Arc, Mutex};

use glave_ports::testing::{CountingEntropy, ManualClock};
use glave_ports::{Clock, Holder, InMemoryLeases, Leases};

use super::super::lease::Held;
use super::*;
use crate::wire::PROTOCOL;

/// A computer whose Pod each look sees as `pods` says, by how many looks came before (the last
/// stays), and another server holding its lease until `lets_go` looks have passed.
struct Fake {
    pods: Vec<Seen>,
    looks: Mutex<usize>,
    /// How many attaches fail before one goes through.
    failing: Mutex<usize>,
    attached: Mutex<usize>,
    woken: Mutex<usize>,
    kept: Mutex<usize>,
    other: Leasing,
    held: Mutex<Option<Held>>,
    lets_go: usize,
}

impl Machine for Fake {
    async fn seen(&self) -> Result<Seen, ComputerError> {
        let look = {
            let mut looks = self.looks.lock().unwrap();
            *looks += 1;
            *looks
        };
        if look > self.lets_go {
            let held = self.held.lock().unwrap().take();
            if let Some(held) = held {
                self.other.release(&held).await;
            }
        }
        let at = (look - 1).min(self.pods.len() - 1);
        Ok(self.pods[at].clone())
    }

    fn attach(&self, pod: &str) -> impl Future<Output = Result<WorkspaceInfo, ComputerError>> {
        assert_eq!(pod, "pod-1");
        let mut failing = self.failing.lock().unwrap();
        let attached = if *failing > 0 {
            *failing -= 1;
            Err(ComputerError::Unreachable("no answer".into()))
        } else {
            *self.attached.lock().unwrap() += 1;
            Ok(home())
        };
        std::future::ready(attached)
    }

    fn wake(&self) -> impl Future<Output = Result<WorkspaceInfo, ComputerError>> {
        *self.woken.lock().unwrap() += 1;
        std::future::ready(Ok(home()))
    }

    fn keep(&self, _held: Held) {
        *self.kept.lock().unwrap() += 1;
    }
}

fn home() -> WorkspaceInfo {
    let id = uuid::Uuid::from_u128(7);
    WorkspaceInfo {
        id: glave_ports::computer::WorkspaceId::from_uuid(id),
        computer: glave_ports::computer::ComputerId::from_uuid(id),
        root: "/home/glave".into(),
        sources: glave_ports::computer::Sources::Folder {
            path: "/home/glave".into(),
        },
        home: None,
        temp: None,
        at: None,
    }
}

fn served(protocol: &str) -> Seen {
    Seen::Served {
        uid: "pod-1".into(),
        protocol: protocol.into(),
    }
}

/// This server, the fake computer of a person whose lease another server holds (until
/// `lets_go` looks have passed) when `held` says so, and the person.
async fn on_one_store(
    pods: Vec<Seen>,
    held: bool,
    lets_go: usize,
    failing: usize,
) -> (Leasing, Fake, (OrgId, PrincipalId)) {
    let clock = Arc::new(ManualClock::at_millis(1_791_000_000_000));
    let entropy = CountingEntropy::starting_at(1);
    let leases: Arc<dyn Leases> =
        Arc::new(InMemoryLeases::new(Arc::clone(&clock) as Arc<dyn Clock>));
    let server = |holder| Leasing {
        leases: Arc::clone(&leases),
        holder,
    };
    let this = server(Holder::new(clock.as_ref(), &entropy).unwrap());
    let other = server(Holder::new(clock.as_ref(), &entropy).unwrap());
    let person = (
        OrgId::from_uuid(uuid::Uuid::from_u128(1)),
        PrincipalId::new(clock.now(), [3; 10]).unwrap(),
    );
    let taken = if held {
        other.take(person.0, person.1).await.unwrap()
    } else {
        None
    };
    let fake = Fake {
        pods,
        looks: Mutex::new(0),
        failing: Mutex::new(failing),
        attached: Mutex::new(0),
        woken: Mutex::new(0),
        kept: Mutex::new(0),
        other,
        held: Mutex::new(taken),
        lets_go,
    };
    (this, fake, person)
}

const WAIT: Duration = Duration::from_mins(10);

#[test]
fn another_server_s_wake_is_waited_for_as_long_as_it_goes_on_then_reached() {
    glave_ports::block_on!(async {
        tokio::time::pause();
        // Three minutes of waking, longer than a holder doing nothing is waited for.
        let mut pods = vec![Seen::Waking; 90];
        pods.push(served(PROTOCOL));
        let (this, fake, person) = on_one_store(pods, true, usize::MAX, 0).await;
        let started = Instant::now();
        let got = reached(&fake, &this, person, WAIT).await;
        assert!(got.is_ok(), "{got:?}");
        assert!(
            started.elapsed() > lease::LEASE_WAIT,
            "waited through the wake"
        );
        assert_eq!(*fake.attached.lock().unwrap(), 1);
        assert_eq!(*fake.woken.lock().unwrap(), 0, "never woken again");
        assert_eq!(*fake.kept.lock().unwrap(), 0, "the lease is the other's");
    });
}

#[test]
fn a_served_computer_is_reached_however_often_it_fails_and_never_replaced() {
    glave_ports::block_on!(async {
        tokio::time::pause();
        // No server holds the lease, as after a rollout.
        let (this, fake, person) = on_one_store(vec![served(PROTOCOL)], false, 0, 5).await;
        let got = reached(&fake, &this, person, WAIT).await;
        assert!(got.is_ok(), "{got:?}");
        assert_eq!(*fake.woken.lock().unwrap(), 0);
        assert_eq!(
            *fake.kept.lock().unwrap(),
            1,
            "the free lease taken once reached"
        );

        // One that never answers is still never replaced: the call is told why.
        let (this, fake, person) = on_one_store(vec![served(PROTOCOL)], false, 0, usize::MAX).await;
        let refused = reached(&fake, &this, person, WAIT).await.unwrap_err();
        assert!(
            refused.message().contains("could not reach it"),
            "{refused:?}"
        );
        assert_eq!(*fake.woken.lock().unwrap(), 0);
    });
}

#[test]
fn a_holder_doing_nothing_is_waited_for_until_it_lets_go_and_then_it_is_woken() {
    glave_ports::block_on!(async {
        tokio::time::pause();
        // The holder lets go after a minute: woken under this server's lease.
        let (this, fake, person) = on_one_store(vec![Seen::Gone], true, 30, 0).await;
        let got = reached(&fake, &this, person, WAIT).await;
        assert!(got.is_ok(), "{got:?}");
        assert_eq!(*fake.woken.lock().unwrap(), 1);
        assert_eq!(*fake.kept.lock().unwrap(), 1);

        // One that never does: told so once the wait for a holder is over, nothing woken.
        let (this, fake, person) = on_one_store(vec![Seen::Gone], true, usize::MAX, 0).await;
        let started = Instant::now();
        let refused = reached(&fake, &this, person, WAIT).await.unwrap_err();
        assert_eq!(refused.message(), lease::ELSEWHERE);
        assert!(started.elapsed() < Duration::from_mins(3));
        assert_eq!(*fake.woken.lock().unwrap(), 0);
    });
}

#[test]
fn one_left_waking_or_speaking_another_protocol_is_woken_under_the_lease() {
    glave_ports::block_on!(async {
        tokio::time::pause();
        for pod in [Seen::Waking, served("2020-01-01"), Seen::Gone] {
            let (this, fake, person) = on_one_store(vec![pod.clone()], false, 0, 0).await;
            let got = reached(&fake, &this, person, WAIT).await;
            assert!(got.is_ok(), "{pod:?}: {got:?}");
            assert_eq!(*fake.woken.lock().unwrap(), 1, "{pod:?}");
            assert_eq!(*fake.attached.lock().unwrap(), 0, "{pod:?}");
        }
    });
}
