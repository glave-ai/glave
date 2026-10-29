//! People's computers on a real cluster, as CI's `kind` job runs them (`GLAVE_TEST_KUBE=1`, a
//! real glave and glave's computer image): planned, and a chat's folder planned on it, nothing runs; the first
//! commands that need it share one wake and one Pod, the next finds it awake, and nothing is
//! left once it is gone; held by a turn it stays awake, unasked it sleeps on its own, and a
//! turn's command wakes it again on the same disk.

use std::time::Instant;

use glave_ports::block_on;
use glave_ports::computer::{Command, Ending, Home, Limits, Sources, WorkspaceId};

use super::super::home::PersonComputer;
use super::super::home::tests::ran;
use super::super::tests::{cluster, left_behind};
use super::People;
use crate::CancelSwitch;
use crate::worker::backend::Backend as _;
use crate::worker::route::Tail;

#[test]
fn a_person_s_computer_is_woken_once_by_the_first_calls_that_need_it() {
    let (Ok(_), Ok(image)) = (
        std::env::var("GLAVE_TEST_WORKER_GLAVE"),
        std::env::var("GLAVE_TEST_COMPUTER_IMAGE"),
    ) else {
        eprintln!(
            "skipped: set GLAVE_TEST_WORKER_GLAVE (a Linux glave) and GLAVE_TEST_COMPUTER_IMAGE \
             (glave's computer image, pinned by digest) to test people's computers"
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

        let pod = format!("glave-computer-{key}");

        // Planned, a chat's folder on it too: nothing asked of the cluster.
        let home = people.home(&key).unwrap();
        let chat = home
            .computers
            .open(
                home.home.computer,
                &Sources::Scratch {
                    name: "chat".into(),
                    owner: None,
                },
            )
            .await;
        let asleep = phase(&cluster, &pod).await;
        // Two calls that need it at once: one wake, one Pod.
        let waking = Instant::now();
        let (first, second) = match &chat {
            Ok(chat) => {
                tokio::join!(
                    command(&home, chat.id, "echo $HOME; pwd"),
                    command(&home, chat.id, "true")
                )
            }
            Err(error) => (Err(format!("{error:?}")), Err(String::new())),
        };
        let woke = waking.elapsed();
        let again = Instant::now();
        let third = match &chat {
            Ok(chat) => command(&home, chat.id, "true").await,
            Err(error) => Err(format!("{error:?}")),
        };
        let at_once = again.elapsed();
        let (destroyed, left) = gone(people, &cluster, &key).await;
        eprintln!(
            "a person's computer woke for its first command in {} ms, and was found awake in {} \
             ms",
            woke.as_millis(),
            at_once.as_millis()
        );
        assert_eq!(asleep, None, "planning wakes nothing");
        assert_eq!(chat.unwrap().root, "/home/glave/.glave/scratch/chat");
        assert_eq!(
            first.unwrap().trim(),
            "/home/glave\n/home/glave/.glave/scratch/chat"
        );
        second.unwrap();
        third.unwrap();
        destroyed.unwrap();
        assert_eq!(left, Vec::<String>::new());
    });
}

