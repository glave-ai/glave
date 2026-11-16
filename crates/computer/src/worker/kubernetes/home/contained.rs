//! A red team's attempts to get out of a person's computer, on a real cluster
//! (`GLAVE_TEST_KUBE=1`, CI's `kind` job, under runc and under gVisor): as an agent's command
//! would, outside glave's sandbox (as Full access runs one, so the Pod alone must hold) and in
//! it, it tries the Kubernetes API, the node's kubelet, a stand-in for glave's server (a Pod and
//! its Service), another person's computer, any other Pod, the cluster's DNS and cloud metadata;
//! looks for credentials and the node; and tries to become root (`invariants.md`, 32;
//! `computers.md`, "A person's computer on Kubernetes"). Every attempt must fail, and each has
//! its control: the same addresses answer a Pod with no network policy, and the internet
//! answers the computer itself. What is tried, and how gently, is in `contained/probe.py`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use glave_ports::block_on;
use glave_ports::computer::{
    Command, CommandId, ComputerId, Computers as _, Ending, Environment, Image, Limits, Sandbox,
    SandboxLevel, SandboxNetwork, Sources, WorkspaceId,
};
use k8s_openapi::api::core::v1::{Node, Pod, Service};
use k8s_openapi::api::discovery::v1::EndpointSlice;
use kube::Api;
use kube::api::{DeleteParams, ListParams, LogParams, PostParams};
use serde_json::{Value, json};

use super::super::Kubernetes;
use super::super::objects::{CONTAINER, PERSON_HOME};
use super::super::tests::{another_pod, cluster, key, left_behind};
use super::PersonComputer;
use super::tests::ran;
use crate::CancelSwitch;
use crate::worker::WorkerComputers;
use crate::worker::route::Tail;

/// What runs each attempt, on the computers and in the control.
const PROBE: &str = include_str!("contained/probe.py");

/// What an attempt should come to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Comes {
    /// Refused, or a name that never resolves.
    Refused,
    /// Exactly this.
    Is(&'static str),
    /// Anything that starts with this.
    Starts(&'static str),
}

impl Comes {
    fn holds(self, said: Option<&str>) -> bool {
        match (self, said) {
            (Self::Refused, Some(said)) => said.starts_with("refused") || said == "unresolved",
            (Self::Is(wanted), Some(said)) => said == wanted,
            (Self::Starts(wanted), Some(said)) => said.starts_with(wanted),
            (_, None) => false,
        }
    }
}

/// Every address a computer tries, and what it must come to there: all of the cluster
/// refused, the internet reached.
const NETWORK: [(&str, Comes); 17] = [
    ("api_service", Comes::Refused),
    ("api_name", Comes::Refused),
    ("api_endpoint", Comes::Refused),
    ("kubelet", Comes::Refused),
    ("server_pod", Comes::Refused),
    ("server_service", Comes::Refused),
    ("server_name", Comes::Refused),
    ("another_computer", Comes::Refused),
    ("another_pod", Comes::Refused),
    ("cluster_dns", Comes::Refused),
    ("dns_pod", Comes::Refused),
    ("internal_name", Comes::Refused),
    ("metadata", Comes::Refused),
    ("metadata_name", Comes::Refused),
    ("metadata_gke", Comes::Refused),
    ("metadata_v6", Comes::Refused),
    ("internet", Comes::Is("reached")),
];

/// What a command outside the sandbox finds of credentials, the node and root.
const FOUND: [(&str, Comes); 18] = [
    ("service_account", Comes::Is("absent")),
    ("cloud_configs", Comes::Is("none")),
    ("secret_variables", Comes::Is("none")),
    ("server_files", Comes::Is("none")),
    ("runtime_sockets", Comes::Is("none")),
    ("devices", Comes::Is("none")),
    ("mounts", Comes::Is("pod-only")),
    ("pid1", Comes::Is("pause")),
    ("host_processes", Comes::Is("none")),
    ("pid1_root", Comes::Refused),
    ("proc_sys", Comes::Refused),
    ("cgroup", Comes::Refused),
    ("ids", Comes::Is("65532/65532/65532/65532")),
    ("capabilities", Comes::Is("none")),
    ("no_new_privs", Comes::Is("1")),
    ("setuid_root", Comes::Refused),
    ("mount", Comes::Refused),
    ("ptrace", Comes::Refused),
];

/// What the control, a Pod with no network policy, reaches of the same addresses: each one
/// answers, so a computer's refusal is its policy's. Another person's computer refuses it too,
/// by that computer's own policy, which lets nothing in.
const CONTROL: [(&str, Comes); 9] = [
    ("api_service", Comes::Is("reached")),
    ("api_name", Comes::Is("reached")),
    ("api_endpoint", Comes::Is("reached")),
    ("kubelet", Comes::Is("reached")),
    ("another_pod", Comes::Is("reached")),
    ("cluster_dns", Comes::Is("answered")),
    ("dns_pod", Comes::Is("reached")),
    ("internal_name", Comes::Is("resolved")),
    ("another_computer", Comes::Refused),
];

/// `PROBE` run by `python3` as a command, in `mode`, against `targets`.
fn probe(mode: &str, targets: &Value) -> String {
    format!(
        "python3 -c '{}' {mode} '{targets}'",
        PROBE.replace('\'', "'\\''")
    )
}

/// `text`'s `check=result` lines, by check.
fn results(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| line.split_once('='))
        .map(|(check, result)| (check.to_owned(), result.to_owned()))
        .collect()
}

