//! People's computers on a real cluster, as CI's `kind` job runs them (`GLAVE_TEST_KUBE=1`, a
//! real glave and image): a wake started when the app opens and the turn that needs the
//! computer share one Pod, a second ask finds it awake, and nothing is left once it is gone;
//! held by a turn it stays awake, idle it sleeps on its own, and a turn wakes it again on the
//! same disk.

use std::time::Instant;

use glave_ports::block_on;
use glave_ports::computer::Limits;

use super::super::home::PersonComputer;
use super::super::home::tests::ran;
use super::super::tests::{cluster, left_behind};
use super::People;
use crate::worker::backend::Backend as _;

#[test]
fn a_person_s_computer_wakes_once_however_many_ask() {
    let (Ok(_), Ok(image)) = (
        std::env::var("GLAVE_TEST_WORKER_GLAVE"),
        std::env::var("GLAVE_TEST_WORKER_IMAGE"),
    ) else {
        eprintln!(
            "skipped: set GLAVE_TEST_WORKER_GLAVE (a Linux glave) and GLAVE_TEST_WORKER_IMAGE \
             (an image with bash and git, pinned by digest) to test people's computers"
        );
        return;
    };
    block_on!(async {
        let runtime_class = std::env::var("GLAVE_TEST_KUBE_RUNTIME_CLASS").ok();
        let Some(cluster) = cluster(runtime_class.as_deref(), "people").await else {
            return;
        };
        let limits = Limits {
            isolation: cluster.isolation(),
            ..Limits::default()
        };
        let people = People::new(cluster.clone(), &image, &limits, 1).unwrap();
        let org = crate::local::new_id().unwrap();
        let key = People::key(org, crate::local::new_id().unwrap());

        let waking = Instant::now();
        // The app opens, and a turn asks for the computer before the wake is done.
        let opened = people.wake(&key);
        let (first, second) = tokio::join!(people.home(&key), people.home(&key));
        let woke = waking.elapsed();
        let again = Instant::now();
        let third = people.home(&key).await;
        let at_once = again.elapsed();
        let ran = match &third {
            Ok(home) => {
                ran(
                    home.computers.as_ref(),
                    home.home.computer,
                    "echo $HOME; pwd",
                )
                .await
            }
            Err(error) => Err(format!("{error:?}")),
        };
        let (destroyed, left) = gone(people, &cluster, &key).await;
        eprintln!(
            "a person's computer woke in {} ms, and was found awake in {} ms",
            woke.as_millis(),
            at_once.as_millis()
        );
        opened.unwrap();
        let (first, second, third) = (first.unwrap(), second.unwrap(), third.unwrap());
        assert_eq!(
            first.home.computer, second.home.computer,
            "one wake, one Pod"
        );
        assert_eq!(
            first.home.computer, third.home.computer,
            "awake, not woken again"
        );
        assert_eq!(first.home.root, "/home/glave");
        assert_eq!(ran.unwrap().trim(), "/home/glave\n/home/glave");
        destroyed.unwrap();
        assert_eq!(left, Vec::<String>::new());
    });
}

#[test]
fn a_person_s_key_is_plain_and_their_own() {
    let org = uuid::Uuid::from_u128(0x0192_0000_0000_7000_8000_aaaa_bbbb_cccc);
    let alice = uuid::Uuid::from_u128(0x0192_0000_0000_7000_8000_0000_0000_0001);
    let bob = uuid::Uuid::from_u128(0x0192_0000_0000_7000_8000_0000_0000_0002);
    let key = People::key(org, alice);
    assert_eq!(key, "aaaabbbbcccc-70008000000000000001");
    assert_ne!(key, People::key(org, bob));
    assert!(super::super::home::plain(&key), "{key}");
}

/// Lets go of `people`, removes the computer `key` names with its disk, and what the test
/// left on `cluster`.
async fn gone(
    people: People,
    cluster: &super::super::Kubernetes,
    key: &str,
) -> (
    Result<(), glave_ports::computer::ComputerError>,
    Vec<String>,
) {
    drop(people);
    let destroyed = PersonComputer::new(cluster.clone(), key, 1)
        .unwrap()
        .destroy()
        .await;
    (destroyed, left_behind(cluster).await)
}

/// Whether the Pod `name` is there, or was within a minute: never a panic.
async fn there(cluster: &super::super::Kubernetes, name: &str, wanted: bool) -> bool {
    for _ in 0..120 {
        if matches!(cluster.pods().get_opt(name).await, Ok(pod) if pod.is_some() == wanted) {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    false
}

#[test]
fn a_person_s_computer_sleeps_once_idle_and_a_turn_wakes_it_again() {
    let (Ok(_), Ok(image)) = (
        std::env::var("GLAVE_TEST_WORKER_GLAVE"),
        std::env::var("GLAVE_TEST_WORKER_IMAGE"),
    ) else {
        eprintln!("skipped: set GLAVE_TEST_WORKER_GLAVE and GLAVE_TEST_WORKER_IMAGE");
        return;
    };
    block_on!(async {
        let runtime_class = std::env::var("GLAVE_TEST_KUBE_RUNTIME_CLASS").ok();
        let Some(cluster) = cluster(runtime_class.as_deref(), "sleep").await else {
            return;
        };
        let limits = Limits {
            isolation: cluster.isolation(),
            ..Limits::default()
        };
        let idle = std::time::Duration::from_secs(3);
        let people = People::idle_after(cluster.clone(), &image, &limits, 1, idle).unwrap();
        let key = People::key(
            crate::local::new_id().unwrap(),
            crate::local::new_id().unwrap(),
        );
        let pod = format!("glave-computer-{key}");

        let first = people.home(&key).await;
        let wrote = match &first {
            Ok(home) => {
                ran(
                    home.computers.as_ref(),
                    home.home.computer,
                    "echo kept > kept.txt",
                )
                .await
            }
            Err(error) => Err(format!("{error:?}")),
        };
        // A turn holds it: three idle limits pass and it stays awake.
        let held = people.hold(&key);
        tokio::time::sleep(idle * 3).await;
        let kept_awake = there(&cluster, &pod, true).await;
        drop(held);
        // Let go of, it sleeps on its own: its Pod gone, its disk kept.
        let slept = there(&cluster, &pod, false).await;
        // A turn wakes it again, on the same disk.
        let waking = Instant::now();
        let again = people.home(&key).await;
        let woke = waking.elapsed();
        let read = match &again {
            Ok(home) => ran(home.computers.as_ref(), home.home.computer, "cat kept.txt").await,
            Err(error) => Err(format!("{error:?}")),
        };
        let (destroyed, left) = gone(people, &cluster, &key).await;
        eprintln!(
            "a person's computer woke from sleep in {} ms",
            woke.as_millis()
        );
        wrote.unwrap();
        assert!(kept_awake, "held, it stays awake");
        assert!(slept, "idle, it sleeps");
        assert_ne!(
            first.unwrap().home.computer,
            again.as_ref().unwrap().home.computer,
            "woken again"
        );
        assert_eq!(read.unwrap().trim(), "kept");
        destroyed.unwrap();
        assert_eq!(left, Vec::<String>::new());
    });
}
