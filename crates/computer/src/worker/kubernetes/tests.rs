//! The Kubernetes backend against a real cluster, only with `GLAVE_TEST_KUBE=1` (CI's `kind`
//! job sets it; elsewhere these tests say they skipped), on the kubeconfig's cluster and
//! namespace. A small image pinned by digest runs a stub in glave's place, so the lifecycle is
//! tested without a Linux build of glave: the Pod's limits and hardening, the network policy,
//! the files copied in, the stream to a worker without a network, the runtime class enforced,
//! and removal. Everything made carries `glave.test=<run>`, and each test checks by that
//! label that nothing is left behind, Secrets included (there are none to leave).
//! `GLAVE_TEST_WORKER_GLAVE` (a Linux glave) and `GLAVE_TEST_WORKER_IMAGE` (an image with bash
//! and git, pinned) add the whole path: a real worker reached over TLS through port-forward,
//! and one without a network reached through attach.

use std::collections::BTreeMap;
use std::time::Duration;

use glave_ports::{Command, Computers, Ending, Environment, Image, Network, Sources, block_on};
use k8s_openapi::api::core::v1::{Pod, Secret};
use k8s_openapi::api::networking::v1::NetworkPolicy;
use kube::Api;
use kube::api::ListParams;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

use super::super::backend::{Backend, Link, Spec};
use super::{Kubernetes, KubernetesSettings};
use crate::tls::Identity;
use crate::token::Token;
use crate::{CancelSwitch, Limits, WorkerComputers, WorkerCredentials};

/// alpine 3.21, by the digest of its multi-platform index.
const IMAGE: &str =
    "alpine@sha256:ce64758a109eb420d874a118f87920e625e12d3634e03b4a5573fd9f6e5d3507";

/// The stub that runs where glave would, without a network: it says how it was started, then
/// echoes its input back, like a server answering on its standard output.
const ECHOES: &[u8] = b"#!/bin/sh\necho \"args: $*\"\ncat\n";
/// The stub for a worker with a network: it says how it was started and what it holds, then
/// waits, as a TLS server would.
const WAITS: &[u8] =
    b"#!/bin/sh\necho \"args: $*\"\nls -l /opt/glave/tls/key.pem\nexec sleep 300\n";

/// The run's label value: CI's run id, or this process's.
fn run_id() -> String {
    std::env::var("GITHUB_RUN_ID").unwrap_or_else(|_| format!("local-{}", std::process::id()))
}

