//! Whether a person's computer's Pod has served (`specs/computers.md`, decision 18): the server
//! that wakes it marks it once its glave answered, with the protocol it speaks, so other servers
//! reach only a computer that finished waking, never one whose waking server is still giving it
//! its token.

use glave_ports::computer::ComputerError;
use k8s_openapi::api::core::v1::Pod;
use kube::Api;
use kube::api::{Patch, PatchParams};
use serde_json::json;

use super::failed;
use super::objects::{self, Progress};
use crate::wire;

/// The annotation a person's computer's Pod is marked served with, naming the protocol its
/// glave speaks.
const SERVED: &str = "glave.computer/served";

/// A person's computer's Pod as the API server shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Seen {
    /// None there, or being removed, or ended (or stuck for good).
    Gone,
    /// Made and not served yet: being woken, or left so by a server that went as it woke it.
    Waking,
    /// Its glave served the server that woke it, speaking `protocol`: the Pod `uid`.
    Served { uid: String, protocol: String },
}

impl Seen {
    /// Whether it is served, speaking the protocol this glave speaks.
    pub(crate) fn speaks_ours(&self) -> Option<&str> {
        match self {
            Self::Served { uid, protocol } if protocol == wire::PROTOCOL => Some(uid),
            _ => None,
        }
    }
}

/// The Pod `name` as the API server shows it.
pub(super) async fn seen(pods: &Api<Pod>, name: &str) -> Result<Seen, ComputerError> {
    let pod = pods.get_opt(name).await.map_err(|error| {
        ComputerError::Unreachable(format!("Kubernetes could not read {name}: {error}"))
    })?;
    let Some(pod) = pod else {
        return Ok(Seen::Gone);
    };
    if pod.metadata.deletion_timestamp.is_some() {
        return Ok(Seen::Gone);
    }
    let served = pod
        .metadata
        .annotations
        .as_ref()
        .and_then(|annotations| annotations.get(SERVED))
        .cloned();
    let progress = objects::progress(&serde_json::to_value(&pod).unwrap_or_default());
    Ok(match (progress, served, pod.metadata.uid) {
        (Progress::Failed(_), ..) => Seen::Gone,
        (Progress::Running, Some(protocol), Some(uid)) => Seen::Served { uid, protocol },
        _ => Seen::Waking,
    })
}

/// Marks the Pod `name`, the very one `uid` names, served with the protocol this glave speaks.
pub(super) async fn mark(pods: &Api<Pod>, name: &str, uid: &str) -> Result<(), ComputerError> {
    // The uid, unchanged, makes the patch fail on any other Pod of that name.
    let patch = json!({"metadata": {"uid": uid, "annotations": {SERVED: wire::PROTOCOL}}});
    pods.patch(name, &PatchParams::default(), &Patch::Merge(&patch))
        .await
        .map_err(|error| failed("could not mark the computer as serving", error))?;
    Ok(())
}