/// Each of `wanted` that `said` doesn't hold to, with what was said, under `label`.
fn wrong(label: &str, said: &BTreeMap<String, String>, wanted: &[(&str, Comes)]) -> Vec<String> {
    wanted
        .iter()
        .filter(|(check, comes)| !comes.holds(said.get(*check).map(String::as_str)))
        .map(|(check, comes)| format!("{label} {check}: {:?}, not {comes:?}", said.get(*check)))
        .collect()
}

/// The addresses inside the cluster the test knows of, as its own kubeconfig reads them.
async fn addresses(cluster: &Kubernetes) -> Result<Value, String> {
    let text = |error: kube::Error| error.to_string();
    let services: Api<Service> = Api::namespaced(cluster.client.clone(), "default");
    let api = services.get("kubernetes").await.map_err(text)?;
    let api_service = api.spec.and_then(|spec| spec.cluster_ip);
    let slices: Api<EndpointSlice> = Api::namespaced(cluster.client.clone(), "default");
    let named = ListParams::default().labels("kubernetes.io/service-name=kubernetes");
    let slice = slices
        .list(&named)
        .await
        .map_err(text)?
        .items
        .into_iter()
        .next();
    let endpoint = slice.as_ref().and_then(|slice| {
        let address = slice.endpoints.first()?.addresses.first()?.clone();
        let port = slice.ports.as_ref()?.first()?.port?;
        Some((address, port))
    });
    let nodes: Api<Node> = Api::all(cluster.client.clone());
    let node = nodes.list(&ListParams::default()).await.map_err(text)?;
    let node = node.items.iter().find_map(|node| {
        let addresses = node.status.as_ref()?.addresses.as_ref()?;
        let internal = addresses.iter().find(|a| a.type_ == "InternalIP")?;
        Some(internal.address.clone())
    });
    let dns: Api<Service> = Api::namespaced(cluster.client.clone(), "kube-system");
    let dns = dns.get("kube-dns").await.map_err(text)?;
    let cluster_dns = dns.spec.and_then(|spec| spec.cluster_ip);
    match (
        api_service,
        endpoint,
        node,
        cluster_dns,
        another_pod(cluster).await,
    ) {
        (Some(api), Some((at, port)), Some(node), Some(dns), Some(pod)) => Ok(json!({
            "api_service": api, "api_endpoint": at, "api_endpoint_port": port, "node": node,
            "cluster_dns": dns, "another_pod": pod,
        })),
        missing => Err(format!(
            "an address inside the cluster could not be read: {missing:?}"
        )),
    }
}

