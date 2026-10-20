//! The objects one worker is on Kubernetes, as the API server takes them: a Pod that waits for
//! glave and then serves with it, and a NetworkPolicy that keeps the worker's network as
//! policy decided. And the files the harness copies in, and how far a starting Pod has got.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::super::backend::{PORT, Spec};
use super::super::tar::{Entry, TooLong, archive};

/// The container that runs the work.
pub(crate) const CONTAINER: &str = "worker";
/// Where glave goes in the worker: an empty folder the harness copies into.
const HOME: &str = "/opt/glave";
/// What the harness copies in last, so the worker knows everything else has arrived.
const READY: &str = "opt/glave/ready";
/// The label every worker object carries, so leftovers can be found.
pub(crate) const LABEL: &str = "glave.worker";

/// Waits (up to ten minutes) for the harness to copy glave in, then becomes it. Run by
/// `/bin/sh -c`, with glave's command line as its arguments.
const WAIT: &str = "i=0; until [ -e /opt/glave/ready ]; do i=$((i+1)); \
if [ \"$i\" -gt 3000 ]; then echo 'glave never arrived in the worker' >&2; exit 1; fi; \
sleep 0.2; done; exec \"$@\"";

/// Where waiting containers are stuck for good: an image that cannot be had, or a container
/// that cannot be made.
const STUCK: [&str; 6] = [
    "ErrImagePull",
    "ImagePullBackOff",
    "InvalidImageName",
    "CreateContainerConfigError",
    "CreateContainerError",
    "RunContainerError",
];

/// What one worker's objects are made of.
pub(crate) struct Shape<'a> {
    pub(crate) name: &'a str,
    pub(crate) spec: &'a Spec,
    pub(crate) runtime_class: Option<&'a str>,
    /// The processor glave was built for, as Kubernetes names it, when known.
    pub(crate) arch: Option<&'a str>,
    /// Labels on every object besides its own, such as a test run's.
    pub(crate) labels: &'a BTreeMap<String, String>,
}

impl Shape<'_> {
    /// The worker's labels: the extra ones and its own name.
    pub(crate) fn labels(&self) -> BTreeMap<String, String> {
        let mut labels = self.labels.clone();
        labels.insert(LABEL.to_owned(), self.name.to_owned());
        labels
    }

    /// The Pod: the environment's image running the wait, then glave serving over TLS on
    /// [`PORT`] or on its standard input and output (no network). Limits as requests too, so
    /// the worker gets what it is allowed; no service account token; no privilege; every
    /// capability dropped; the runtime's seccomp profile; and a runtime class when one is set.
    pub(crate) fn pod(&self) -> Value {
        let serve = if self.spec.network {
            json!([
                format!("{HOME}/glave"),
                "computer",
                "serve",
                "--tls",
                format!("0.0.0.0:{PORT}"),
                format!("{HOME}/tls")
            ])
        } else {
            json!([format!("{HOME}/glave"), "computer", "serve", "--stdio"])
        };
        let mut command = vec![json!("/bin/sh"), json!("-c"), json!(WAIT), json!("wait")];
        command.extend(serve.as_array().cloned().unwrap_or_default());
        let env: Vec<Value> = self
            .spec
            .env
            .iter()
            .map(|(name, value)| json!({"name": name, "value": value}))
            .collect();
        let mut amounts = serde_json::Map::new();
        if let Some(cpus) = self.spec.cpus {
            amounts.insert("cpu".into(), json!(cpus.to_string()));
        }
        if let Some(gb) = self.spec.memory_gb {
            amounts.insert("memory".into(), json!(format!("{gb}Gi")));
        }
        let mut mounts = vec![json!({"name": "glave", "mountPath": HOME})];
        let mut volumes = vec![json!({"name": "glave", "emptyDir": {}})];
        if self.spec.network {
            // The identity lives in memory, never on the node's disk.
            mounts.push(json!({"name": "tls", "mountPath": format!("{HOME}/tls")}));
            volumes.push(json!({
                "name": "tls",
                "emptyDir": {"medium": "Memory", "sizeLimit": "1Mi"}
            }));
        }
        let ports = if self.spec.network {
            json!([{"name": "glave", "containerPort": PORT, "protocol": "TCP"}])
        } else {
            json!([])
        };
        let mut nodes = serde_json::Map::new();
        nodes.insert("kubernetes.io/os".into(), json!("linux"));
        if let Some(arch) = self.arch {
            nodes.insert("kubernetes.io/arch".into(), json!(arch));
        }
        let container = json!({
            "name": CONTAINER,
            "image": self.spec.image,
            "command": command,
            "env": env,
            "ports": ports,
            "stdin": !self.spec.network,
            "stdinOnce": !self.spec.network,
            "resources": {"requests": amounts.clone(), "limits": amounts},
            "volumeMounts": mounts,
            "securityContext": {
                "allowPrivilegeEscalation": false,
                "privileged": false,
                "capabilities": {"drop": ["ALL"]},
                "seccompProfile": {"type": "RuntimeDefault"}
            }
        });
        json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {"name": self.name, "labels": self.labels()},
            "spec": {
                "containers": [container],
                "volumes": volumes,
                "restartPolicy": "Never",
                "automountServiceAccountToken": false,
                "enableServiceLinks": false,
                // The Pod's pause process is its first, and reaps what glave's commands leave.
                "shareProcessNamespace": true,
                "terminationGracePeriodSeconds": 5,
                "runtimeClassName": self.runtime_class,
                "nodeSelector": nodes,
            }
        })
    }

    /// The worker's NetworkPolicy: nothing comes in (the harness arrives through the API
    /// server's port-forward or attach, which no policy sees), and nothing goes out unless
    /// the worker has the network, which then reaches anywhere but link-local addresses,
    /// where clouds serve their machines' credentials (DNS there is still allowed).
    pub(crate) fn network_policy(&self) -> Value {
        let egress = if self.spec.network {
            json!([
                {"to": [
                    {"ipBlock": {"cidr": "0.0.0.0/0", "except": ["169.254.0.0/16"]}},
                    {"ipBlock": {"cidr": "::/0", "except": ["fe80::/10", "fd00:ec2::254/128"]}}
                ]},
                {"ports": [{"protocol": "UDP", "port": 53}, {"protocol": "TCP", "port": 53}]}
            ])
        } else {
            json!([])
        };
        json!({
            "apiVersion": "networking.k8s.io/v1",
            "kind": "NetworkPolicy",
            "metadata": {"name": self.name, "labels": self.labels()},
            "spec": {
                "podSelector": {"matchLabels": {LABEL: self.name}},
                "policyTypes": ["Ingress", "Egress"],
                "ingress": [],
                "egress": egress,
            }
        })
    }
}