/// The cluster, when the test is asked for; `None` (and a note) otherwise. What it makes is
/// labelled with the run and `case`, so tests running at once each find only their own.
async fn cluster(runtime_class: Option<&str>, case: &str) -> Option<Kubernetes> {
    if std::env::var("GLAVE_TEST_KUBE").as_deref() != Ok("1") {
        eprintln!(
            "skipped: set GLAVE_TEST_KUBE=1 with a cluster in the kubeconfig to test the Kubernetes backend"
        );
        return None;
    }
    let settings = KubernetesSettings {
        kubeconfig: None,
        namespace: std::env::var("GLAVE_TEST_KUBE_NAMESPACE").ok(),
        runtime_class: runtime_class.map(str::to_owned),
        glave: std::env::var_os("GLAVE_TEST_WORKER_GLAVE")
            .map_or_else(Default::default, Into::into),
    };
    let labels = BTreeMap::from([
        ("glave.test".to_owned(), run_id()),
        ("glave.case".to_owned(), case.to_owned()),
    ]);
    Some(
        Kubernetes::connect(settings)
            .await
            .unwrap()
            .with_labels(labels),
    )
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

fn name() -> String {
    format!("glave-worker-test-{}", crate::local::new_id().simple())
}

/// Everything this test made, by its labels.
fn mine(cluster: &Kubernetes) -> ListParams {
    let labels: Vec<String> = cluster
        .labels
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect();
    ListParams::default().labels(&labels.join(","))
}

/// What this test left: Pods, network policies and Secrets with its label, waited for (a
/// removed Pod takes a moment to go).
async fn left_behind(cluster: &Kubernetes) -> Vec<String> {
    let label = mine(cluster);
    let mut left = Vec::new();
    for _ in 0..120 {
        left.clear();
        let pods = cluster.pods().list(&label).await.unwrap();
        left.extend(
            pods.items
                .iter()
                .map(|p| format!("pod {:?}", p.metadata.name)),
        );
        let policies = cluster.policies().list(&label).await.unwrap();
        left.extend(
            policies
                .items
                .iter()
                .map(|p| format!("policy {:?}", p.metadata.name)),
        );
        let secrets: Api<Secret> = Api::namespaced(cluster.client.clone(), &cluster.namespace);
        let secrets = secrets.list(&label).await.unwrap();
        left.extend(
            secrets
                .items
                .iter()
                .map(|s| format!("secret {:?}", s.metadata.name)),
        );
        if left.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    left
}

async fn object<K: serde::Serialize + Clone + serde::de::DeserializeOwned + std::fmt::Debug>(
    api: &Api<K>,
    name: &str,
) -> Value {
    serde_json::to_value(api.get(name).await.unwrap()).unwrap()
}

/// What the worker printed, once it printed `lines` lines.
async fn printed(cluster: &Kubernetes, name: &str, lines: usize) -> String {
    let mut said = String::new();
    for _ in 0..60 {
        said = cluster.logs(name).await;
        if said.lines().count() >= lines {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    said
}

#[test]
fn the_kubernetes_backend_starts_limits_reaches_and_removes_workers() {
    block_on!(async {
        let Some(cluster) = cluster(None, "lifecycle").await else {
            return;
        };
        limited(&cluster).await;
        streamed(&cluster).await;
        assert_eq!(left_behind(&cluster).await, Vec::<String>::new());
    });
}

async fn limited(cluster: &Kubernetes) {
    let name = name();
    let wanted = spec(true);
    let link = cluster.start_with(&name, &wanted, WAITS).await;
    let pod: Value = object::<Pod>(&cluster.pods(), &name).await;
    let policy: Value = object::<NetworkPolicy>(&cluster.policies(), &name).await;
    let said = printed(cluster, &name, 2).await;
    cluster.remove(&name).await.unwrap();
    let Link::Tls {
        address,
        fingerprint,
        forward,
    } = link.unwrap()
    else {
        panic!("a worker with a network is reached over TLS");
    };
    if let Some(forward) = forward {
        forward.abort();
    }
    assert!(address.ip().is_loopback(), "{address}");
    assert_eq!(fingerprint, wanted.identity.fingerprint);
    let container = &pod["spec"]["containers"][0];
    let amounts = json!({"cpu": "1", "memory": "1Gi"});
    assert_eq!(container["resources"]["limits"], amounts);
    assert_eq!(container["resources"]["requests"], amounts);
    assert_eq!(pod["spec"]["automountServiceAccountToken"], json!(false));
    assert_eq!(
        container["securityContext"]["capabilities"]["drop"],
        json!(["ALL"])
    );
    assert_eq!(
        container["securityContext"]["allowPrivilegeEscalation"],
        json!(false)
    );
    assert!(
        pod["spec"].get("runtimeClassName").is_none(),
        "none configured"
    );
    assert!(
        !pod.to_string().contains("PRIVATE KEY"),
        "the key never reaches the API server"
    );
    assert!(container["env"].to_string().contains("GREETING"));
    assert_eq!(policy["spec"]["policyTypes"], json!(["Ingress", "Egress"]));
    assert!(policy["spec"]["egress"].to_string().contains("0.0.0.0/0"));
    assert!(
        said.contains("args: computer serve --tls 0.0.0.0:7070 /opt/glave/tls"),
        "{said}"
    );
    assert!(
        said.contains("-rw-------"),
        "the key is the worker's alone: {said}"
    );
    assert_eq!(left_behind(cluster).await, Vec::<String>::new());
}

async fn streamed(cluster: &Kubernetes) {
    let name = name();
    let link = cluster
        .start_with(&name, &spec(false), ECHOES)
        .await
        .unwrap();
    let policy: Value = object::<NetworkPolicy>(&cluster.policies(), &name).await;
    let Link::Stream {
        read,
        mut write,
        task,
    } = link
    else {
        panic!("a worker without a network is reached by stream");
    };
    let mut lines = BufReader::new(read).lines();
    write.write_all(b"hello\n").await.unwrap();
    write.flush().await.unwrap();
    let first = lines.next_line().await.unwrap().unwrap();
    // The stub may have printed its arguments before the attachment; the echo always comes.
    let echoed = if first == "hello" {
        first
    } else {
        assert_eq!(first, "args: computer serve --stdio");
        lines.next_line().await.unwrap().unwrap()
    };
    assert_eq!(echoed, "hello");
    task.abort();
    cluster.remove(&name).await.unwrap();
    // Both directions are governed, and the API server drops empty rule lists: no rules.
    assert_eq!(policy["spec"]["policyTypes"], json!(["Ingress", "Egress"]));
    let none = |rules: &Value| rules.is_null() || *rules == json!([]);
    assert!(
        none(&policy["spec"]["egress"]),
        "nothing goes out: {policy}"
    );
    assert!(
        none(&policy["spec"]["ingress"]),
        "nothing comes in: {policy}"
    );
    assert_eq!(left_behind(cluster).await, Vec::<String>::new());
}

#[test]
fn a_runtime_class_is_required_of_the_cluster_and_nothing_is_left_when_it_is_missing() {
    block_on!(async {
        let Some(cluster) = cluster(Some("glave-test-missing"), "missing").await else {
            return;
        };
        assert_eq!(cluster.isolation(), crate::Isolation::Vm);
        let refused = cluster.start(&spec(true)).await;
        let message = refused.unwrap_err().message().to_owned();
        assert!(message.contains("glave-test-missing"), "{message}");
        assert_eq!(left_behind(&cluster).await, Vec::<String>::new());
    });
}

/// A real worker, from `GLAVE_TEST_WORKER_GLAVE` and `GLAVE_TEST_WORKER_IMAGE`, with the
/// network or without, running `uname -s` and answering.
async fn real(cluster: &Kubernetes, image: &str, network: Network) {
    // With a runtime class, policy may require one, and the Pod runs under it.
    let limits = Limits {
        isolation: cluster.isolation(),
        ..Limits::default()
    };
    let workers = WorkerComputers::kubernetes(cluster.clone(), limits, WorkerCredentials::None);
    let environment = Environment {
        image: Image::Oci(image.to_owned()),
        network: Some(network),
        ..Environment::host()
    };
    let made = workers.create(&environment).await.unwrap();
    let label = mine(cluster);
    let pods = cluster.pods().list(&label).await.unwrap();
    let classes: Vec<Option<String>> = pods
        .items
        .iter()
        .map(|pod| {
            pod.spec
                .as_ref()
                .and_then(|spec| spec.runtime_class_name.clone())
        })
        .collect();
    assert_eq!(classes, vec![cluster.runtime_class.clone()]);
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
        (Ending::Exited { code: 0 }, "Linux".to_owned()),
        "{network:?}"
    );
}

#[test]
fn a_real_worker_runs_a_command_over_tls_and_over_attach() {
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
        let runtime_class = std::env::var("GLAVE_TEST_KUBE_RUNTIME_CLASS").ok();
        let Some(cluster) = cluster(runtime_class.as_deref(), "real").await else {
            return;
        };
        real(&cluster, &image, Network::On).await;
        real(&cluster, &image, Network::Off).await;
        assert_eq!(left_behind(&cluster).await, Vec::<String>::new());
    });
}
