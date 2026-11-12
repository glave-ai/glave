//! People's computers on a real cluster, only with `GLAVE_TEST_KUBE=1` (CI's `kind` job sets
//! it; elsewhere these tests say they skipped), on the kubeconfig's cluster and namespace. A
//! stub in glave's place, on glave's computer image (`GLAVE_TEST_COMPUTER_IMAGE`), tests the
//! Pod without a Linux build of glave: its limits and hardening as the API server keeps them,
//! the network policy, the files copied in, the bridges into it (an exec each), and sleep. A
//! runtime class the cluster lacks, or an isolation policy asks for that the cluster can't
//! give, leaves nothing behind. Everything made carries `glave.test=<run>`, and each test
//! checks by that label that nothing is left behind, Secrets included (there are none to
//! leave). The whole path, with a real glave, is in `home/tests.rs` and `people/tests.rs`.

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use glave_ports::computer::{ComputerError, Computers as _, Environment, Image, Isolation, Limits};
use glave_ports::{Clock as _, SystemClock, block_on};
use k8s_openapi::api::core::v1::{PersistentVolumeClaim, Pod, Secret};
use k8s_openapi::api::networking::v1::NetworkPolicy;
use kube::Api;
use kube::api::ListParams;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

use super::{Kubernetes, KubernetesSettings, PersonComputer, Spec};
use crate::token::Token;
use crate::worker::WorkerComputers;

/// alpine 3.21, by the digest of its multi-platform index: never pulled, as the Pod naming it
/// is refused or never made.
const IMAGE: &str =
    "alpine@sha256:ce64758a109eb420d874a118f87920e625e12d3634e03b4a5573fd9f6e5d3507";

/// The stub that runs where glave would: serving, it says how it was started and what it
/// holds, then waits, as a server on a socket would; as a bridge, it says how it was started,
/// then echoes its input back, like a server answering through it.
const STUB: &[u8] = b"#!/bin/sh\necho \"args: $*\"\ncase \"$2\" in bridge) exec cat ;; esac\n\
ls -l /opt/glave/secret/token\nexec sleep 300\n";

/// The run's label value: CI's run id, or this process's.
fn run_id() -> String {
    std::env::var("GITHUB_RUN_ID").unwrap_or_else(|_| format!("local-{}", std::process::id()))
}