/// The address of the Pod `name`, once it has one.
async fn address_of(cluster: &Kubernetes, name: &str) -> Result<String, String> {
    for _ in 0..240 {
        let pod = cluster
            .pods()
            .get_opt(name)
            .await
            .map_err(|e| e.to_string())?;
        if let Some(ip) = pod.and_then(|pod| pod.status?.pod_ip) {
            return Ok(ip);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err(format!("{name} had no address within two minutes"))
}

/// Everything the Pod `name` printed so far.
async fn printed(cluster: &Kubernetes, name: &str) -> String {
    let params = LogParams {
        container: Some(CONTAINER.into()),
        ..LogParams::default()
    };
    cluster.pods().logs(name, &params).await.unwrap_or_default()
}

/// The stand-in for glave's server, named `name`: a Pod with no network policy, listening on
/// glave's port behind a Service of its own, which first tries `targets` as the control does.
/// The Service's address once it has one.
async fn stand_in(
    cluster: &Kubernetes,
    name: &str,
    image: &str,
    targets: &Value,
) -> Result<String, String> {
    let text = |error: kube::Error| error.to_string();
    let mut labels = cluster.labels.clone();
    labels.insert("glave.server".to_owned(), name.to_owned());
    let pod = json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": {"name": name, "labels": labels},
        "spec": {
            "restartPolicy": "Never",
            "automountServiceAccountToken": false,
            "terminationGracePeriodSeconds": 1,
            "containers": [{
                "name": CONTAINER,
                "image": image,
                "command": ["python3", "-c", PROBE, "control", targets.to_string()],
                "securityContext": {
                    "allowPrivilegeEscalation": false,
                    "capabilities": {"drop": ["ALL"]},
                    "runAsNonRoot": true,
                    "seccompProfile": {"type": "RuntimeDefault"}
                },
            }],
        }
    });
    let pod: Pod = serde_json::from_value(pod).map_err(|e| e.to_string())?;
    cluster
        .pods()
        .create(&PostParams::default(), &pod)
        .await
        .map_err(text)?;
    let service = json!({
        "apiVersion": "v1",
        "kind": "Service",
        "metadata": {"name": name, "labels": cluster.labels},
        "spec": {"selector": {"glave.server": name}, "ports": [{"port": 4180, "targetPort": 4180}]}
    });
    let service: Service = serde_json::from_value(service).map_err(|e| e.to_string())?;
    let services: Api<Service> = Api::namespaced(cluster.client.clone(), &cluster.namespace);
    let made = services
        .create(&PostParams::default(), &service)
        .await
        .map_err(text)?;
    made.spec
        .and_then(|spec| spec.cluster_ip)
        .ok_or_else(|| format!("the Service {name} has no address"))
}

