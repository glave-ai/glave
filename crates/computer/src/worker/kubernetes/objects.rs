//! The objects a person's computer is on Kubernetes while awake, as the API server takes them:
//! a Pod that waits for glave and then serves with it on a Unix socket, and a NetworkPolicy
//! that keeps its network as policy decided. And the files the harness copies in, or the glave
//! its image carries, and how far a starting Pod has got.

use std::collections::BTreeMap;

use glave_ports::host::{CLOUD_METADATA, INSIDE};
use serde_json::{Value, json};

use super::super::tar::{Entry, TooLong, archive};
use super::Spec;

/// The container that runs the work.
pub(crate) const CONTAINER: &str = "worker";
/// Where glave goes in the computer: an empty folder the harness copies into.
const HOME: &str = "/opt/glave";
/// glave in the computer.
pub(crate) const GLAVE: &str = "/opt/glave/glave";
/// The socket glave serves on in the computer, which each bridge connects to.
pub(crate) const SOCKET: &str = "/opt/glave/glave.sock";
/// The folder of the computer's token, in memory.
const SECRET: &str = "/opt/glave/secret";
/// The computer's token, from `/`: renewed in place.
pub(crate) const TOKEN: &str = "opt/glave/secret/token";
/// What the harness copies in last, so the computer knows everything else has arrived.
const READY: &str = "opt/glave/ready";
/// Run by `/bin/sh -c` in a person's computer, with the SHA-256 of the glave the server chose as
/// `$0` and the token, as a tar archive, on standard input: puts the token in, then, only when
/// the glave glave's computer image carries (`images/computer`, with its SHA-256 beside it) is
/// that very glave, names it as the computer's and says everything has arrived. Fails
/// otherwise, and the server copies glave in instead.
const FROM_IMAGE: &str = "tar -x -f - -C / && \
[ \"$(cat /usr/libexec/glave/glave.sha256 2>/dev/null)\" = \"$0\" ] && \
ln -s /usr/libexec/glave/glave /opt/glave/glave && : > /opt/glave/ready";
/// The label a person's computer's objects carry, its key as the value.
pub(crate) const COMPUTER_LABEL: &str = "glave.computer";
/// The user and group a person's computer runs as: Wolfi's `nonroot`, never root, owning
/// its disk (`specs/computers.md`, decision 12).
const PERSON_USER: u32 = 65_532;
/// How long a person's computer goes unasked, with nothing running in the background, before
/// it ends on its own: asleep after 2 hours idle (`specs/computers.md`, decision 4).
pub(crate) const IDLE_MINUTES: u32 = 120;
/// Where a person's disk is mounted in their computer: their home.
pub(crate) const PERSON_HOME: &str = "/home/glave";
/// Where glave on a person's computer makes scratch folders: its folder in their home.
pub(crate) const PERSON_SCRATCH: &str = "/home/glave/.glave/scratch";
/// What a person's computer may write outside its disk (its container's layer, `/tmp`), asked
/// for and limited alike: room for a large install's temporary files, never its own disk, and
/// GKE Autopilot sets the limit to what is asked for anyway (owner, 2026-11-05).
const OUTSIDE_THE_DISK: &str = "4Gi";

/// Waits (up to ten minutes) for the harness to copy glave in, then becomes it. Run by
/// `/bin/sh -c`, with glave's command line as its arguments.
const WAIT: &str = "i=0; until [ -e /opt/glave/ready ]; do i=$((i+1)); \
if [ \"$i\" -gt 3000 ]; then echo 'glave never arrived in the computer' >&2; exit 1; fi; \
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

/// A person's computer: its key, the claim that is its disk, and the glave it serves with.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Home<'a> {
    pub(crate) key: &'a str,
    pub(crate) claim: &'a str,
    /// The minutes it goes unasked before it ends on its own.
    pub(crate) idle_minutes: u32,
    /// The SHA-256 of the glave the server chose for it: its image's own serves when it is
    /// that one, and is copied in otherwise.
    pub(crate) glave: &'a str,
}

impl Home<'_> {
    /// What runs in the computer to put its token in and serve with its image's glave, when
    /// that is the glave the server chose ([`FROM_IMAGE`]); it fails when it is not.
    pub(crate) fn image_glave(&self) -> [&str; 4] {
        ["/bin/sh", "-c", FROM_IMAGE, self.glave]
    }
}

