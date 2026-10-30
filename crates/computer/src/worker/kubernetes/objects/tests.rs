//! The objects a worker or a person's computer is made of, as the API server is sent them.

use std::time::SystemTime;

use glave_ports::{Clock as _, SystemClock};

use super::*;
use crate::tls::Identity;
use crate::token::Token;

fn spec(network: bool) -> Spec {
    Spec {
        image: "alpine@sha256:ce64758a109eb420d874a118f87920e625e12d3634e03b4a5573fd9f6e5d3507"
            .into(),
        env: BTreeMap::from([("GREETING".to_owned(), "hi".to_owned())]),
        cpu_millis: Some(2_000),
        memory_mib: Some(4_096),
        network,
        identity: Identity::mint().unwrap(),
        token: Token::mint(
            SystemTime::from(SystemClock.now()),
            std::time::Duration::from_secs(60),
        )
        .unwrap(),
        starter: glave_process::Glave::this(),
    }
}

/// A person's computer, `k1`, from `spec`.
fn person<'a>(spec: &'a Spec, labels: &'a BTreeMap<String, String>) -> Shape<'a> {
    Shape {
        name: "glave-computer-k1",
        home: Some(Home {
            key: "k1",
            claim: "glave-home-k1",
            idle_minutes: IDLE_MINUTES,
        }),
        owner: None,
        ..shape(spec, labels)
    }
}

/// The Pod of a worker from `spec`.
fn shape_of(spec: &Spec, labels: &BTreeMap<String, String>) -> Value {
    shape(spec, labels).pod()
}

fn shape<'a>(spec: &'a Spec, labels: &'a BTreeMap<String, String>) -> Shape<'a> {
    Shape {
        name: "glave-worker-1",
        spec,
        runtime_class: Some("gvisor"),
        arch: Some("arm64"),
        labels,
        owner: None,
        home: None,
    }
}

#[test]
fn a_pod_is_limited_hardened_and_isolated() {
    let labels = BTreeMap::from([("glave.test".to_owned(), "run".to_owned())]);
    let spec = spec(true);
    let pod = shape(&spec, &labels).pod();
    let container = &pod["spec"]["containers"][0];
    let amounts = json!({"cpu": "2", "memory": "4Gi"});
    assert_eq!(container["resources"]["limits"], amounts);
    assert_eq!(container["resources"]["requests"], amounts);
    assert_eq!(pod["spec"]["runtimeClassName"], json!("gvisor"));
    assert_eq!(pod["spec"]["automountServiceAccountToken"], json!(false));
    assert_eq!(pod["spec"]["nodeSelector"]["kubernetes.io/arch"], "arm64");
    let security = &container["securityContext"];
    assert_eq!(security["allowPrivilegeEscalation"], json!(false));
    assert_eq!(security["capabilities"]["drop"], json!(["ALL"]));
    assert_eq!(
        pod["metadata"]["labels"],
        json!({"glave.test": "run", "glave.worker": "glave-worker-1"})
    );
    let starter = serde_json::to_string(&spec.starter).unwrap();
    assert_eq!(
        pod["metadata"]["annotations"],
        json!({"glave.started-by": starter})
    );
    let command = container["command"].to_string();
    assert!(
        command.contains(
            "\"/opt/glave/glave\",\"computer\",\"serve\",\"--socket\",\
             \"/opt/glave/glave.sock\",\"/opt/glave/secret\"]"
        ),
        "{command}"
    );
    assert_eq!(
        container["env"],
        json!([{"name": "GREETING", "value": "hi"}])
    );
    assert_eq!(pod["spec"]["volumes"][1]["emptyDir"]["medium"], "Memory");
    assert!(
        !pod.to_string().contains(spec.token.secret()),
        "the token is never in the Pod"
    );
    assert!(container.get("ports").is_none() && container.get("stdin").is_none());
    let parsed: k8s_openapi::api::core::v1::Pod = serde_json::from_value(pod).unwrap();
    assert!(parsed.spec.is_some());
}