/// The cluster, when the test is asked for; `None` (and a note) otherwise. What it makes is
/// labelled with the run and `case`, so tests running at once each find only their own.
pub(super) async fn cluster(runtime_class: Option<&str>, case: &str) -> Option<Kubernetes> {
    if std::env::var("GLAVE_TEST_KUBE").as_deref() != Ok("1") {
        eprintln!(
            "skipped: set GLAVE_TEST_KUBE=1 with a cluster in the kubeconfig to test people's \
             computers on Kubernetes"
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

/// `cluster` with `glave` as the glave it puts in computers, written to `folder`.
pub(super) fn with_glave(
    cluster: &Kubernetes,
    folder: &glave_ports::testing::TempDir,
    glave: &[u8],
) -> Kubernetes {
    let path = folder.path().join("glave");
    std::fs::write(&path, glave).unwrap();
    Kubernetes {
        glave: path,
        ..cluster.clone()
    }
}

/// A new key for a person's computer of the test `case`.
pub(super) fn key(case: &str) -> String {
    let id = crate::local::new_id().unwrap().simple().to_string();
    format!("{case}-{}", id.get(id.len() - 12..).unwrap())
}

fn spec(image: &str) -> Spec {
    Spec {
        image: image.into(),
        env: BTreeMap::from([("GREETING".to_owned(), "hi".to_owned())]),
        cpu_millis: None,
        memory_mib: None,
        network: true,
        token: Token::mint(
            SystemTime::from(SystemClock.now()),
            Duration::from_secs(600),
        )
        .unwrap(),
    }
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

/// What this test left: Pods, network policies, disks and Secrets with its label, waited for (a
/// removed Pod takes a moment to go).
pub(super) async fn left_behind(cluster: &Kubernetes) -> Vec<String> {
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
        let claims: Api<PersistentVolumeClaim> =
            Api::namespaced(cluster.client.clone(), &cluster.namespace);
        let claims = claims.list(&label).await.unwrap();
        left.extend(
            claims
                .items
                .iter()
                .map(|c| format!("disk {:?}", c.metadata.name)),
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

/// The object `name` as the API server keeps it, or why it could not be read.
async fn object<K: serde::Serialize + Clone + serde::de::DeserializeOwned + std::fmt::Debug>(
    api: &Api<K>,
    name: &str,
) -> Result<Value, String> {
    let kept = api.get(name).await.map_err(|error| format!("{error}"))?;
    serde_json::to_value(kept).map_err(|error| format!("{error}"))
}

/// What the computer printed, once it printed `lines` lines.
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

/// What one bridge into `computer` answers to `hello`: the stub says how it was started, then
/// echoes.
async fn through(computer: &PersonComputer) -> Result<Vec<String>, String> {
    let (read, mut write) = computer.bridge().await.map_err(|e| format!("{e:?}"))?;
    let mut lines = BufReader::new(read).lines();
    write
        .write_all(b"hello\n")
        .await
        .map_err(|e| e.to_string())?;
    write.flush().await.map_err(|e| e.to_string())?;
    let mut said = Vec::new();
    for _ in 0..2 {
        let line = lines.next_line().await.map_err(|e| e.to_string())?;
        said.push(line.unwrap_or_default());
    }
    Ok(said)
}

/// What a person's computer's Pod, its policy and the stub in it showed, while awake.
struct Seen {
    pod: Value,
    policy: Value,
    said: String,
    bridges: (Vec<String>, Vec<String>),
}

/// Looks at the awake computer `name`, as `computer` reaches it.
async fn seen(cluster: &Kubernetes, computer: &PersonComputer, name: &str) -> Result<Seen, String> {
    let pod = object::<Pod>(&cluster.pods(), name).await?;
    let policy = object::<NetworkPolicy>(&cluster.policies(), name).await?;
    let said = printed(cluster, name, 2).await;
    // Each connection is a bridge of its own, the computer going on between them.
    let bridges = (through(computer).await?, through(computer).await?);
    Ok(Seen {
        pod,
        policy,
        said,
        bridges,
    })
}

#[test]
fn a_persons_computer_is_limited_hardened_reached_and_put_to_sleep() {
    let Ok(image) = std::env::var("GLAVE_TEST_COMPUTER_IMAGE") else {
        eprintln!(
            "skipped: set GLAVE_TEST_COMPUTER_IMAGE (glave's computer image, pinned by digest) \
             to test a person's computer's Pod"
        );
        return;
    };
    block_on!(async {
        let class = std::env::var("GLAVE_TEST_KUBE_RUNTIME_CLASS").ok();
        let Some(cluster) = cluster(class.as_deref(), "lifecycle").await else {
            return;
        };
        let folder = glave_ports::testing::TempDir::new("stub").unwrap();
        let stubbed = with_glave(&cluster, &folder, STUB);
        let key = key("lifecycle");
        let name = format!("glave-computer-{key}");
        let computer = PersonComputer::new(stubbed, &key, 1).unwrap();
        let wanted = spec(&image);
        let woke = computer.wake(&wanted).await;
        let seen = match &woke {
            Ok(_) => seen(&cluster, &computer, &name).await,
            Err(error) => Err(format!("wake: {error:?}")),
        };
        let slept = computer.sleep().await;
        let after = cluster.pods().get_opt(&name).await.unwrap();
        let destroyed = computer.destroy().await;
        let left = left_behind(&cluster).await;
        woke.unwrap();
        let seen = seen.unwrap();
        slept.unwrap();
        assert!(after.is_none(), "asleep, its Pod is gone");
        destroyed.unwrap();
        assert_eq!(left, Vec::<String>::new());
        limited_and_hardened(&seen, &wanted, class.as_deref());
        served_and_reached(&seen);
    });
}

/// `seen`'s Pod and policy, as the API server kept them for a computer woken from `wanted` under
/// `class`: limited, hardened, the token nowhere in them, nothing let in.
fn limited_and_hardened(seen: &Seen, wanted: &Spec, class: Option<&str>) {
    let (pod, policy) = (&seen.pod, &seen.policy);
    let container = &pod["spec"]["containers"][0];
    let amounts = json!({"cpu": "250m", "memory": "512Mi", "ephemeral-storage": "4Gi"});
    assert_eq!(
        container["resources"]["limits"], amounts,
        "it wakes smallest"
    );
    assert_eq!(container["resources"]["requests"], amounts);
    assert_eq!(pod["spec"]["automountServiceAccountToken"], json!(false));
    let security = &container["securityContext"];
    assert_eq!(security["capabilities"]["drop"], json!(["ALL"]));
    assert_eq!(security["allowPrivilegeEscalation"], json!(false));
    assert_eq!(security["runAsUser"], json!(65532));
    assert_eq!(
        pod["spec"]
            .get("runtimeClassName")
            .cloned()
            .unwrap_or(Value::Null),
        json!(class),
        "the runtime class set, if any"
    );
    assert!(
        !pod.to_string().contains(wanted.token.secret()),
        "the token never reaches the API server"
    );
    assert!(container.get("ports").is_none(), "nothing listens: {pod}");
    assert!(container["env"].to_string().contains("GREETING"));
    assert_eq!(policy["spec"]["policyTypes"], json!(["Ingress", "Egress"]));
    assert!(policy["spec"]["egress"].to_string().contains("0.0.0.0/0"));
    let none = |rules: &Value| rules.is_null() || *rules == json!([]);
    assert!(
        none(&policy["spec"]["ingress"]),
        "nothing comes in: {policy}"
    );
}

/// What the stub in `seen`'s computer said: glave started serving as a person's computer
/// serves, the token its own, and each bridge a connection of its own.
fn served_and_reached(seen: &Seen) {
    let said = &seen.said;
    assert!(
        said.contains(
            "args: computer serve --socket /opt/glave/glave.sock /opt/glave/secret --idle \
             120 --keep-disk"
        ),
        "{said}"
    );
    assert!(
        said.contains("-rw-------"),
        "the token is the computer's alone: {said}"
    );
    let echoed = vec![
        "args: computer bridge /opt/glave/glave.sock".to_owned(),
        "hello".to_owned(),
    ];
    assert_eq!(seen.bridges, (echoed.clone(), echoed));
}

#[test]
fn a_runtime_class_the_cluster_lacks_or_isolation_it_cannot_give_leaves_nothing_behind() {
    block_on!(async {
        let Some(missing) = cluster(Some("glave-test-missing"), "missing").await else {
            return;
        };
        let folder = glave_ports::testing::TempDir::new("stub").unwrap();
        let key = key("missing");
        let computer = PersonComputer::new(with_glave(&missing, &folder, STUB), &key, 1).unwrap();
        assert_eq!(computer.isolation(), Isolation::Vm);
        let refused = computer.wake(&spec(IMAGE)).await;
        // Its disk is kept, as a person's is, until it is removed.
        let destroyed = computer.destroy().await;

        // A policy that asks for isolation the cluster can't give refuses the wake before
        // anything is made.
        let plain = cluster(None, "missing").await.unwrap();
        let computer = PersonComputer::new(with_glave(&plain, &folder, STUB), &key, 1).unwrap();
        let limits = Limits {
            isolation: Isolation::Vm,
            ..Limits::default()
        };
        let environment = Environment {
            image: Image::Oci(IMAGE.into()),
            ..Environment::host()
        };
        let needs = WorkerComputers::new(computer, limits)
            .create(&environment)
            .await;
        let left = left_behind(&missing).await;

        let message = refused.unwrap_err().message().to_owned();
        assert!(message.contains("glave-test-missing"), "{message}");
        destroyed.unwrap();
        assert!(
            matches!(&needs, Err(ComputerError::Refused(m)) if m.contains("isolation = \"vm\"")),
            "{needs:?}"
        );
        assert_eq!(left, Vec::<String>::new());
    });
}

/// The IP of a Pod that is not the test's and listens on TCP: a CoreDNS Pod, at its metrics
/// port, which no policy of the cluster's keeps anything from.
pub(super) async fn another_pod(cluster: &Kubernetes) -> Option<String> {
    let dns: Api<Pod> = Api::namespaced(cluster.client.clone(), "kube-system");
    let pods = dns
        .list(&ListParams::default().labels("k8s-app=kube-dns"))
        .await
        .ok()?;
    pods.items
        .iter()
        .find_map(|pod| pod.status.as_ref().and_then(|status| status.pod_ip.clone()))
}
