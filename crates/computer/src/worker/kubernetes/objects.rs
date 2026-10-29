//! The objects one worker is on Kubernetes, as the API server takes them: a Pod that waits for
//! glave and then serves with it on a Unix socket, and a NetworkPolicy that keeps the worker's
//! network as policy decided. And the files the harness copies in, and how far a starting Pod
//! has got.

use std::collections::BTreeMap;

use glave_ports::host::{CLOUD_METADATA, INSIDE, Network};
use serde_json::{Value, json};

use super::super::backend::{STARTED_BY, Spec};
use super::super::tar::{Entry, TooLong, archive};

/// The container that runs the work.
pub(crate) const CONTAINER: &str = "worker";
/// Where glave goes in the worker: an empty folder the harness copies into.
const HOME: &str = "/opt/glave";
/// glave in the worker.
pub(crate) const GLAVE: &str = "/opt/glave/glave";
/// The socket glave serves on in the worker, which each bridge connects to.
pub(crate) const SOCKET: &str = "/opt/glave/glave.sock";
/// The folder of the worker's token, in memory.
const SECRET: &str = "/opt/glave/secret";
/// The worker's token, from `/`: renewed in place.
pub(crate) const TOKEN: &str = "opt/glave/secret/token";
/// What the harness copies in last, so the worker knows everything else has arrived.
const READY: &str = "opt/glave/ready";
/// The label every worker object carries, so leftovers can be found.
pub(crate) const LABEL: &str = "glave.worker";
/// The label a person's computer's objects carry instead, its key as the value: never a
/// worker's, so no glave reaps a person's computer as a worker it left.
pub(crate) const COMPUTER_LABEL: &str = "glave.computer";
/// The user and group a person's computer runs as: Wolfi's `nonroot`, never root, owning
/// its disk (`specs/computers.md`, decision 12).
const PERSON_USER: u32 = 65_532;
/// How long a person's computer goes unasked, with nothing running in the background, before
/// it ends on its own: asleep after 10 minutes idle (`specs/computers.md`, decision 4).
pub(crate) const IDLE_MINUTES: u32 = 10;
/// Where a person's disk is mounted in their computer: their home.
pub(crate) const PERSON_HOME: &str = "/home/glave";
/// Where glave on a person's computer makes scratch folders: its folder in their home.
pub(crate) const PERSON_SCRATCH: &str = "/home/glave/.glave/scratch";
/// The most a person's computer may write outside its disk (its container's layer, `/tmp`),
/// so one person can never fill a node's disk.
const OUTSIDE_THE_DISK: &str = "10Gi";
/// What a person's computer reserves of a node's disk: its glave and a little work in `/tmp`.
const ON_THE_NODE: &str = "1Gi";

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

/// The Pod glave runs in, which its workers name as their owner: when it is deleted, the
/// cluster removes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Owner {
    pub(crate) name: String,
    pub(crate) uid: String,
}

/// A person's computer: its key, and the claim that is its disk.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Home<'a> {
    pub(crate) key: &'a str,
    pub(crate) claim: &'a str,
    /// The minutes it goes unasked before it ends on its own.
    pub(crate) idle_minutes: u32,
}

/// What one worker's objects are made of.
pub(crate) struct Shape<'a> {
    pub(crate) name: &'a str,
    pub(crate) spec: &'a Spec,
    pub(crate) runtime_class: Option<&'a str>,
    /// The processor glave was built for, as Kubernetes names it, when known.
    pub(crate) arch: Option<&'a str>,
    /// Labels on every object besides its own, such as a test run's.
    pub(crate) labels: &'a BTreeMap<String, String>,
    /// The Pod glave runs in, when it runs in one beside its workers.
    pub(crate) owner: Option<&'a Owner>,
    /// The person whose computer this is, when it is one: their disk is its home.
    pub(crate) home: Option<Home<'a>>,
}