#[test]
fn a_worker_is_owned_by_the_pod_glave_runs_in_when_there_is_one() {
    let labels = BTreeMap::new();
    let spec = spec(true);
    let alone = shape(&spec, &labels);
    assert_eq!(alone.pod()["metadata"]["ownerReferences"], json!([]));
    let owner = Owner {
        name: "glave-0".into(),
        uid: "8d1c2a52-0000-4000-8000-000000000001".into(),
    };
    let with_owner = Shape {
        owner: Some(&owner),
        ..alone
    };
    let reference = json!([{
        "apiVersion": "v1",
        "kind": "Pod",
        "name": "glave-0",
        "uid": "8d1c2a52-0000-4000-8000-000000000001"
    }]);
    assert_eq!(with_owner.pod()["metadata"]["ownerReferences"], reference);
    assert_eq!(
        with_owner.network_policy()["metadata"]["ownerReferences"],
        reference
    );
    let parsed: k8s_openapi::api::core::v1::Pod = serde_json::from_value(with_owner.pod()).unwrap();
    assert_eq!(
        parsed.metadata.owner_references.map(|owners| owners.len()),
        Some(1)
    );
}

#[test]
fn a_persons_computer_mounts_its_disk_as_home_and_is_never_a_worker() {
    let labels = BTreeMap::from([("glave.test".to_owned(), "run".to_owned())]);
    let mut spec = spec(true);
    spec.starter = None;
    spec.env.insert("HOME".into(), "/root".into());
    let computer = person(&spec, &labels);
    let pod = computer.pod();
    let container = &pod["spec"]["containers"][0];
    assert_eq!(
        pod["metadata"]["labels"],
        json!({"glave.test": "run", "glave.computer": "k1"})
    );
    assert_eq!(pod["metadata"]["ownerReferences"], json!([]));
    assert_eq!(
        pod["spec"]["volumes"][1],
        json!({"name": "home", "persistentVolumeClaim": {"claimName": "glave-home-k1"}})
    );
    assert_eq!(
        container["volumeMounts"][1],
        json!({"name": "home", "mountPath": "/home/glave"})
    );
    assert_eq!(
        container["env"],
        json!([
            {"name": "GREETING", "value": "hi"},
            {"name": "HOME", "value": "/home/glave"}
        ])
    );
    assert_eq!(
        container["resources"]["limits"],
        json!({"cpu": "2", "memory": "4Gi", "ephemeral-storage": "10Gi"})
    );
    assert_eq!(
        container["resources"]["requests"],
        json!({"cpu": "2", "memory": "4Gi", "ephemeral-storage": "1Gi"})
    );
    let command = container["command"].as_array().unwrap();
    assert_eq!(
        command.get(command.len() - 2..).unwrap(),
        [json!("--idle"), json!("10")],
        "it sleeps on its own once idle"
    );
    let policy = computer.network_policy();
    assert_eq!(
        policy["spec"]["podSelector"]["matchLabels"],
        json!({"glave.computer": "k1"})
    );

    let parsed: k8s_openapi::api::core::v1::Pod = serde_json::from_value(pod).unwrap();
    assert!(parsed.spec.is_some());
}

#[test]
fn a_persons_computer_is_never_root_and_reaches_nothing_in_the_cluster() {
    let labels = BTreeMap::new();
    let spec = spec(true);
    let computer = person(&spec, &labels);
    let pod = computer.pod();
    let container = &pod["spec"]["containers"][0];
    assert_eq!(
        container["volumeMounts"][2],
        json!({"name": "tmp", "mountPath": "/tmp"})
    );
    assert_eq!(
        container["securityContext"],
        json!({
            "allowPrivilegeEscalation": false,
            "privileged": false,
            "capabilities": {"drop": ["ALL"]},
            "seccompProfile": {"type": "RuntimeDefault"},
            "runAsNonRoot": true,
            "runAsUser": 65532,
            "runAsGroup": 65532,
            "readOnlyRootFilesystem": true
        }),
        "never root, nothing written outside its home and /tmp"
    );
    assert_eq!(
        pod["spec"]["securityContext"],
        json!({"fsGroup": 65532, "fsGroupChangePolicy": "OnRootMismatch"})
    );
    let policy = computer.network_policy();
    let out = &policy["spec"]["egress"][0]["to"];
    let refused = |at: usize| out[at]["ipBlock"]["except"].as_array().unwrap().clone();
    for inside in [
        "10.0.0.0/8",
        "172.16.0.0/12",
        "192.168.0.0/16",
        "100.64.0.0/10",
    ] {
        assert!(refused(0).contains(&json!(inside)), "{inside} is refused");
    }
    assert!(refused(1).contains(&json!("fc00::/7")));
    assert_eq!(policy["spec"]["ingress"], json!([]), "nothing comes in");
}

