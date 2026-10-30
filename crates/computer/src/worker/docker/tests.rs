//! The Docker backend against a real Docker Engine, only with `GLAVE_TEST_DOCKER=1` (CI's Linux
//! runner sets it; elsewhere these tests say they skipped). A small image pinned by digest
//! runs a stub in glave's place, so the lifecycle is tested without a Linux build of glave:
//! the limits, the files put in before start, no network or a port on 127.0.0.1 only, the
//! stream to a worker without a network, and removal. Everything made carries the label
//! `glave.test=<run>` (CI sweeps the run by it) and `glave.test.case=<id>`, one per test, so
//! each test checks that it left nothing behind without seeing the tests running beside it.
//! `GLAVE_TEST_WORKER_GLAVE` (a Linux glave) and `GLAVE_TEST_WORKER_IMAGE` (an image with bash
//! and git, pinned) add the whole path: a real worker reached over TLS.

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use glave_ports::computer::{Command, Computers, Ending, Environment, Image, Limits, Sources};
use glave_ports::{Clock as _, SystemClock, block_on};
use http::Method;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

use super::super::backend::{Backend, Link, Spec};
use super::{Docker, DockerSettings};
use crate::tls::Identity;
use crate::token::Token;
use crate::{CancelSwitch, WorkerComputers, WorkerCredentials};

/// alpine 3.21, by the digest of its multi-platform index.
const IMAGE: &str =
    "alpine@sha256:ce64758a109eb420d874a118f87920e625e12d3634e03b4a5573fd9f6e5d3507";

/// The stub that runs where glave would: it says how it was started and what it holds, then
/// echoes its input back, like a server answering on its standard output.
const STUB: &[u8] = b"#!/bin/sh\necho \"args: $*\"\nls -l /opt/glave/tls/key.pem\ncat\n";
/// A stub that only waits, as a TLS server would, so its port stays published.
const WAITS: &[u8] = b"#!/bin/sh\nexec sleep 300\n";
/// A stub that waits for its input to end, as a worker's glave serving TLS does.
const UNTIL_INPUT_ENDS: &[u8] = b"#!/bin/sh\ncat >/dev/null\n";

/// The run's label value: CI's run id, or this process's.
fn run_id() -> String {
    std::env::var("GITHUB_RUN_ID").unwrap_or_else(|_| format!("local-{}", std::process::id()))
}

/// Docker labelled for one test, and that test's case id; `None` (and a note) unless asked for.
fn docker() -> Option<(Docker, String)> {
    if std::env::var("GLAVE_TEST_DOCKER").as_deref() != Ok("1") {
        eprintln!(
            "skipped: set GLAVE_TEST_DOCKER=1 with Docker running to test the Docker backend"
        );
        return None;
    }
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let settings = DockerSettings {
        socket: None,
        glave: std::env::var_os("GLAVE_TEST_WORKER_GLAVE")
            .map_or_else(Default::default, Into::into),
    };
    let case = crate::local::new_id().unwrap().simple().to_string();
    let labels = BTreeMap::from([
        ("glave.test".to_owned(), run_id()),
        ("glave.test.case".to_owned(), case.clone()),
    ]);
    Some((
        Docker::new(settings, home.as_deref()).with_labels(labels),
        case,
    ))
}

fn spec(network: bool) -> Spec {
    Spec {
        image: IMAGE.into(),
        env: BTreeMap::from([("GREETING".to_owned(), "hi".to_owned())]),
        cpu_millis: Some(1_000),
        memory_mib: Some(1_024),
        network,
        identity: Identity::mint().unwrap(),
        token: Token::mint(
            SystemTime::from(SystemClock.now()),
            Duration::from_secs(600),
        )
        .unwrap(),
        starter: glave_process::Glave::this(),
    }
}

async fn get(docker: &Docker, path: &str) -> Value {
    docker
        .engine
        .json(Method::GET, path, None)
        .await
        .unwrap()
        .json()
}

async fn has_image(docker: &Docker, image: &str) -> bool {
    let answer = docker
        .engine
        .json(Method::GET, &format!("/images/{image}/json"), None)
        .await
        .unwrap();
    answer.status.is_success()
}

