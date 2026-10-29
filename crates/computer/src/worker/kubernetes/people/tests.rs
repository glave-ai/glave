//! People's computers on a real cluster, as CI's `kind` job runs them (`GLAVE_TEST_KUBE=1`, a
//! real glave and image): a wake started when the app opens and the turn that needs the
//! computer share one Pod, a second ask finds it awake, and nothing is left once it is gone.

use std::time::Instant;

use glave_ports::block_on;
use glave_ports::computer::{Command, Ending, Limits};

use super::super::home::PersonComputer;
use super::super::tests::{cluster, left_behind};
use super::People;
use crate::CancelSwitch;
use crate::worker::backend::Backend as _;
use crate::worker::route::Tail;

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
                let output = Tail::default();
                home.computers
                    .run(
                        home.home.id,
                        &Command::new("echo $HOME; pwd"),
                        &output,
                        &CancelSwitch::default(),
                    )
                    .await
                    .map(|finished| (finished.ending, output.text(1_000)))
                    .map_err(|error| format!("{error:?}"))
            }
            Err(error) => Err(format!("{error:?}")),
        };
        drop(people);
        let destroyed = PersonComputer::new(cluster.clone(), &key, 1)
            .unwrap()
            .destroy()
            .await;
        let left = left_behind(&cluster).await;
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
        let (ending, said) = ran.unwrap();
        assert_eq!(ending, Ending::Exited { code: 0 }, "{said}");
        assert_eq!(said.trim(), "/home/glave\n/home/glave");
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