#[test]
fn without_the_network_glave_still_serves_on_its_socket_and_nothing_goes_in_or_out() {
    let labels = BTreeMap::new();
    let off = spec(false);
    let shape = shape(&off, &labels);
    let pod = shape.pod();
    let container = &pod["spec"]["containers"][0];
    assert_eq!(
        container["command"],
        shape_of(&spec(true), &labels)["spec"]["containers"][0]["command"],
        "the same with the network or without"
    );
    let policy = shape.network_policy();
    assert_eq!(policy["spec"]["egress"], json!([]));
    assert_eq!(policy["spec"]["ingress"], json!([]));
    assert_eq!(
        policy["spec"]["podSelector"]["matchLabels"]["glave.worker"],
        "glave-worker-1"
    );
    let on = spec(true);
    let open = Shape { spec: &on, ..shape }.network_policy();
    assert_eq!(
        open["spec"]["egress"][0]["to"],
        json!([
            {"ipBlock": {
                "cidr": "0.0.0.0/0",
                "except": ["169.254.0.0/16", "100.100.100.200/32", "168.63.129.16/32"]
            }},
            {"ipBlock": {
                "cidr": "::/0",
                "except": [
                    "fe80::/10",
                    "fd00:ec2::254/128",
                    "fd00:ec2::23/128",
                    "fd20:ce::254/128",
                    "fd00:a9fe:a9fe::1/128",
                    "fd00:42::42/128"
                ]
            }}
        ])
    );
    assert_eq!(open["spec"]["ingress"], json!([]));
    let parsed: k8s_openapi::api::networking::v1::NetworkPolicy =
        serde_json::from_value(open).unwrap();
    assert!(parsed.spec.is_some());
}

#[test]
fn the_token_goes_in_and_ready_comes_last() {
    let spec = spec(true);
    let archive = files(b"glave", &spec).unwrap();
    let text = String::from_utf8_lossy(&archive);
    assert!(text.contains("opt/glave/secret/token") && text.contains(spec.token.secret()));
    assert!(text.rfind("opt/glave/ready") > text.rfind("opt/glave/secret/token"));
    assert!(
        !text.contains("PRIVATE KEY"),
        "no TLS: the exec runs inside the API server's"
    );
    assert_eq!(arch(Some(0x3E)), Some("amd64"));
    assert_eq!(arch(None), None);
}

#[test]
fn a_starting_pod_is_running_waiting_or_failed() {
    let pod = |state: Value| {
        json!({"status": {"phase": "Pending", "containerStatuses": [
            {"name": CONTAINER, "state": state}
        ]}})
    };
    assert_eq!(progress(&pod(json!({"running": {}}))), Progress::Running);
    let pulling = progress(&pod(json!({"waiting": {"reason": "ContainerCreating"}})));
    assert!(matches!(pulling, Progress::Waiting(_)));
    let bad = progress(&pod(
        json!({"waiting": {"reason": "ErrImagePull", "message": "not found"}}),
    ));
    assert!(matches!(bad, Progress::Failed(why) if why.contains("not found")));
    let ended = progress(&pod(
        json!({"terminated": {"reason": "Error", "exitCode": 1}}),
    ));
    assert!(matches!(ended, Progress::Failed(why) if why.contains("exit code 1")));
    let unscheduled = json!({"status": {"phase": "Pending", "conditions": [
        {"type": "PodScheduled", "status": "False", "reason": "Unschedulable",
         "message": "0/1 nodes are available"}
    ]}});
    assert!(matches!(progress(&unscheduled), Progress::Waiting(why) if why.contains("0/1 nodes")));
}

#[test]
fn a_resize_is_done_when_its_container_runs_at_the_new_size_and_refused_when_it_cannot_fit() {
    let amounts = json!({"cpu": "500m", "memory": "1Gi"});
    let pod = |limits: Value, conditions: Value| {
        json!({"status": {
            "conditions": conditions,
            "containerStatuses": [{"name": CONTAINER, "resources": {"limits": limits}}]
        }})
    };
    let small = json!({"cpu": "250m", "memory": "1Gi"});
    assert_eq!(
        resized(&pod(small.clone(), json!([])), &amounts),
        Resized::Pending
    );
    assert_eq!(
        resized(&pod(amounts.clone(), json!([])), &amounts),
        Resized::Done
    );
    let infeasible = json!([{"type": "PodResizePending", "status": "True",
        "reason": "Infeasible", "message": "Node didn't have enough capacity"}]);
    assert!(matches!(
        resized(&pod(small.clone(), infeasible), &amounts),
        Resized::Refused(why) if why.contains("enough capacity")
    ));
    let deferred = json!([{"type": "PodResizePending", "status": "True", "reason": "Deferred"}]);
    assert_eq!(resized(&pod(small, deferred), &amounts), Resized::Pending);
}