/// What the test `case` left: containers, volumes and networks with its run's and its labels.
async fn left_behind(docker: &Docker, case: &str) -> Vec<String> {
    let labels = [
        format!("glave.test={}", run_id()),
        format!("glave.test.case={case}"),
    ];
    let filter = super::query(&json!({ "label": labels }).to_string());
    let mut left = Vec::new();
    for (kind, path, field) in [
        (
            "container",
            format!("/containers/json?all=1&filters={filter}"),
            "/Names/0",
        ),
        ("volume", format!("/volumes?filters={filter}"), "/Name"),
        ("network", format!("/networks?filters={filter}"), "/Name"),
    ] {
        let found = get(docker, &path).await;
        let items = found
            .get("Volumes")
            .unwrap_or(&found)
            .as_array()
            .cloned()
            .unwrap_or_default();
        left.extend(items.iter().map(|item| {
            format!(
                "{kind} {}",
                item.pointer(field).and_then(Value::as_str).unwrap_or("?")
            )
        }));
    }
    left
}

/// Removes the test image when this run pulled it.
async fn forget_image(docker: &Docker, had: bool) {
    if !had {
        let path = format!("/images/{}?force=1", super::query(IMAGE));
        let _ = docker.engine.json(Method::DELETE, &path, None).await;
        assert!(
            !has_image(docker, IMAGE).await,
            "the pulled image is removed"
        );
    }
}

#[test]
fn the_docker_backend_starts_limits_reaches_and_removes_workers() {
    let Some((docker, case)) = docker() else {
        return;
    };
    block_on!(async {
        // One test, run in order, so removing the image it pulled never races another.
        let had = has_image(&docker, IMAGE).await;
        let checked = async {
            limited(&docker, &case).await;
            streamed(&docker, &case).await;
            published(&docker, &case).await;
            ended(&docker, &case).await;
            reaped(&docker, &case).await;
        };
        checked.await;
        assert_eq!(left_behind(&docker, &case).await, Vec::<String>::new());
        forget_image(&docker, had).await;
    });
}

async fn limited(docker: &Docker, case: &str) {
    let name = format!(
        "glave-worker-test-{}",
        crate::local::new_id().unwrap().simple()
    );
    let link = docker.start_with(&name, &spec(false), STUB).await;
    let inspected = get(docker, &format!("/containers/{name}/json")).await;
    let removed = docker.remove(&name).await;
    let link = link.unwrap();
    removed.unwrap();
    let host = &inspected["HostConfig"];
    assert_eq!(host["NanoCpus"], json!(1_000_000_000));
    assert_eq!(host["Memory"], json!(1_i64 << 30));
    assert_eq!(host["NetworkMode"], json!("none"));
    assert_eq!(host["SecurityOpt"], json!(["no-new-privileges"]));
    let env = inspected["Config"]["Env"].to_string();
    assert!(env.contains("GREETING=hi"), "{env}");
    assert!(
        !env.contains("PRIVATE KEY"),
        "the key is a file, not a variable"
    );
    drop(link);
    assert_eq!(left_behind(docker, case).await, Vec::<String>::new());
}

async fn streamed(docker: &Docker, case: &str) {
    let name = format!(
        "glave-worker-test-{}",
        crate::local::new_id().unwrap().simple()
    );
    let link = docker.start_with(&name, &spec(false), STUB).await.unwrap();
    let Link::Stream {
        read,
        mut write,
        task,
    } = link
    else {
        panic!("a worker without a network is reached by stream");
    };
    let mut lines = BufReader::new(read).lines();
    let args = lines.next_line().await.unwrap().unwrap();
    assert_eq!(args, "args: computer serve --stdio");
    let key = lines.next_line().await.unwrap().unwrap();
    assert!(key.starts_with("-rw-------"), "{key}");
    write.write_all(b"hello\n").await.unwrap();
    write.flush().await.unwrap();
    assert_eq!(lines.next_line().await.unwrap().unwrap(), "hello");
    task.abort();
    docker.remove(&name).await.unwrap();
    assert_eq!(left_behind(docker, case).await, Vec::<String>::new());
}

async fn published(docker: &Docker, case: &str) {
    let name = format!(
        "glave-worker-test-{}",
        crate::local::new_id().unwrap().simple()
    );
    let wanted = spec(true);
    let link = docker.start_with(&name, &wanted, WAITS).await;
    let inspected = get(docker, &format!("/containers/{name}/json")).await;
    docker.remove(&name).await.unwrap();
    let Link::Tls {
        address,
        fingerprint,
        forward,
    } = link.unwrap()
    else {
        panic!("a worker with a network is reached over TLS");
    };
    if let Some(holding) = forward {
        holding.abort();
    }
    assert!(address.ip().is_loopback(), "{address}");
    assert_eq!(fingerprint, wanted.identity.fingerprint);
    let bindings = &inspected["HostConfig"]["PortBindings"]["7070/tcp"][0];
    assert_eq!(bindings["HostIp"], json!("127.0.0.1"));
    assert_eq!(inspected["HostConfig"]["NetworkMode"], json!("bridge"));
    assert_eq!(left_behind(docker, case).await, Vec::<String>::new());
}

