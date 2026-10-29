//! A person's computer on a real cluster, with `GLAVE_TEST_KUBE=1`, a real glave
//! (`GLAVE_TEST_WORKER_GLAVE`) and glave's computer image (`GLAVE_TEST_COMPUTER_IMAGE`,
//! `images/computer`), as CI's `kind` job runs it: a file written before it sleeps is there
//! after it wakes, it runs as `nonroot` and writes nowhere but its home and `/tmp`, its tools
//! are there, it reaches the internet but nothing in the cluster, a bridge into it
//! that drops ends only that bridge, the wake is timed, and destroying it leaves nothing
//! behind.

use std::time::{Duration, Instant};

use glave_ports::block_on;
use glave_ports::computer::{
    Command, ComputerId, Computers, Ending, Environment, Image, Limits, Network, Sources,
};

use super::super::tests::{cluster, left_behind};
use super::PersonComputer;
use crate::worker::backend::Backend as _;
use crate::worker::route::Tail;
use crate::{CancelSwitch, WorkerComputers, WorkerCredentials};

/// What the first wake writes and checks: a file kept on its disk, and the walls around it,
/// each said as a fact so a failure shows which.
const FENCED: &str = "echo kept > kept.txt; echo \"home=$HOME\"; echo \"user=$(id -u)\"; \
    echo \"name=$(whoami) at $(awk -F: '$3 == 65532 { print $6 }' /etc/passwd)\"; \
    for tool in bash git cc make tar python3 uv node npm fnm rustup cargo jq rg ssh curl; do \
    command -v \"$tool\" >/dev/null || echo \"missing=$tool\"; done; \
    touch /tmp/t && echo tmp=writable || echo tmp=read-only; \
    touch /var/tmp/glave-x 2>/dev/null && echo root=writable || echo root=read-only; \
    reach() { timeout 5 bash -c \"</dev/tcp/$1/$2\" 2>/dev/null && echo reached || echo refused; }; \
    python3 -c \"import socket; socket.gethostbyname('kube-dns.kube-system.svc.cluster.local')\" \
    2>/dev/null && echo dns=yes || echo dns=no; \
    echo \"api=$(reach kubernetes.default.svc.cluster.local 443)\"; \
    echo \"pods=$(reach kube-dns.kube-system.svc.cluster.local 9153)\"; \
    echo \"internet=$(reach 1.1.1.1 443)\"";

/// Runs `command` at the person's home on the computer `computers` woke, and what it printed,
/// or why it could not: never a panic, so the test removes what it made before it asserts.
pub(in crate::worker::kubernetes) async fn ran(
    computers: &dyn Computers,
    computer: ComputerId,
    command: &str,
) -> Result<String, String> {
    let home = Sources::Folder {
        path: super::super::objects::PERSON_HOME.into(),
    };
    let opened = computers
        .open(computer, &home)
        .await
        .map_err(|error| format!("open: {error:?}"))?;
    let output = Tail::default();
    let finished = computers
        .run(
            opened.id,
            &Command::new(command),
            &output,
            &CancelSwitch::default(),
        )
        .await
        .map_err(|error| format!("{command}: {error:?}"))?;
    match finished.ending {
        Ending::Exited { code: 0 } => Ok(output.text(1_000)),
        ending => Err(format!("{command}: {ending:?} {}", output.text(1_000))),
    }
}

/// Kills every bridge into the computer, the one carrying this command too, as an API server
/// restarting drops its execs; the name is put together as it runs, so the command's own line
/// does not match it.
const DROP_BRIDGES: &str = "b=brid; b=\"${b}ge\"; for p in /proc/[0-9]*; do \
    case \"$(tr '\\0' ' ' < \"$p/cmdline\" 2>/dev/null)\" in \
    *\"computer $b \"*) kill \"${p#/proc/}\";; esac; done";

/// Wakes the computer, runs `commands` on it one after another, and puts it to sleep again,
/// whatever happened; what the last printed.
async fn awake(
    computers: &WorkerComputers,
    environment: &Environment,
    commands: &[&str],
) -> (Duration, Result<String, String>) {
    let waking = Instant::now();
    let woke = computers.create(environment).await;
    let took = waking.elapsed();
    let Ok(woke) = woke else {
        return (
            took,
            woke.map(|_| String::new())
                .map_err(|e| format!("wake: {e:?}")),
        );
    };
    let mut said = Err("no command".to_owned());
    for command in commands {
        said = ran(computers, woke.id, command).await;
    }
    let slept = computers.destroy(woke.id).await;
    (took, slept.map_err(|e| format!("sleep: {e:?}")).and(said))
}

#[test]
fn a_persons_computer_keeps_its_disk_while_it_sleeps() {
    let (Ok(_), Ok(image)) = (
        std::env::var("GLAVE_TEST_WORKER_GLAVE"),
        std::env::var("GLAVE_TEST_COMPUTER_IMAGE"),
    ) else {
        eprintln!(
            "skipped: set GLAVE_TEST_WORKER_GLAVE (a Linux glave) and GLAVE_TEST_COMPUTER_IMAGE \
             (glave's computer image, pinned by digest) to test a person's computer"
        );
        return;
    };
    block_on!(async {
        let runtime_class = std::env::var("GLAVE_TEST_KUBE_RUNTIME_CLASS").ok();
        let Some(cluster) = cluster(runtime_class.as_deref(), "home").await else {
            return;
        };
        let id = crate::local::new_id().unwrap().simple().to_string();
        let key = format!("test-{}", id.get(id.len() - 12..).unwrap());
        let person = PersonComputer::new(cluster.clone(), &key, 1).unwrap();
        let limits = Limits {
            isolation: cluster.isolation(),
            ..Limits::default()
        };
        let computers = WorkerComputers::on(Box::new(person), limits, WorkerCredentials::None);
        let environment = Environment {
            image: Image::Oci(image),
            network: Some(Network::On),
            ..Environment::host()
        };

        let (_, wrote) = awake(&computers, &environment, &[FENCED]).await;
        // Asleep in between: its Pod gone, its disk kept. Awake, its bridge drops (the call
        // carried on it is lost) and the next call reaches it through a new one.
        let after = [DROP_BRIDGES, "cat kept.txt"];
        let (woke, read) = awake(&computers, &environment, &after).await;
        let destroyed = PersonComputer::new(cluster.clone(), &key, 1)
            .unwrap()
            .destroy()
            .await;
        let left = left_behind(&cluster).await;
        eprintln!("a person's computer woke in {} ms", woke.as_millis());
        assert_eq!(
            wrote.unwrap().trim(),
            "home=/home/glave\nuser=65532\nname=nonroot at /home/glave\ntmp=writable\nroot=read-only\ndns=yes\n\
             api=refused\npods=refused\ninternet=reached",
            "its own home, glave's image's tools, never root, nothing written outside it but \
             /tmp, and the internet but nothing in the cluster"
        );
        assert_eq!(read.unwrap().trim(), "kept");
        destroyed.unwrap();
        assert_eq!(left, Vec::<String>::new());
    });
}

#[test]
fn a_persons_computer_is_named_by_a_plain_key() {
    for key in ["", "Alice", "a_b", "-a", "a-", "a.b", &"a".repeat(41)] {
        assert!(!super::plain(key), "{key:?}");
    }
    assert!(super::plain("org-3f2a-person-9c1b"));
    assert!(super::plain(&"a".repeat(40)));
}
