//! A person's computer on a real cluster, with `GLAVE_TEST_KUBE=1`, a real glave
//! (`GLAVE_TEST_WORKER_GLAVE`) and glave's computer image (`GLAVE_TEST_COMPUTER_IMAGE`,
//! `images/computer`, built with that glave in it), as CI's `kind` job runs it: a file written
//! before it sleeps is there after it wakes, it serves with its image's glave under the runtime
//! class set, it runs as `nonroot` and writes nowhere but its home and `/tmp`, its tools are
//! there, it reaches the internet but nothing in the cluster (the API, CoreDNS by name and
//! another Pod by its address), a bridge into it that drops ends only that bridge and the next
//! is signed in with a fresh token, the wake is timed, and destroying it leaves nothing behind.
//! Given another glave than its image's, glave is copied in.

use std::time::{Duration, Instant};

use glave_ports::block_on;
use glave_ports::computer::{
    Command, ComputerId, Computers, Ending, Environment, Image, Limits, Sources,
};

use super::super::Kubernetes;
use super::super::tests::{another_pod, cluster, key, left_behind, with_glave};
use super::PersonComputer;
use crate::CancelSwitch;
use crate::worker::WorkerComputers;
use crate::worker::route::Tail;

/// What the first wake writes and checks: a file kept on its disk, and the walls around it,
/// each said as a fact so a failure shows which.
const FENCED: &str = "echo kept > kept.txt; echo \"home=$HOME\"; echo \"user=$(id -u)\"; \
    echo \"glave=$(readlink /opt/glave/glave)\"; \
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

/// The SHA-256 of the computer's token, never the token: which one it holds now.
const TOKEN_HELD: &str = "sha256sum /opt/glave/secret/token | cut -c1-64";

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

/// What one wake showed: how long it took, the runtime class its Pod ran under, and what each
/// command printed, in order.
struct Woke {
    took: Duration,
    class: Option<String>,
    said: Vec<Result<String, String>>,
}

/// Wakes the computer `key` names on `cluster`, runs `commands` on it one after another, and
/// puts it to sleep again, whatever happened; or why it could not.
async fn awake(
    computers: &WorkerComputers,
    (cluster, key): (&Kubernetes, &str),
    environment: &Environment,
    commands: &[&str],
) -> Result<Woke, String> {
    let waking = Instant::now();
    let woke = computers.create(environment).await;
    let took = waking.elapsed();
    let woke = woke.map_err(|e| format!("wake: {e:?}"))?;
    let pod = cluster
        .pods()
        .get_opt(&format!("glave-computer-{key}"))
        .await;
    let class = pod
        .ok()
        .flatten()
        .and_then(|pod| pod.spec?.runtime_class_name);
    let mut said = Vec::new();
    for command in commands {
        said.push(ran(computers, woke.id, command).await);
    }
    computers
        .destroy(woke.id)
        .await
        .map_err(|e| format!("sleep: {e:?}"))?;
    Ok(Woke { took, class, said })
}

/// What the last of `woke`'s commands printed, or why there is none.
fn last(woke: &Result<Woke, String>) -> Result<String, String> {
    match woke {
        Ok(woke) => woke
            .said
            .last()
            .cloned()
            .unwrap_or_else(|| Err("no command".into())),
        Err(why) => Err(why.clone()),
    }
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
        let key = key("test");
        let person = PersonComputer::new(cluster.clone(), &key, 1).unwrap();
        let limits = Limits {
            isolation: cluster.isolation(),
            ..Limits::default()
        };
        let computers = WorkerComputers::new(person, limits.clone());
        let environment = Environment {
            image: Image::Oci(image),
            ..Environment::host()
        };
        let pod = another_pod(&cluster)
            .await
            .unwrap_or_else(|| "no-pod-found".into());
        let fenced = format!("{FENCED}; echo \"pod=$(reach {pod} 9153)\"");

        let named = (&cluster, key.as_str());
        let wrote = awake(&computers, named, &environment, &[&fenced]).await;
        // Asleep in between: its Pod gone, its disk kept. Awake, its bridges drop (the call
        // carried on one is lost) and the next call reaches it through a new one, signed in
        // with a fresh token.
        let after = [TOKEN_HELD, DROP_BRIDGES, TOKEN_HELD, "cat kept.txt"];
        let read = awake(&computers, named, &environment, &after).await;
        // Given another glave than its image's, the same computer has it copied in.
        let copied = {
            let folder = glave_ports::testing::TempDir::new("another-glave").unwrap();
            let cluster = another_glave(&cluster, &folder);
            let person = PersonComputer::new(cluster.clone(), &key, 1).unwrap();
            let computers = WorkerComputers::new(person, limits.clone());
            let which = "echo \"link=$(readlink /opt/glave/glave)\"; \
                test -x /opt/glave/glave && echo copied";
            awake(&computers, (&cluster, &key), &environment, &[which]).await
        };
        let destroyed = PersonComputer::new(cluster.clone(), &key, 1)
            .unwrap()
            .destroy()
            .await;
        let left = left_behind(&cluster).await;
        let took =
            |woke: &Result<Woke, String>| woke.as_ref().ok().map(|woke| woke.took.as_millis());
        eprintln!(
            "a person's computer woke in {:?} ms with its image's glave, and in {:?} ms with \
             glave copied in",
            took(&read),
            took(&copied)
        );
        assert_eq!(last(&copied).unwrap().trim(), "link=\ncopied");
        assert_eq!(
            last(&wrote).unwrap().trim(),
            "home=/home/glave\nuser=65532\nglave=/usr/libexec/glave/glave\n\
             name=nonroot at /home/glave\ntmp=writable\nroot=read-only\ndns=yes\n\
             api=refused\npods=refused\ninternet=reached\npod=refused",
            "its own home, glave's image's glave and tools, never root, nothing written \
             outside it but /tmp, and the internet but nothing in the cluster"
        );
        assert_eq!(
            wrote.as_ref().unwrap().class,
            runtime_class,
            "it runs under the runtime class set"
        );
        renewed_and_kept(&read.unwrap());
        destroyed.unwrap();
        assert_eq!(left, Vec::<String>::new());
    });
}