/// glave and, for a worker with a network, its identity, as a tar archive extracted at `/`;
/// `ready` last, so the wait ends only once everything is in.
pub(crate) fn files(glave: &[u8], spec: &Spec) -> Result<Vec<u8>, TooLong> {
    let token = spec.token.to_file();
    let mut entries = vec![Entry::File {
        name: "opt/glave/glave",
        mode: 0o755,
        content: glave,
    }];
    if spec.network {
        entries.extend([
            Entry::File {
                name: "opt/glave/tls/cert.pem",
                mode: 0o644,
                content: spec.identity.certificate.as_bytes(),
            },
            Entry::File {
                name: "opt/glave/tls/key.pem",
                mode: 0o600,
                content: spec.identity.key.as_bytes(),
            },
            Entry::File {
                name: "opt/glave/tls/token",
                mode: 0o600,
                content: token.as_bytes(),
            },
        ]);
    }
    entries.push(Entry::File {
        name: READY,
        mode: 0o644,
        content: b"",
    });
    archive(&entries)
}

/// Kubernetes' name for the processor of an ELF machine number.
pub(crate) fn arch(machine: Option<u16>) -> Option<&'static str> {
    match machine {
        Some(0xB7) => Some("arm64"),
        Some(0x3E) => Some("amd64"),
        _ => None,
    }
}

/// How far a starting Pod has got.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Progress {
    /// The worker container runs.
    Running,
    /// It will never run, and why.
    Failed(String),
    /// Not yet, and what it waits for.
    Waiting(String),
}