impl Shape<'_> {
    /// The worker's labels: the extra ones and its own name, or a person's computer's key.
    pub(crate) fn labels(&self) -> BTreeMap<String, String> {
        let mut labels = self.labels.clone();
        let (label, value) = self.own_label();
        labels.insert(label.to_owned(), value.to_owned());
        labels
    }

    /// The label that is this object's own, and its value.
    fn own_label(&self) -> (&'static str, &str) {
        match self.home {
            Some(home) => (COMPUTER_LABEL, home.key),
            None => (LABEL, self.name),
        }
    }

    /// The worker's annotations: the glave that started it, when known ([`STARTED_BY`]),
    /// which a label's value cannot hold.
    fn annotations(&self) -> BTreeMap<String, String> {
        self.spec
            .starter
            .as_ref()
            .and_then(|starter| serde_json::to_string(starter).ok())
            .map(|starter| BTreeMap::from([(STARTED_BY.to_owned(), starter)]))
            .unwrap_or_default()
    }

    /// The metadata of each of the worker's objects.
    fn metadata(&self) -> Value {
        let owners: Vec<Value> = self
            .owner
            .iter()
            .map(|owner| {
                json!({"apiVersion": "v1", "kind": "Pod", "name": owner.name, "uid": owner.uid})
            })
            .collect();
        json!({
            "name": self.name,
            "labels": self.labels(),
            "annotations": self.annotations(),
            "ownerReferences": owners,
        })
    }

    /// The Pod: the environment's image running the wait, then glave serving on [`SOCKET`].
    /// Limits as requests too, so the worker gets what it is allowed; no service account token;
    /// no privilege; every capability dropped; the runtime's seccomp profile; and a runtime
    /// class when one is set.
    pub(crate) fn pod(&self) -> Value {
        let command = self.command();
        let mut env: Vec<Value> = self
            .spec
            .env
            .iter()
            .filter(|(name, _)| self.home.is_none() || name.as_str() != "HOME")
            .map(|(name, value)| json!({"name": name, "value": value}))
            .collect();
        let (mut amounts, mut limits) = self.amounts();
        let mut mounts = vec![json!({"name": "glave", "mountPath": HOME})];
        let mut volumes = vec![json!({"name": "glave", "emptyDir": {}})];
        if let Some(home) = self.home {
            env.push(json!({"name": "HOME", "value": PERSON_HOME}));
            mounts.push(json!({"name": "home", "mountPath": PERSON_HOME}));
            volumes.push(json!({
                "name": "home",
                "persistentVolumeClaim": {"claimName": home.claim}
            }));
            // The root filesystem is read-only; `/tmp` is the one place outside the home a
            // command may write, within the computer's `ephemeral-storage`.
            mounts.push(json!({"name": "tmp", "mountPath": "/tmp"}));
            volumes.push(json!({"name": "tmp", "emptyDir": {}}));
            limits.insert("ephemeral-storage".into(), json!(OUTSIDE_THE_DISK));
            // Asked for explicitly: a limit alone would be asked for whole, and reserve it on
            // the node for every computer awake.
            amounts.insert("ephemeral-storage".into(), json!(ON_THE_NODE));
        }
        // The token lives in memory, never on the node's disk.
        mounts.push(json!({"name": "secret", "mountPath": SECRET}));
        volumes.push(json!({
            "name": "secret",
            "emptyDir": {"medium": "Memory", "sizeLimit": "1Mi"}
        }));
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
            "resources": {"requests": amounts, "limits": limits},
            "volumeMounts": mounts,
            "securityContext": self.container_security(),
        });
        json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": self.metadata(),
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
                "securityContext": self.pod_security(),
            }
        })
    }

    /// The container's security: no privilege, every capability dropped, the runtime's
    /// seccomp profile; a person's computer also never root, its root filesystem read-only.
    fn container_security(&self) -> Value {
        match &self.home {
            Some(_) => json!({
                "allowPrivilegeEscalation": false,
                "privileged": false,
                "capabilities": {"drop": ["ALL"]},
                "seccompProfile": {"type": "RuntimeDefault"},
                "runAsNonRoot": true,
                "runAsUser": PERSON_USER,
                "runAsGroup": PERSON_USER,
                "readOnlyRootFilesystem": true
            }),
            None => json!({
                "allowPrivilegeEscalation": false,
                "privileged": false,
                "capabilities": {"drop": ["ALL"]},
                "seccompProfile": {"type": "RuntimeDefault"}
            }),
        }
    }

    /// The Pod's security: for a person's computer, its disk owned by its group, set only when
    /// it differs, so a large home is not walked at every wake; a worker runs as its image
    /// says.
    fn pod_security(&self) -> Value {
        match &self.home {
            Some(_) => json!({
                "fsGroup": PERSON_USER,
                "fsGroupChangePolicy": "OnRootMismatch"
            }),
            None => json!({}),
        }
    }

    /// The container's command: the wait, then glave serving on [`SOCKET`] to the bridges
    /// that come in through the API server, each signed in with the token in [`SECRET`].
    fn command(&self) -> Vec<Value> {
        let mut command: Vec<Value> = [
            "/bin/sh", "-c", WAIT, "wait", GLAVE, "computer", "serve", "--socket", SOCKET, SECRET,
        ]
        .into_iter()
        .map(Value::from)
        .collect();
        // A person's computer sleeps on its own once idle, servers or not (#558).
        if let Some(home) = &self.home {
            command.extend([json!("--idle"), json!(home.idle_minutes.to_string())]);
        }
        command
    }

    /// The CPU and memory the size asks for, as requests and as limits (the same).
    fn amounts(
        &self,
    ) -> (
        serde_json::Map<String, Value>,
        serde_json::Map<String, Value>,
    ) {
        let mut amounts = serde_json::Map::new();
        if let Some(cpus) = self.spec.cpus {
            amounts.insert("cpu".into(), json!(cpus.to_string()));
        }
        if let Some(gb) = self.spec.memory_gb {
            amounts.insert("memory".into(), json!(format!("{gb}Gi")));
        }
        (amounts.clone(), amounts)
    }

    /// The worker's NetworkPolicy: nothing comes in (the harness arrives through an exec
    /// through the API server, which no policy sees), and nothing goes out unless
    /// the worker has the network, which then reaches anywhere but where clouds serve their
    /// machines' credentials ([`CLOUD_METADATA`]), and for a person's computer anything that is
    /// not the internet ([`INSIDE`]); DNS on port 53 is still allowed.
    pub(crate) fn network_policy(&self) -> Value {
        let egress = if self.spec.network {
            // A person's computer reaches nothing inside the cluster either.
            let inside: &[Network] = if self.home.is_some() { &INSIDE } else { &[] };
            let except = |v4: bool| -> Vec<String> {
                CLOUD_METADATA
                    .iter()
                    .chain(inside)
                    .filter(|network| network.is_ipv4() == v4)
                    .map(ToString::to_string)
                    .collect()
            };
            json!([
                {"to": [
                    {"ipBlock": {"cidr": "0.0.0.0/0", "except": except(true)}},
                    {"ipBlock": {"cidr": "::/0", "except": except(false)}}
                ]},
                {"ports": [{"protocol": "UDP", "port": 53}, {"protocol": "TCP", "port": 53}]}
            ])
        } else {
            json!([])
        };
        let (label, value) = self.own_label();
        json!({
            "apiVersion": "networking.k8s.io/v1",
            "kind": "NetworkPolicy",
            "metadata": self.metadata(),
            "spec": {
                "podSelector": {"matchLabels": {label: value}},
                "policyTypes": ["Ingress", "Egress"],
                "ingress": [],
                "egress": egress,
            }
        })
    }
}

/// glave and the worker's token, as a tar archive extracted at `/`; `ready` last, so the wait
/// ends only once everything is in.
pub(crate) fn files(glave: &[u8], spec: &Spec) -> Result<Vec<u8>, TooLong> {
    let token = spec.token.to_file();
    archive(&[
        Entry::File {
            name: "opt/glave/glave",
            mode: 0o755,
            content: glave,
        },
        Entry::File {
            name: TOKEN,
            mode: 0o600,
            content: token.as_bytes(),
        },
        Entry::File {
            name: READY,
            mode: 0o644,
            content: b"",
        },
    ])
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
mod tests;