/// What the second wake said: the token it held before its bridges dropped, then the one the
/// next bridge brought, and the file kept on its disk.
fn renewed_and_kept(read: &Woke) {
    let held = |at: usize| read.said.get(at).cloned().unwrap().unwrap();
    let (before, renewed) = (held(0), held(2));
    assert_eq!(
        (before.len(), renewed.len()),
        (64, 64),
        "{before} {renewed}"
    );
    assert_ne!(
        before, renewed,
        "the new bridge is signed in with a fresh token"
    );
    assert_eq!(held(3).trim(), "kept");
}

/// The computers suite, on a person's computer reached through bridges into its Pod as the
/// server reaches it, under gVisor, where glave's sandbox, which the suite runs, starts: all
/// of it but what needs a git host on the runner, which a Pod never reaches.
#[test]
fn a_persons_computer_passes_the_computers_suite() {
    let (Ok(_), Ok(image), Ok(class)) = (
        std::env::var("GLAVE_TEST_WORKER_GLAVE"),
        std::env::var("GLAVE_TEST_COMPUTER_IMAGE"),
        std::env::var("GLAVE_TEST_KUBE_RUNTIME_CLASS"),
    ) else {
        eprintln!(
            "skipped: set GLAVE_TEST_WORKER_GLAVE, GLAVE_TEST_COMPUTER_IMAGE and a gVisor \
             GLAVE_TEST_KUBE_RUNTIME_CLASS to run the suite on a person's computer"
        );
        return;
    };
    if !class.contains("gvisor") {
        eprintln!("skipped: the suite runs glave's sandbox, which starts here under gVisor");
        return;
    }
    block_on!(async {
        let Some(cluster) = cluster(Some(&class), "suite").await else {
            return;
        };
        let key = key("suite");
        let limits = Limits {
            isolation: cluster.isolation(),
            ..Limits::default()
        };
        let person = PersonComputer::new(cluster.clone(), &key, 2).unwrap();
        let computers = WorkerComputers::new(person, limits);
        let environment = Environment {
            image: Image::Oci(image),
            ..Environment::host()
        };
        // The empty folder the suite works in, made on the disk before its own wake.
        let made = awake(
            &computers,
            (&cluster, &key),
            &environment,
            &["mkdir -p suite"],
        )
        .await;
        let scratch = format!("{}/suite", super::super::objects::PERSON_HOME);
        let verdict = match last(&made) {
            Ok(_) => {
                glave_ports::conformance::computers_apart(&computers, &environment, &scratch).await
            }
            Err(why) => Err(why),
        };
        // The suite ends by putting it to sleep; one that failed before may have left it awake.
        let destroyed = PersonComputer::new(cluster.clone(), &key, 2)
            .unwrap()
            .destroy()
            .await;
        let left = left_behind(&cluster).await;
        assert_eq!(verdict, Ok(()));
        destroyed.unwrap();
        assert_eq!(left, Vec::<String>::new());
    });
}

/// `cluster` with another glave than the one it was given, in `folder`: the same program with a
/// byte after it, which it never reads, so its SHA-256 is not its image's glave's.
fn another_glave(cluster: &Kubernetes, folder: &glave_ports::testing::TempDir) -> Kubernetes {
    let mut bytes = std::fs::read(&cluster.glave).unwrap();
    bytes.push(0);
    with_glave(cluster, folder, &bytes)
}

#[test]
fn a_persons_computer_is_named_by_a_plain_key() {
    for key in ["", "Alice", "a_b", "-a", "a-", "a.b", &"a".repeat(41)] {
        assert!(!super::plain(key), "{key:?}");
    }
    assert!(super::plain("org-3f2a-person-9c1b"));
    assert!(super::plain(&"a".repeat(40)));
}