/// What one person's computer's objects are made of.
pub(crate) struct Shape<'a> {
    pub(crate) name: &'a str,
    pub(crate) spec: &'a Spec,
    pub(crate) runtime_class: Option<&'a str>,
    /// The processor glave was built for, as Kubernetes names it, when known.
    pub(crate) arch: Option<&'a str>,
    /// Labels on every object besides its own, such as a test run's.
    pub(crate) labels: &'a BTreeMap<String, String>,
    /// The person whose computer this is: their disk is its home.
    pub(crate) home: Home<'a>,
}

impl Shape<'_> {
    /// The computer's labels: the extra ones and its key.
    pub(crate) fn labels(&self) -> BTreeMap<String, String> {
        let mut labels = self.labels.clone();
        labels.insert(COMPUTER_LABEL.to_owned(), self.home.key.to_owned());
        labels
    }

    /// The metadata of each of the computer's objects.
    fn metadata(&self) -> Value {
        json!({"name": self.name, "labels": self.labels()})
    }

    /// The Pod: the image running the wait, then glave serving on [`SOCKET`], the person's disk
    /// as its home. Limits as requests too, so the computer gets what it is allowed; no
    /// service account token; no privilege; every capability dropped; the runtime's seccomp
    /// profile; never root; a read-only root filesystem; and a runtime class when one is set.
    pub(crate) fn pod(&self) -> Value {
        let mut env: Vec<Value> = self
            .spec
            .env
            .iter()
            .filter(|(name, _)| name.as_str() != "HOME")
            .map(|(name, value)| json!({"name": name, "value": value}))
            .collect();
        env.push(json!({"name": "HOME", "value": PERSON_HOME}));
        let mut amounts = self.amounts();
        amounts.insert("ephemeral-storage".into(), json!(OUTSIDE_THE_DISK));
        // The root filesystem is read-only; `/tmp` is the one place outside the home a command
        // may write, within the computer's `ephemeral-storage`. The token lives in memory,
        // never on the node's disk.
        let mounts = json!([
            {"name": "glave", "mountPath": HOME},
            {"name": "home", "mountPath": PERSON_HOME},
            {"name": "tmp", "mountPath": "/tmp"},
            {"name": "secret", "mountPath": SECRET},
        ]);
        let volumes = json!([
            {"name": "glave", "emptyDir": {}},
            {"name": "home", "persistentVolumeClaim": {"claimName": self.home.claim}},
            {"name": "tmp", "emptyDir": {}},
            {"name": "secret", "emptyDir": {"medium": "Memory", "sizeLimit": "1Mi"}},
        ]);
        let mut nodes = serde_json::Map::new();
        nodes.insert("kubernetes.io/os".into(), json!("linux"));
        if let Some(arch) = self.arch {
            nodes.insert("kubernetes.io/arch".into(), json!(arch));
        }
        let container = json!({
            "name": CONTAINER,
            "image": self.spec.image,
            "command": self.command(),
            "env": env,
            "resources": {"requests": amounts, "limits": amounts},
            "volumeMounts": mounts,
            "securityContext": {
                "allowPrivilegeEscalation": false,
                "privileged": false,
                "capabilities": {"drop": ["ALL"]},
                "seccompProfile": {"type": "RuntimeDefault"},
                "runAsNonRoot": true,
                "runAsUser": PERSON_USER,
                "runAsGroup": PERSON_USER,
                "readOnlyRootFilesystem": true
            },
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
                // Its disk owned by its group, set only when it differs, so a large home is
                // not walked at every wake.
                "securityContext": {
                    "fsGroup": PERSON_USER,
                    "fsGroupChangePolicy": "OnRootMismatch"
                },
            }
        })
    }

    /// The container's command: the wait, then glave serving on [`SOCKET`] to the bridges
    /// that come in through the API server, each signed in with the token in [`SECRET`],
    /// sleeping on its own once idle, servers or not (#558), and keeping its disk from filling.
    fn command(&self) -> Vec<Value> {
        let idle = self.home.idle_minutes.to_string();
        [
            "/bin/sh",
            "-c",
            WAIT,
            "wait",
            GLAVE,
            "computer",
            "serve",
            "--socket",
            SOCKET,
            SECRET,
            "--idle",
            &idle,
            "--keep-disk",
        ]
        .into_iter()
        .map(Value::from)
        .collect()
    }

    /// The CPU and memory the size asks for, requested and limited alike.
    fn amounts(&self) -> serde_json::Map<String, Value> {
        let mut amounts = serde_json::Map::new();
        if let Some(millis) = self.spec.cpu_millis {
            amounts.insert("cpu".into(), json!(cpu(millis)));
        }
        if let Some(mib) = self.spec.memory_mib {
            amounts.insert("memory".into(), json!(memory(mib)));
        }
        amounts
    }

    /// The computer's NetworkPolicy: nothing comes in (the harness arrives through an exec
    /// through the API server, which no policy sees), and nothing goes out unless the computer
    /// has the network, which then reaches the internet alone: never where clouds serve their
    /// machines' credentials ([`CLOUD_METADATA`]), nor anything that is not the internet
    /// ([`INSIDE`]), so no other Pod, server or database in the cluster; DNS on port 53 is
    /// still allowed.
    pub(crate) fn network_policy(&self) -> Value {
        let egress = if self.spec.network {
            let except = |v4: bool| -> Vec<String> {
                CLOUD_METADATA
                    .iter()
                    .chain(&INSIDE)
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
        json!({
            "apiVersion": "networking.k8s.io/v1",
            "kind": "NetworkPolicy",
            "metadata": self.metadata(),
            "spec": {
                "podSelector": {"matchLabels": {COMPUTER_LABEL: self.home.key}},
                "policyTypes": ["Ingress", "Egress"],
                "ingress": [],
                "egress": egress,
            }
        })
    }
}

/// `millis` thousandths of a CPU as Kubernetes writes a quantity: whole CPUs plainly, else in
/// thousandths (`250m`).
pub(crate) fn cpu(millis: u32) -> String {
    if millis.is_multiple_of(1_000) {
        (millis / 1_000).to_string()
    } else {
        format!("{millis}m")
    }
}

/// `mib` MiB of memory as Kubernetes writes a quantity: whole GiB as such, else in MiB.
pub(crate) fn memory(mib: u32) -> String {
    if mib.is_multiple_of(1_024) {
        format!("{}Gi", mib / 1_024)
    } else {
        format!("{mib}Mi")
    }
}

/// glave and the computer's token, as a tar archive extracted at `/`; `ready` last, so the wait
/// ends only once everything is in.
pub(crate) fn files(glave: &[u8], spec: &Spec) -> Result<Vec<u8>, TooLong> {
    let token = spec.token.to_file();
    archive(&[
        Entry {
            name: "opt/glave/glave",
            mode: 0o755,
            content: glave,
        },
        Entry {
            name: TOKEN,
            mode: 0o600,
            content: token.as_bytes(),
        },
        Entry {
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

/// How far an in-place resize of a Pod has got.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Resized {
    /// Its container runs at the size asked for.
    Done,
    /// It will not be, and why: the node can never fit it, or the resize failed.
    Refused(String),
    /// Not yet.
    Pending,
}

/// How far the resize of `pod` (as the API server returns it) to `amounts` (its container's
/// `cpu` and `memory`, requested and limited alike) has got.
pub(crate) fn resized(pod: &Value, amounts: &Value) -> Resized {
    let refusal = pod
        .pointer("/status/conditions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|condition| {
            let (kind, reason) = (condition.get("type"), condition.get("reason"));
            condition.get("status") == Some(&json!("True"))
                && ((kind == Some(&json!("PodResizePending"))
                    && reason == Some(&json!("Infeasible")))
                    || (kind == Some(&json!("PodResizeInProgress"))
                        && reason == Some(&json!("Error"))))
        });
    if let Some(condition) = refusal {
        let text = |key: &str| {
            condition
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
        };
        return Resized::Refused(format!(
            "the computer cannot grow there ({}: {})",
            text("reason"),
            text("message")
        ));
    }
    let running = pod
        .pointer("/status/containerStatuses")
        .and_then(Value::as_array)
        .and_then(|all| {
            all.iter()
                .find(|c| c.get("name") == Some(&json!(CONTAINER)))
        })
        .and_then(|status| status.pointer("/resources/limits"));
    let at = |key: &str| running.and_then(|limits| limits.get(key)) == amounts.get(key);
    if at("cpu") && at("memory") {
        Resized::Done
    } else {
        Resized::Pending
    }
}

/// How far a starting Pod has got.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Progress {
    /// The computer's container runs.
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
            "the computer stopped ({}, exit code {}) {}",
            text(ended.get("reason")),
            ended.get("exitCode").and_then(Value::as_i64).unwrap_or(-1),
            text(ended.get("message"))
        ));
    }
    if matches!(phase, "Failed" | "Succeeded") {
        return Progress::Failed(format!(
            "the computer stopped ({phase}): {}",
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
            Progress::Failed(format!("the computer cannot start ({why})"))
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