/// Runs `script` in `workspace` on `home`, and what it printed, or why it could not: never a
/// panic, so the test removes what it made before it asserts.
async fn command(home: &Home, workspace: WorkspaceId, script: &str) -> Result<String, String> {
    match outcome(home, workspace, script).await? {
        (Ending::Exited { code: 0 }, said) => Ok(said),
        (ending, said) => Err(format!("{script}: {ending:?} {said}")),
    }
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

/// Whether the Pod `name` ended (its glave exited) within three minutes: never a panic.
async fn ended(cluster: &super::super::Kubernetes, name: &str) -> bool {
    for _ in 0..360 {
        if matches!(
            phase(cluster, name).await.as_deref(),
            Some("Succeeded" | "Failed")
        ) {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    false
}

/// The uid of the Pod `name`, if it is there.
async fn uid(cluster: &super::super::Kubernetes, name: &str) -> Option<String> {
    cluster
        .pods()
        .get_opt(name)
        .await
        .ok()
        .flatten()?
        .metadata
        .uid
}

/// The phase the Pod `name` is in, if it is there.
async fn phase(cluster: &super::super::Kubernetes, name: &str) -> Option<String> {
    let pod = cluster.pods().get_opt(name).await.ok().flatten()?;
    pod.status?.phase
}

#[test]
fn a_person_s_computer_sleeps_once_idle_and_a_turn_wakes_it_again() {
    let (Ok(_), Ok(image)) = (
        std::env::var("GLAVE_TEST_WORKER_GLAVE"),
        std::env::var("GLAVE_TEST_COMPUTER_IMAGE"),
    ) else {
        eprintln!("skipped: set GLAVE_TEST_WORKER_GLAVE and GLAVE_TEST_COMPUTER_IMAGE");
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
        // Asleep after a minute unasked, so a turn holding it asks every 15 seconds.
        let people = People::new(cluster.clone(), &image, &limits, 1)
            .unwrap()
            .idle_after(1);
        let key = People::key(
            crate::local::new_id().unwrap(),
            crate::local::new_id().unwrap(),
        );
        let pod = format!("glave-computer-{key}");

        let home = people.home(&key).unwrap();
        let wrote = ran(
            home.computers.as_ref(),
            home.home.computer,
            "echo kept > kept.txt",
        )
        .await;
        let first = uid(&cluster, &pod).await;
        // A turn holds it: two idle limits and more pass and it stays awake.
        let held = people.hold(&key);
        tokio::time::sleep(std::time::Duration::from_secs(150)).await;
        let kept_awake = phase(&cluster, &pod).await;
        drop(held);
        // Let go of, it sleeps on its own: its glave ends, so its Pod does, its disk kept.
        let slept = ended(&cluster, &pod).await;
        // A turn's command wakes it again, on the same disk.
        let waking = Instant::now();
        let read = ran(home.computers.as_ref(), home.home.computer, "cat kept.txt").await;
        let woke = waking.elapsed();
        let again = uid(&cluster, &pod).await;
        let (destroyed, left) = gone(people, &cluster, &key).await;
        eprintln!(
            "a person's computer woke from sleep in {} ms",
            woke.as_millis()
        );
        wrote.unwrap();
        assert_eq!(
            kept_awake.as_deref(),
            Some("Running"),
            "held, it stays awake"
        );
        assert!(slept, "idle, it sleeps");
        assert!(first.is_some());
        assert_ne!(first, again, "woken again, as a Pod of its own");
        assert_eq!(read.unwrap().trim(), "kept");
        destroyed.unwrap();
        assert_eq!(left, Vec::<String>::new());
    });
}

/// Runs `script` in `workspace` on `home`: how it ended and what it printed, or why it could
/// not run.
async fn outcome(
    home: &Home,
    workspace: WorkspaceId,
    script: &str,
) -> Result<(Ending, String), String> {
    let output = Tail::default();
    let finished = home
        .computers
        .run(
            workspace,
            &Command::new(script),
            &output,
            &CancelSwitch::default(),
        )
        .await
        .map_err(|error| format!("{script}: {error:?}"))?;
    Ok((finished.ending, output.text(2_000)))
}

/// The memory the Pod `name`'s container is limited to now, if it is there.
async fn memory_limit(cluster: &super::super::Kubernetes, name: &str) -> Option<String> {
    let pod = cluster.pods().get_opt(name).await.ok().flatten()?;
    let pod = serde_json::to_value(pod).ok()?;
    pod.pointer("/spec/containers/0/resources/limits/memory")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

#[test]
fn a_person_s_computer_that_runs_out_of_memory_grows_in_place() {
    let (Ok(_), Ok(image)) = (
        std::env::var("GLAVE_TEST_WORKER_GLAVE"),
        std::env::var("GLAVE_TEST_COMPUTER_IMAGE"),
    ) else {
        eprintln!("skipped: set GLAVE_TEST_WORKER_GLAVE and GLAVE_TEST_COMPUTER_IMAGE");
        return;
    };
    block_on!(async {
        let runtime_class = std::env::var("GLAVE_TEST_KUBE_RUNTIME_CLASS").ok();
        let Some(cluster) = cluster(runtime_class.as_deref(), "grows").await else {
            return;
        };
        let limits = Limits {
            isolation: cluster.isolation(),
            ..Limits::default()
        };
        let people = People::new(cluster.clone(), &image, &limits, 1).unwrap();
        let key = People::key(
            crate::local::new_id().unwrap(),
            crate::local::new_id().unwrap(),
        );
        let pod = format!("glave-computer-{key}");
        let home = people.home(&key).unwrap();
        let big = "python3 -c 'b = bytearray(700 * 1024 * 1024); print(\"allocated\")'";
        let (first, small, second) = match home
            .computers
            .open(
                home.home.computer,
                &Sources::Scratch {
                    name: "big".into(),
                    owner: None,
                },
            )
            .await
        {
            Ok(chat) => {
                let first = outcome(&home, chat.id, big).await;
                let small = memory_limit(&cluster, &pod).await;
                (first, small, outcome(&home, chat.id, big).await)
            }
            Err(error) => (Err(format!("{error:?}")), None, Err(String::new())),
        };
        let grown = memory_limit(&cluster, &pod).await;
        let (destroyed, left) = gone(people, &cluster, &key).await;
        eprintln!("first: {first:?}\nsecond: {second:?}");
        let (ending, said) = first.unwrap();
        assert_eq!(ending, Ending::Signalled, "killed for memory: {said}");
        assert!(
            said.contains("glave: this computer now has 1 GiB of memory. Run the command again."),
            "grown in place: {said}"
        );
        assert_eq!(
            small.as_deref(),
            Some("1Gi"),
            "its Pod's limit, grown in place"
        );
        let (ending, said) = second.unwrap();
        assert_eq!(
            (ending, said.trim()),
            (Ending::Exited { code: 0 }, "allocated")
        );
        assert_eq!(grown.as_deref(), Some("1Gi"));
        destroyed.unwrap();
        assert_eq!(left, Vec::<String>::new());
    });
}