/// Waits, up to three minutes, until the stand-in `name` has tried every address.
async fn probed(cluster: &Kubernetes, name: &str) -> Result<String, String> {
    for _ in 0..360 {
        let said = printed(cluster, name).await;
        if said.contains("probed") {
            return Ok(said);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err(format!(
        "the stand-in {name} never finished its control: {}",
        printed(cluster, name).await
    ))
}

/// Another person's computer, listening on 8080 for anyone who gets in: the command, once it
/// says it listens.
async fn listening(
    computers: &WorkerComputers,
    computer: ComputerId,
) -> Result<(WorkspaceId, CommandId), String> {
    let home = Sources::Folder {
        path: PERSON_HOME.into(),
    };
    let opened = computers
        .open(computer, &home)
        .await
        .map_err(|e| format!("{e:?}"))?;
    let mut listen = Command::new(probe("listen", &json!({})));
    listen.timeout_secs = Some(300);
    let started = computers
        .start(opened.id, &listen)
        .await
        .map_err(|e| format!("{e:?}"))?;
    let mut heard = String::new();
    for _ in 0..60 {
        let wait = Duration::from_millis(500);
        let printed = computers
            .output(opened.id, started, wait)
            .await
            .map_err(|e| format!("{e:?}"))?;
        heard.push_str(&printed.text);
        if heard.contains("listening") {
            return Ok((opened.id, started));
        }
        if printed.ended.is_some() {
            break;
        }
    }
    Err(format!("another person's computer never listened: {heard}"))
}

/// What `script` printed in `computer`'s project `~/p` in glave's sandbox, with the network on:
/// as Auto runs a command there.
async fn sandboxed(
    computers: &WorkerComputers,
    computer: ComputerId,
    script: &str,
) -> Result<String, String> {
    ran(computers, computer, "mkdir -p p").await?;
    let project = Sources::Folder {
        path: format!("{PERSON_HOME}/p"),
    };
    let opened = computers
        .open(computer, &project)
        .await
        .map_err(|e| format!("{e:?}"))?;
    let mut command = Command::new(script);
    command.sandbox = Some(Sandbox {
        level: SandboxLevel::Project,
        network: SandboxNetwork::Direct,
        home: Some(PathBuf::from(PERSON_HOME)),
        glave_home: Some(PathBuf::from(format!("{PERSON_HOME}/.glave"))),
    });
    let output = Tail::default();
    let finished = computers
        .run(opened.id, &command, &output, &CancelSwitch::default())
        .await
        .map_err(|e| format!("{e:?}"))?;
    match finished.ending {
        Ending::Exited { code: 0 } => Ok(output.text(2_000)),
        ending => Err(format!("{ending:?} {}", output.text(1_000))),
    }
}

/// What the attempts came to, before anything is asserted.
struct Tried {
    control: String,
    outside: String,
    inside: Option<String>,
    another_heard: String,
    server_heard: String,
}

/// Wakes two people's computers, A and B, and a stand-in for glave's server; B listens, the
/// stand-in tries every address as the control, then A tries them all, outside glave's sandbox
/// and (under gVisor, where it starts) in it.
async fn attempt(
    cluster: &Kubernetes,
    (a, b, b_key): (&WorkerComputers, &WorkerComputers, &str),
    (environment, server, sandbox): (&Environment, &str, bool),
) -> Result<Tried, String> {
    let mut targets = addresses(cluster).await?;
    let (woke_a, woke_b) = tokio::join!(a.create(environment), b.create(environment));
    let woke_a = woke_a.map_err(|e| format!("wake: {e:?}"))?;
    let woke_b = woke_b.map_err(|e| format!("wake: {e:?}"))?;
    let (listened_in, listener) = listening(b, woke_b.id).await?;
    let another = address_of(cluster, &format!("glave-computer-{b_key}")).await?;
    targets["another_computer"] = json!(another);
    let Image::Oci(image) = &environment.image else {
        return Err("the computers' image is not an OCI image".into());
    };
    targets["server_service"] = json!(stand_in(cluster, server, image, &targets).await?);
    let control = probed(cluster, server).await?;
    targets["server"] = json!(address_of(cluster, server).await?);
    targets["server_name"] = json!(format!("{server}.{}.svc.cluster.local", cluster.namespace));
    let outside = ran(a, woke_a.id, &probe("all", &targets)).await?;
    let inside = if sandbox {
        Some(sandboxed(a, woke_a.id, &probe("net", &targets)).await?)
    } else {
        None
    };
    let another_heard = b
        .output(listened_in, listener, Duration::ZERO)
        .await
        .map_or_else(|e| format!("{e:?}"), |printed| printed.text);
    let server_heard = printed(cluster, server).await;
    let _ = tokio::join!(a.destroy(woke_a.id), b.destroy(woke_b.id));
    Ok(Tried {
        control,
        outside,
        inside,
        another_heard,
        server_heard,
    })
}

#[test]
fn nothing_gets_out_of_a_persons_computer() {
    let (Ok(_), Ok(image)) = (
        std::env::var("GLAVE_TEST_WORKER_GLAVE"),
        std::env::var("GLAVE_TEST_COMPUTER_IMAGE"),
    ) else {
        eprintln!(
            "skipped: set GLAVE_TEST_WORKER_GLAVE (a Linux glave) and GLAVE_TEST_COMPUTER_IMAGE \
             (glave's computer image, pinned by digest) to try to get out of a person's computer"
        );
        return;
    };
    block_on!(async {
        let class = std::env::var("GLAVE_TEST_KUBE_RUNTIME_CLASS").ok();
        let Some(cluster) = cluster(class.as_deref(), "contained").await else {
            return;
        };
        let gvisor = class
            .as_deref()
            .is_some_and(|class| class.contains("gvisor"));
        let (a, b, server) = (key("contained"), key("contained"), key("server"));
        let limits = Limits {
            isolation: cluster.isolation(),
            ..Limits::default()
        };
        let computers = |key: &str| {
            let person = PersonComputer::new(cluster.clone(), key, 1).unwrap();
            WorkerComputers::new(person, limits.clone())
        };
        let (computers_a, computers_b) = (computers(&a), computers(&b));
        let environment = Environment {
            image: Image::Oci(image),
            ..Environment::host()
        };
        let server = format!("glave-test-{server}");
        let tried = attempt(
            &cluster,
            (&computers_a, &computers_b, &b),
            (&environment, &server, gvisor),
        )
        .await;
        // Whatever happened, everything made goes before anything is asserted.
        let destroyed = [
            PersonComputer::new(cluster.clone(), &a, 1)
                .unwrap()
                .destroy()
                .await,
            PersonComputer::new(cluster.clone(), &b, 1)
                .unwrap()
                .destroy()
                .await,
        ];
        let services: Api<Service> = Api::namespaced(cluster.client.clone(), &cluster.namespace);
        let _ = services.delete(&server, &DeleteParams::default()).await;
        let _ = cluster
            .pods()
            .delete(&server, &DeleteParams::default())
            .await;
        let service_left = services.get_opt(&server).await.ok().flatten();
        let left = left_behind(&cluster).await;
        let tried = tried.unwrap();
        judged(&tried, class.as_deref(), gvisor);
        for destroyed in destroyed {
            destroyed.unwrap();
        }
        assert!(service_left.is_none(), "the stand-in's Service is removed");
        assert_eq!(left, Vec::<String>::new());
    });
}

/// Asserts every attempt in `tried` failed and every control held, printing them all first.
fn judged(tried: &Tried, class: Option<&str>, gvisor: bool) {
    let (control, outside) = (results(&tried.control), results(&tried.outside));
    let inside = tried.inside.as_deref().map(results);
    eprintln!(
        "under {class:?}\ncontrol (a Pod with no policy):\n{}\n\na person's computer, outside \
         glave's sandbox:\n{}\n\nin glave's sandbox:\n{:?}\n\nanother person's computer \
         heard:\n{}\n\nthe stand-in for glave's server heard:\n{}",
        tried.control, tried.outside, tried.inside, tried.another_heard, tried.server_heard
    );
    let kernel = if gvisor {
        Comes::Is("gvisor")
    } else {
        Comes::Starts("linux ")
    };
    let mut wrong_all = wrong("control", &control, &CONTROL);
    wrong_all.extend(wrong("outside", &outside, &NETWORK));
    wrong_all.extend(wrong("outside", &outside, &FOUND));
    wrong_all.extend(wrong("outside", &outside, &[("kernel", kernel)]));
    if let Some(inside) = &inside {
        wrong_all.extend(wrong("sandboxed", inside, &NETWORK));
    }
    if tried.another_heard.contains("accepted") {
        wrong_all.push(format!(
            "another person's computer was reached: {}",
            tried.another_heard
        ));
    }
    if tried.server_heard.contains("accepted") {
        wrong_all.push(format!(
            "the server's stand-in was reached: {}",
            tried.server_heard
        ));
    }
    assert_eq!(
        gvisor,
        inside.is_some(),
        "glave's sandbox is tried where it starts"
    );
    assert_eq!(wrong_all, Vec::<String>::new(), "nothing gets out");
}