/// How far the Pod `pod` (as the API server returns it) has got.
pub(crate) fn progress(pod: &Value) -> Progress {
    fn text(value: Option<&Value>) -> &str {
        value.and_then(Value::as_str).unwrap_or_default()
    }
    let phase = text(pod.pointer("/status/phase"));
    let worker = pod
        .pointer("/status/containerStatuses")
        .and_then(Value::as_array)
        .and_then(|all| {
            all.iter()
                .find(|c| c.get("name") == Some(&json!(CONTAINER)))
        });
    let state = worker.and_then(|status| status.get("state"));
    if let Some(ended) = state.and_then(|state| state.get("terminated")) {
        return Progress::Failed(format!(
            "the worker stopped ({}, exit code {}) {}",
            text(ended.get("reason")),
            ended.get("exitCode").and_then(Value::as_i64).unwrap_or(-1),
            text(ended.get("message"))
        ));
    }
    if matches!(phase, "Failed" | "Succeeded") {
        return Progress::Failed(format!(
            "the worker stopped ({phase}): {}",
            text(pod.pointer("/status/message"))
        ));
    }
    if state.is_some_and(|state| state.get("running").is_some()) {
        return Progress::Running;
    }
    if let Some(waiting) = state.and_then(|state| state.get("waiting")) {
        let reason = text(waiting.get("reason"));
        let why = format!("{reason}: {}", text(waiting.get("message")));
        return if STUCK.contains(&reason) {
            Progress::Failed(format!("the worker cannot start ({why})"))
        } else {
            Progress::Waiting(why)
        };
    }
    let conditions: Vec<String> = pod
        .pointer("/status/conditions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|condition| condition.get("status") == Some(&json!("False")))
        .map(|condition| {
            format!(
                "{}: {}",
                text(condition.get("reason")),
                text(condition.get("message"))
            )
        })
        .collect();
    Progress::Waiting(format!("{phase} {}", conditions.join("; ")))
}

#[cfg(test)]
mod tests {
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
            cpus: Some(2),
            memory_gb: Some(4),
            network,
            identity: Identity::mint().unwrap(),
            token: Token::mint(
                SystemTime::from(SystemClock.now()),
                std::time::Duration::from_secs(60),
            )
            .unwrap(),
        }
    }

    fn shape<'a>(spec: &'a Spec, labels: &'a BTreeMap<String, String>) -> Shape<'a> {
        Shape {
            name: "glave-worker-1",
            spec,
            runtime_class: Some("gvisor"),
            arch: Some("arm64"),
            labels,
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
        let command = container["command"].to_string();
        assert!(command.contains("\"--tls\",\"0.0.0.0:7070\""), "{command}");
        assert_eq!(
            container["env"],
            json!([{"name": "GREETING", "value": "hi"}])
        );
        assert_eq!(pod["spec"]["volumes"][1]["emptyDir"]["medium"], "Memory");
        assert!(
            !pod.to_string().contains("PRIVATE KEY"),
            "the key is never in the Pod"
        );
        let parsed: k8s_openapi::api::core::v1::Pod = serde_json::from_value(pod).unwrap();
        assert!(parsed.spec.is_some());
    }

    #[test]
    fn without_the_network_glave_is_on_stdio_and_nothing_goes_in_or_out() {
        let labels = BTreeMap::new();
        let off = spec(false);
        let shape = shape(&off, &labels);
        let pod = shape.pod();
        let container = &pod["spec"]["containers"][0];
        assert_eq!(container["stdin"], json!(true));
        assert!(container["command"].to_string().contains("--stdio"));
        assert_eq!(container["ports"], json!([]));
        let policy = shape.network_policy();
        assert_eq!(policy["spec"]["egress"], json!([]));
        assert_eq!(policy["spec"]["ingress"], json!([]));
        assert_eq!(
            policy["spec"]["podSelector"]["matchLabels"]["glave.worker"],
            "glave-worker-1"
        );
        let on = spec(true);
        let open = Shape { spec: &on, ..shape }.network_policy();
        let egress = open["spec"]["egress"].to_string();
        assert!(egress.contains("169.254.0.0/16"), "{egress}");
        assert_eq!(open["spec"]["ingress"], json!([]));
        let parsed: k8s_openapi::api::networking::v1::NetworkPolicy =
            serde_json::from_value(open).unwrap();
        assert!(parsed.spec.is_some());
    }

    #[test]
    fn the_identity_goes_in_only_with_the_network_and_ready_comes_last() {
        let with = files(b"glave", &spec(true)).unwrap();
        let without = files(b"glave", &spec(false)).unwrap();
        let text = String::from_utf8_lossy(&with);
        assert!(text.contains("opt/glave/tls/key.pem") && text.contains("PRIVATE KEY"));
        assert!(text.rfind("opt/glave/ready") > text.rfind("opt/glave/tls/token"));
        assert!(!String::from_utf8_lossy(&without).contains("PRIVATE KEY"));
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
        assert!(
            matches!(progress(&unscheduled), Progress::Waiting(why) if why.contains("0/1 nodes"))
        );
    }
}
