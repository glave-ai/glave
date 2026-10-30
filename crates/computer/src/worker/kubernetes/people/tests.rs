//! People's computers on a real cluster, as CI's `kind` job runs them (`GLAVE_TEST_KUBE=1`, a
//! real glave and glave's computer image): planned, and a chat's and a project's folder planned
//! on it, nothing runs; the first commands that need it share one wake and one Pod, the next
//! finds it awake, and nothing is left once it is gone; held by a turn it stays awake, unasked
//! it sleeps on its own, and a turn's command wakes it again on the same disk.

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

        // Planned, a chat's folder and a project's on it too: nothing asked of the cluster.
        let home = people.home(&key).unwrap();
        let scratch = Sources::Scratch {
            name: "chat".into(),
            owner: None,
        };
        let chat = home.computers.open(home.home.computer, &scratch).await;
        let path = "/home/glave/glave/app".to_owned();
        let folder = Sources::Folder { path: path.clone() };
        let project = home.computers.open(home.home.computer, &folder).await;
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
        assert_eq!(project.unwrap().root, path);
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

#[test]
fn a_person_s_computer_has_the_same_id_on_every_server() {
    let org = uuid::Uuid::from_u128(0x0192_0000_0000_7000_8000_aaaa_bbbb_cccc);
    let alice = People::key(org, uuid::Uuid::from_u128(1));
    let bob = People::key(org, uuid::Uuid::from_u128(2));
    let planned = |key: &str| super::planned_home(key).unwrap().computer;
    assert_eq!(
        planned(&alice),
        planned(&alice),
        "asked again, or by another server"
    );
    assert_ne!(planned(&alice), planned(&bob));
    assert!(super::planned_home("").is_err(), "no key names no computer");
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

/// glave's computer image, when a real glave and it are given; `None`, and a note, otherwise.
fn computer_image() -> Option<String> {
    let (Ok(_), Ok(image)) = (
        std::env::var("GLAVE_TEST_WORKER_GLAVE"),
        std::env::var("GLAVE_TEST_COMPUTER_IMAGE"),
    ) else {
        eprintln!("skipped: set GLAVE_TEST_WORKER_GLAVE and GLAVE_TEST_COMPUTER_IMAGE");
        return None;
    };
    Some(image)
}

#[test]
fn a_person_s_computer_sleeps_once_idle_and_a_turn_wakes_it_again() {
    let Some(image) = computer_image() else {
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

/// The CPU and memory the Pod `name`'s container runs at now, as its status says, if it is
/// there.
async fn running_at(cluster: &super::super::Kubernetes, name: &str) -> Option<serde_json::Value> {
    let pod = cluster.pods().get_opt(name).await.ok().flatten()?;
    let pod = serde_json::to_value(pod).ok()?;
    let limits = pod.pointer("/status/containerStatuses/0/resources/limits")?;
    Some(serde_json::json!({"cpu": limits.get("cpu")?, "memory": limits.get("memory")?}))
}

/// A person's computer that runs out of memory restarts with twice as much, and one held at its
/// CPU limit grows in place, each saying so to whoever ran the command.
#[test]
fn a_person_s_computer_grows_for_what_a_command_runs_short_of() {
    let Some(image) = computer_image() else {
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
        // Three loops at once use up each period's quarter of a CPU early, so the computer is
        // held at its limit for most of the run, well past half.
        let busy = "for _ in 1 2 3; do timeout 12 sh -c 'while :; do :; done' & done; wait; \
            grep throttled_usec /sys/fs/cgroup/cpu.stat; echo busy";
        let mut seen = Vec::new();
        let mut at = Vec::new();
        if let Ok(chat) = home
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
            for script in ["true", big, big, busy] {
                seen.push(outcome(&home, chat.id, script).await);
                at.push(running_at(&cluster, &pod).await);
            }
        }
        let (destroyed, left) = gone(people, &cluster, &key).await;
        eprintln!("ran: {seen:#?}\nat: {at:#?}");
        let woke = serde_json::json!({"cpu": "250m", "memory": "512Mi"});
        assert_eq!(at.first().cloned().flatten(), Some(woke), "it wakes small");
        let lost = seen.get(1).cloned().unwrap().unwrap_err();
        assert!(
            lost.contains(
                "glave: this computer ran out of memory at 512 MiB and restarts with 1 GiB"
            ),
            "{lost}"
        );
        let (ending, said) = seen.get(2).cloned().unwrap().unwrap();
        assert_eq!(
            (ending, said.trim()),
            (Ending::Exited { code: 0 }, "allocated")
        );
        let bigger = serde_json::json!({"cpu": "250m", "memory": "1Gi"});
        assert_eq!(at.get(2).cloned().flatten(), Some(bigger), "woken bigger");
        let (ending, said) = seen.get(3).cloned().unwrap().unwrap();
        assert_eq!(ending, Ending::Exited { code: 0 });
        assert!(
            said.contains("glave: this computer now has 0.5 CPUs."),
            "{said}"
        );
        let faster = serde_json::json!({"cpu": "500m", "memory": "1Gi"});
        assert_eq!(at.get(3).cloned().flatten(), Some(faster), "grown in place");
        destroyed.unwrap();
        assert_eq!(left, Vec::<String>::new());
    });
}