/// A worker with a network stops at once when the glave holding its input lets go of it, as one
/// killed outright does: the system closes its connection to Docker, and Docker the worker's
/// input.
async fn ended(docker: &Docker, case: &str) {
    let name = format!(
        "glave-worker-test-{}",
        crate::local::new_id().unwrap().simple()
    );
    let link = docker
        .start_with(&name, &spec(true), UNTIL_INPUT_ENDS)
        .await
        .unwrap();
    let Link::Tls {
        forward: Some(holding),
        ..
    } = link
    else {
        panic!("a worker with a network has its input held");
    };
    let path = format!("/containers/{name}/json");
    assert_eq!(get(docker, &path).await["State"]["Running"], json!(true));
    holding.abort();
    let _ = holding.await;
    let mut running = true;
    // A guard against a hang: it stops in moments.
    for _ in 0..300 {
        running = get(docker, &path).await["State"]["Running"] == json!(true);
        if !running {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    docker.remove(&name).await.unwrap();
    assert!(!running, "the worker stops once its input is let go of");
    assert_eq!(left_behind(docker, case).await, Vec::<String>::new());
}

/// A glave starting removes the workers of a glave that is gone, and only those.
async fn reaped(docker: &Docker, case: &str) {
    let this = glave_process::Glave::this().unwrap();
    let ended = super::super::tests::changed(&this, "start", "a glave that ended");
    let elsewhere = super::super::tests::changed(&ended, "host", "another machine");
    let mut names = Vec::new();
    for starter in [Some(ended), Some(elsewhere), Some(this), None] {
        let name = format!(
            "glave-worker-test-{}",
            crate::local::new_id().unwrap().simple()
        );
        let wanted = Spec {
            starter,
            ..spec(true)
        };
        if let Link::Tls {
            forward: Some(holding),
            ..
        } = docker.start_with(&name, &wanted, WAITS).await.unwrap()
        {
            holding.abort();
        }
        names.push(name);
    }
    let workers =
        WorkerComputers::docker(docker.clone(), Limits::default(), WorkerCredentials::None);
    let reaped = workers.reap().await;
    let listed = docker.list().await.unwrap();
    for name in &names {
        docker.remove(name).await.unwrap();
    }
    assert_eq!(reaped.unwrap(), 1);
    let mut kept: Vec<String> = names.split_off(1);
    kept.sort();
    let mut listed: Vec<String> = listed.into_iter().map(|listed| listed.name).collect();
    listed.sort();
    assert_eq!(listed, kept, "only the worker of the glave that ended goes");
    assert_eq!(left_behind(docker, case).await, Vec::<String>::new());
}

#[test]
fn a_real_worker_runs_a_command_over_tls() {
    let Some((docker, case)) = docker() else {
        return;
    };
    let (Ok(_), Ok(image)) = (
        std::env::var("GLAVE_TEST_WORKER_GLAVE"),
        std::env::var("GLAVE_TEST_WORKER_IMAGE"),
    ) else {
        eprintln!(
            "skipped: set GLAVE_TEST_WORKER_GLAVE (a Linux glave) and GLAVE_TEST_WORKER_IMAGE \
             (an image with bash and git, pinned by digest) for the whole path"
        );
        return;
    };
    block_on!(async {
        let workers =
            WorkerComputers::docker(docker.clone(), Limits::default(), WorkerCredentials::None);
        let environment = Environment {
            image: Image::Oci(image),
            ..Environment::host()
        };
        let made = workers.create(&environment).await.unwrap();
        let sources = Sources::Folder {
            path: "/tmp".into(),
        };
        let opened = workers.open(made.id, &sources).await;
        let ran = match &opened {
            Ok(opened) => {
                let output = super::super::route::Tail::default();
                workers
                    .run(
                        opened.id,
                        &Command::new("uname -s"),
                        &output,
                        &CancelSwitch::default(),
                    )
                    .await
                    .map(|finished| (finished.ending, output.text(100)))
            }
            Err(error) => Err(error.clone()),
        };
        workers.destroy(made.id).await.unwrap();
        assert_eq!(
            ran.unwrap(),
            (Ending::Exited { code: 0 }, "Linux".to_owned())
        );
        assert_eq!(left_behind(&docker, &case).await, Vec::<String>::new());
    });
}
