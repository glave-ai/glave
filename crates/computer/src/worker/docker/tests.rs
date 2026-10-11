//! The Docker backend against a real Docker Engine, only with `GLAVE_TEST_DOCKER=1` (CI's Linux
//! runner sets it; elsewhere these tests say they skipped). A small image pinned by digest
//! runs a stub in glave's place, so the lifecycle is tested without a Linux build of glave:
//! the limits, the files put in before start, no network or a port on 127.0.0.1 only, the
//! stream to a worker without a network, and removal. Everything made carries the label
//! `glave.test=<run>`, and each test checks by that label that nothing is left behind.
//! `GLAVE_TEST_WORKER_GLAVE` (a Linux glave) and `GLAVE_TEST_WORKER_IMAGE` (an image with bash
//! and git, pinned) add the whole path: a real worker reached over TLS.

use std::collections::BTreeMap;
use std::time::Duration;

use glave_ports::{Command, Computers, Ending, Environment, Image, Sources};
use http::Method;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

use super::super::backend::{Backend, Link, Spec};
use super::{Docker, DockerSettings};
use crate::tls::Identity;
use crate::token::Token;
use crate::{Credentials, Limits, Switch, WorkerComputers};

/// alpine 3.21, by the digest of its multi-platform index.
const IMAGE: &str =
    "alpine@sha256:ce64758a109eb420d874a118f87920e625e12d3634e03b4a5573fd9f6e5d3507";

/// The stub that runs where glave would: it says how it was started and what it holds, then
/// echoes its input back, like a server answering on its standard output.
const STUB: &[u8] = b"#!/bin/sh\necho \"args: $*\"\nls -l /opt/glave/tls/key.pem\ncat\n";
/// A stub that only waits, as a TLS server would, so its port stays published.
const WAITS: &[u8] = b"#!/bin/sh\nexec sleep 300\n";

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

/// The run's label value: CI's run id, or this process's.
fn run_id() -> String {
    std::env::var("GITHUB_RUN_ID").unwrap_or_else(|_| format!("local-{}", std::process::id()))
}

/// Docker, when the test is asked for; `None` (and a note) otherwise.
fn docker() -> Option<Docker> {
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
    let labels = BTreeMap::from([("glave.test".to_owned(), run_id())]);
    Some(Docker::new(settings, home.as_deref()).with_labels(labels))
}

fn spec(network: bool) -> Spec {
    Spec {
        image: IMAGE.into(),
        env: BTreeMap::from([("GREETING".to_owned(), "hi".to_owned())]),
        cpus: Some(1),
        memory_gb: Some(1),
        network,
        identity: Identity::mint().unwrap(),
        token: Token::mint(std::time::SystemTime::now(), Duration::from_secs(600)).unwrap(),
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

/// What this run left: containers, volumes and networks with its label.
async fn left_behind(docker: &Docker) -> Vec<String> {
    let filter = super::query(&json!({"label": [format!("glave.test={}", run_id())]}).to_string());
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
    let Some(docker) = docker() else { return };
    block_on(async {
        // One test, run in order, so removing the image it pulled never races another.
        let had = has_image(&docker, IMAGE).await;
        let checked = async {
            limited(&docker).await;
            streamed(&docker).await;
            published(&docker).await;
        };
        checked.await;
        assert_eq!(left_behind(&docker).await, Vec::<String>::new());
        forget_image(&docker, had).await;
    });
}

async fn limited(docker: &Docker) {
    let name = format!("glave-worker-test-{}", crate::local::new_id().simple());
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
    assert_eq!(left_behind(docker).await, Vec::<String>::new());
}

async fn streamed(docker: &Docker) {
    let name = format!("glave-worker-test-{}", crate::local::new_id().simple());
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
    assert_eq!(left_behind(docker).await, Vec::<String>::new());
}

async fn published(docker: &Docker) {
    let name = format!("glave-worker-test-{}", crate::local::new_id().simple());
    let wanted = spec(true);
    let link = docker.start_with(&name, &wanted, WAITS).await;
    let inspected = get(docker, &format!("/containers/{name}/json")).await;
    docker.remove(&name).await.unwrap();
    let Link::Tls {
        address,
        fingerprint,
        ..
    } = link.unwrap()
    else {
        panic!("a worker with a network is reached over TLS");
    };
    assert!(address.ip().is_loopback(), "{address}");
    assert_eq!(fingerprint, wanted.identity.fingerprint);
    let bindings = &inspected["HostConfig"]["PortBindings"]["7070/tcp"][0];
    assert_eq!(bindings["HostIp"], json!("127.0.0.1"));
    assert_eq!(inspected["HostConfig"]["NetworkMode"], json!("bridge"));
    assert_eq!(left_behind(docker).await, Vec::<String>::new());
}

#[test]
fn a_real_worker_runs_a_command_over_tls() {
    let Some(docker) = docker() else { return };
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
    block_on(async {
        let workers = WorkerComputers::docker(docker.clone(), Limits::default(), Credentials::None);
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
                        &Switch::default(),
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
        assert_eq!(left_behind(&docker).await, Vec::<String>::new());
    });
}
