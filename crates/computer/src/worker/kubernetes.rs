//! Workers on Kubernetes: a Pod per worker in one namespace, from the environment's pinned
//! image, with CPU and memory requested and limited, a runtime class (gVisor or Kata) when one
//! is set, no service account token, no privilege, and a NetworkPolicy of its own. glave and
//! the worker's token are copied in through the API server once the Pod runs (an exec of
//! `tar`), so the token is never stored there; the Pod waits for them, then becomes glave,
//! serving on a Unix socket. The harness reaches it through the API server too, each
//! connection an exec of a bridge to that socket (`link`). When glave runs in a Pod beside its
//! workers, each worker names that Pod as its owner, so the cluster removes them when it is
//! deleted.

mod home;
mod link;
mod objects;
mod people;
mod sizing;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use glave_ports::BoxFuture;
use glave_ports::computer::{ComputerError, Isolation};
use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::api::networking::v1::NetworkPolicy;
use kube::api::{
    AttachParams, DeleteParams, ListParams, LogParams, Patch, PatchParams, PostParams,
};
use kube::client::UpgradeConnectionError;
use kube::config::{KubeConfigOptions, Kubeconfig};
use kube::{Api, Client, Config};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::backend::{
    Backend, COPIED_WITHIN, Link, Listed, STARTED_BY, Spec, Started, Stream, saying,
};
use super::docker::elf_machine;
use crate::token::Token;
use objects::{Home, Owner, Progress, Resized, Shape};
pub use people::{Holding, Leasing, People, person_sandbox};
use sizing::Size;

/// How long a Pod has to start running: long enough to pull a large image.
const START_WAIT: Duration = Duration::from_mins(5);
/// How often a starting Pod is looked at.
const POLL: Duration = Duration::from_millis(500);
/// Bytes sent to `tar` at a time while copying glave in.
const CHUNK: usize = 1 << 20;
/// How long an in-place resize has to take effect before a person's computer is restarted
/// bigger instead.
const RESIZED_WITHIN: Duration = Duration::from_mins(1);

/// Which cluster workers run on, and the glave that goes into them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KubernetesSettings {
    /// A kubeconfig file; when `None`, `KUBECONFIG`, `~/.kube/config`, or the service account
    /// of the Pod glave itself runs in.
    pub kubeconfig: Option<PathBuf>,
    /// The namespace workers run in; the kubeconfig's when `None`.
    pub namespace: Option<String>,
    /// The runtime class every worker runs under (gVisor's `runsc`, or Kata), for workers
    /// kept apart from the node by a kernel or a machine of their own.
    pub runtime_class: Option<String>,
    /// A Linux build of glave for the nodes' processor.
    pub glave: PathBuf,
}

/// A Kubernetes cluster as a place workers run.
#[derive(Clone)]
pub struct Kubernetes {
    client: Client,
    namespace: String,
    runtime_class: Option<String>,
    glave: PathBuf,
    /// Labels put on every object besides its own, such as a test run's.
    labels: BTreeMap<String, String>,
    /// The Pod glave runs in, when it runs in one in the workers' namespace: every worker names
    /// it as its owner, so the cluster removes them when it is deleted, however glave ended.
    owner: Option<Owner>,
}

impl std::fmt::Debug for Kubernetes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Kubernetes")
            .field("namespace", &self.namespace)
            .field("runtime_class", &self.runtime_class)
            .finish_non_exhaustive()
    }
}

fn failed(what: &str, error: impl std::fmt::Display) -> ComputerError {
    ComputerError::Failed(format!("Kubernetes {what}: {error}"))
}

/// Whether `error` is the API server saying the object is not there.
fn not_found(error: &kube::Error) -> bool {
    matches!(error, kube::Error::Api(status) if status.is_not_found())
}

/// An exec in the Pod `name` that failed with `error`, as a computer error: final when the API
/// server refused it, or the Pod is not there or not running (an exec into a container that
/// ended is answered as the API server's own error); unreachable, which a bridge tries again,
/// when the Pod runs, or cannot be read, as while the API server restarts.
async fn exec_failed(
    pods: &Api<Pod>,
    name: &str,
    what: &str,
    error: &kube::Error,
) -> ComputerError {
    let refused = match error {
        kube::Error::Api(status) => status.code < 500,
        kube::Error::UpgradeConnection(UpgradeConnectionError::ProtocolSwitch(code)) => {
            code.is_client_error()
        }
        _ => false,
    };
    if refused || matches!(running(pods, name).await, Ok(false)) {
        failed(what, error)
    } else {
        ComputerError::Unreachable(format!("Kubernetes {what}: {error}"))
    }
}

/// Whether the Pod `name` runs, as the API server says: there, not being removed, its container
/// running.
async fn running(pods: &Api<Pod>, name: &str) -> Result<bool, ComputerError> {
    let pod = pods.get_opt(name).await.map_err(|error| {
        ComputerError::Unreachable(format!("Kubernetes could not read {name}: {error}"))
    })?;
    Ok(pod.is_some_and(|pod| {
        pod.metadata.deletion_timestamp.is_none()
            && objects::progress(&serde_json::to_value(&pod).unwrap_or_default())
                == Progress::Running
    }))
}

impl Kubernetes {
    /// The cluster `settings` name, with a client signed in as the kubeconfig (or the service
    /// account) says.
    pub async fn connect(settings: KubernetesSettings) -> Result<Self, ComputerError> {
        let config = match &settings.kubeconfig {
            Some(path) => {
                let text = tokio::fs::read_to_string(path).await.map_err(|error| {
                    ComputerError::Refused(
                        format!("could not read the kubeconfig {}: {error}", path.display()).into(),
                    )
                })?;
                let kubeconfig = Kubeconfig::from_yaml(&text).map_err(|error| {
                    ComputerError::Refused(
                        format!("{} is not a kubeconfig: {error}", path.display()).into(),
                    )
                })?;
                Config::from_custom_kubeconfig(kubeconfig, &KubeConfigOptions::default())
                    .await
                    .map_err(|error| {
                        ComputerError::Refused(
                            format!("the kubeconfig {} cannot be used: {error}", path.display())
                                .into(),
                        )
                    })?
            }
            None => Config::infer().await.map_err(|error| {
                ComputerError::Refused(
                    format!(
                        "no Kubernetes cluster found (KUBECONFIG, ~/.kube/config, or a service \
                     account): {error}"
                    )
                    .into(),
                )
            })?,
        };
        let namespace = settings
            .namespace
            .clone()
            .unwrap_or_else(|| config.default_namespace.clone());
        let client = Client::try_from(config).map_err(|error| failed("client", error))?;
        let owner = this_pod(&Api::namespaced(client.clone(), &namespace), &namespace).await;
        Ok(Self {
            client,
            namespace,
            runtime_class: settings.runtime_class,
            glave: settings.glave,
            labels: BTreeMap::new(),
            owner,
        })
    }

    /// Names `owner` as every worker's owner, for the test that deletes one.
    #[cfg(test)]
    pub(crate) fn owned_by(mut self, owner: Owner) -> Self {
        self.owner = Some(owner);
        self
    }

    /// Puts `labels` on every object a worker is made of too, such as a test run's.
    #[must_use]
    pub fn with_labels(mut self, labels: BTreeMap<String, String>) -> Self {
        self.labels = labels;
        self
    }

    fn pods(&self) -> Api<Pod> {
        Api::namespaced(self.client.clone(), &self.namespace)
    }

    fn policies(&self) -> Api<NetworkPolicy> {
        Api::namespaced(self.client.clone(), &self.namespace)
    }

    async fn glave(&self) -> Result<Vec<u8>, ComputerError> {
        tokio::fs::read(&self.glave).await.map_err(|error| {
            ComputerError::Refused(
                format!(
                    "could not read the glave for workers at {}: {error}",
                    self.glave.display()
                )
                .into(),
            )
        })
    }

    /// Starts the worker `name` from `spec`, with `glave` as the program it serves with.
    pub(crate) async fn start_with(
        &self,
        name: &str,
        spec: &Spec,
        glave: &[u8],
    ) -> Result<Link, ComputerError> {
        self.start_on(name, spec, glave, None).await
    }

    /// Starts `name` from `spec` with `glave`: a worker, or with `home` a person's computer on
    /// their disk, owned by no Pod, since a person's computer outlives any one glave.
    async fn start_on(
        &self,
        name: &str,
        spec: &Spec,
        glave: &[u8],
        home: Option<Home<'_>>,
    ) -> Result<Link, ComputerError> {
        let arch = objects::arch(elf_machine(glave));
        let shape = Shape {
            name,
            spec,
            runtime_class: self.runtime_class.as_deref(),
            arch,
            labels: &self.labels,
            owner: if home.is_some() {
                None
            } else {
                self.owner.as_ref()
            },
            home,
        };
        // The policy first, so the Pod never runs without it.
        let policy: NetworkPolicy = serde_json::from_value(shape.network_policy())
            .map_err(|error| failed("network policy", error))?;
        self.policies()
            .create(&PostParams::default(), &policy)
            .await
            .map_err(|error| failed("refused the worker's network policy", error))?;
        let pod: Pod = serde_json::from_value(shape.pod()).map_err(|error| failed("pod", error))?;
        let pods = self.pods();
        pods.create(&PostParams::default(), &pod)
            .await
            .map_err(|error| failed("refused the worker", error))?;
        self.running(name).await?;
        let files = objects::files(glave, spec).map_err(|long| {
            ComputerError::Failed(format!("{} is too long a name for tar", long.0))
        })?;
        self.copy_in(name, &files).await?;
        Ok(Link::Bridged)
    }

    /// Waits for the worker's container to run.
    async fn running(&self, name: &str) -> Result<(), ComputerError> {
        let pods = self.pods();
        let mut waited = Duration::ZERO;
        loop {
            let pod = pods
                .get(name)
                .await
                .map_err(|error| failed("could not read the worker", error))?;
            let pod = serde_json::to_value(&pod).unwrap_or(Value::Null);
            match objects::progress(&pod) {
                Progress::Running => return Ok(()),
                Progress::Failed(why) => return Err(ComputerError::Failed(why)),
                Progress::Waiting(why) if waited >= START_WAIT => {
                    return Err(ComputerError::Failed(format!(
                        "the worker did not start in {} minutes: {why}",
                        START_WAIT.as_secs() / 60
                    )));
                }
                Progress::Waiting(_) => {
                    tokio::time::sleep(POLL).await;
                    waited = waited.saturating_add(POLL);
                }
            }
        }
    }

    /// Copies `files` (a tar archive) into the running worker through the API server, within
    /// [`COPIED_WITHIN`].
    async fn copy_in(&self, name: &str, files: &[u8]) -> Result<(), ComputerError> {
        tokio::time::timeout(COPIED_WITHIN, self.copying(name, files))
            .await
            .unwrap_or_else(|_| {
                Err(ComputerError::Failed(format!(
                    "copying glave into the worker did not finish within {}",
                    glave_ports::spoken(COPIED_WITHIN)
                )))
            })
    }

    async fn copying(&self, name: &str, files: &[u8]) -> Result<(), ComputerError> {
        let params = AttachParams::default()
            .container(objects::CONTAINER)
            .stdin(true)
            .stdout(false)
            .stderr(true)
            .max_stdin_buf_size(CHUNK);
        let pods = self.pods();
        let mut process = match pods
            .exec(name, ["tar", "-x", "-f", "-", "-C", "/"], &params)
            .await
        {
            Ok(process) => process,
            Err(error) => {
                let what = "could not copy glave into the worker";
                return Err(exec_failed(&pods, name, what, &error).await);
            }
        };
        let (Some(mut input), Some(mut errors)) = (process.stdin(), process.stderr()) else {
            return Err(ComputerError::Failed(
                "copying glave in gave no standard input".into(),
            ));
        };
        let status = process.take_status();
        let write = async move {
            for chunk in files.chunks(CHUNK) {
                input.write_all(chunk).await?;
            }
            input.shutdown().await
        };
        let read = async move {
            let mut said = String::new();
            let _ = errors.read_to_string(&mut said).await;
            said
        };
        let (written, said) = tokio::join!(write, read);
        let ended = match status {
            Some(status) => status.await,
            None => None,
        };
        let _ = process.join().await;
        written.map_err(|error| failed("could not copy glave into the worker", error))?;
        match ended {
            // A cluster too old to close standard input on its own says nothing; the wait in
            // the worker then tells whether everything arrived.
            None => Ok(()),
            Some(status) if status.status.as_deref() == Some("Success") => Ok(()),
            Some(status) => Err(ComputerError::Failed(format!(
                "copying glave into the worker failed (the image needs tar): {} {}",
                status.message.unwrap_or_default(),
                said.trim()
            ))),
        }
    }

    /// Resizes the running Pod `name` in place to `size`, CPU and memory requested and limited
    /// alike, through its `resize` subresource, and waits, within [`RESIZED_WITHIN`], until its
    /// container runs at it. A cluster that cannot, or a node with no room, refuses it.
    async fn resize(&self, name: &str, size: Size) -> Result<(), ComputerError> {
        let amounts = json!({
            "cpu": objects::cpu(size.cpu_millis),
            "memory": objects::memory(size.memory_mib),
        });
        let patch = json!({"spec": {"containers": [{
            "name": objects::CONTAINER,
            "resources": {"requests": amounts, "limits": amounts},
        }]}});
        let pods = self.pods();
        pods.patch_subresource(
            "resize",
            name,
            &PatchParams::default(),
            &Patch::Strategic(&patch),
        )
        .await
        .map_err(|error| failed("refused to resize the computer", error))?;
        tokio::time::timeout(RESIZED_WITHIN, self.resized(name, &amounts))
            .await
            .unwrap_or_else(|_| {
                Err(ComputerError::Failed(format!(
                    "the computer was not resized within {}",
                    glave_ports::spoken(RESIZED_WITHIN)
                )))
            })
    }

    /// Waits until the Pod `name`'s container runs at `amounts`, or its resize is refused.
    async fn resized(&self, name: &str, amounts: &Value) -> Result<(), ComputerError> {
        let pods = self.pods();
        loop {
            let pod = pods
                .get(name)
                .await
                .map_err(|error| failed("could not read the computer", error))?;
            let pod = serde_json::to_value(&pod).unwrap_or(Value::Null);
            match objects::resized(&pod, amounts) {
                Resized::Done => return Ok(()),
                Resized::Refused(why) => return Err(ComputerError::Failed(why)),
                Resized::Pending => tokio::time::sleep(POLL).await,
            }
        }
    }

    /// The last lines a worker printed, for saying why it did not start.
    pub(crate) async fn logs(&self, name: &str) -> String {
        let params = LogParams {
            container: Some(objects::CONTAINER.into()),
            tail_lines: Some(20),
            ..LogParams::default()
        };
        self.pods().logs(name, &params).await.unwrap_or_default()
    }
}

/// The namespace of the service account a Pod runs as, in every container of one.
const POD_NAMESPACE: &str = "/var/run/secrets/kubernetes.io/serviceaccount/namespace";

/// The Pod this glave runs in, when it runs in one in `namespace`: by its name, which is its
/// host's name, and its uid as the API server says. None anywhere else, or when it cannot be
/// read: workers are then still removed by the next glave that starts in the same Pod.
async fn this_pod(pods: &Api<Pod>, namespace: &str) -> Option<Owner> {
    let own = tokio::fs::read_to_string(POD_NAMESPACE).await.ok()?;
    if own.trim() != namespace {
        return None;
    }
    let host = tokio::fs::read_to_string("/etc/hostname").await.ok()?;
    let name = host.trim().to_owned();
    let pod = pods.get_opt(&name).await.ok()??;
    Some(Owner {
        uid: pod.metadata.uid?,
        name,
    })
}

impl Backend for Kubernetes {
    fn isolation(&self) -> Isolation {
        if self.runtime_class.is_some() {
            Isolation::Vm
        } else {
            Isolation::Container
        }
    }

    fn start<'a>(&'a self, spec: &'a Spec) -> BoxFuture<'a, Result<Started, ComputerError>> {
        Box::pin(async move {
            let name = format!("glave-worker-{}", crate::local::new_id()?.simple());
            let started = match self.glave().await {
                Ok(glave) => self.start_with(&name, spec, &glave).await,
                Err(error) => Err(error),
            };
            match started {
                Ok(link) => Ok(Started { name, link }),
                Err(error) => {
                    let said = self.logs(&name).await;
                    let _ = self.remove(&name).await;
                    Err(saying(error, &said))
                }
            }
        })
    }

    fn renew<'a>(
        &'a self,
        name: &'a str,
        token: &'a Token,
    ) -> BoxFuture<'a, Result<(), ComputerError>> {
        Box::pin(async move {
            let file = super::tar::token_file(objects::TOKEN, token)?;
            self.copy_in(name, &file).await
        })
    }

    fn bridge<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Stream, ComputerError>> {
        Box::pin(link::bridge(self.pods(), name))
    }

    fn list(&self) -> BoxFuture<'_, Result<Vec<Listed>, ComputerError>> {
        Box::pin(async move {
            let selector: Vec<String> = std::iter::once(objects::LABEL.to_owned())
                .chain(
                    self.labels
                        .iter()
                        .map(|(key, value)| format!("{key}={value}")),
                )
                .collect();
            let params = ListParams::default().labels(&selector.join(","));
            let pods = self
                .pods()
                .list(&params)
                .await
                .map_err(|error| failed("could not list the workers", error))?;
            let policies =
                self.policies().list(&params).await.map_err(|error| {
                    failed("could not list the workers' network policies", error)
                })?;
            // A policy without its Pod is what a glave that ended between the two left.
            let mut listed = BTreeMap::new();
            let metadata = pods
                .items
                .into_iter()
                .map(|pod| pod.metadata)
                .chain(policies.items.into_iter().map(|policy| policy.metadata));
            for metadata in metadata {
                let Some(name) = metadata.name else {
                    continue;
                };
                let started_by = metadata
                    .annotations
                    .and_then(|mut annotations| annotations.remove(STARTED_BY));
                listed.entry(name).or_insert(started_by);
            }
            Ok(listed
                .into_iter()
                .map(|(name, started_by)| Listed { name, started_by })
                .collect())
        })
    }

    fn remove<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<(), ComputerError>> {
        Box::pin(async move {
            let now = DeleteParams {
                grace_period_seconds: Some(0),
                ..DeleteParams::default()
            };
            let pod = self.pods().delete(name, &now).await;
            let policy = self.policies().delete(name, &now).await;
            for (what, result) in [("worker", pod.err()), ("network policy", policy.err())] {
                if let Some(error) = result
                    && !not_found(&error)
                {
                    return Err(failed(
                        &format!("could not remove the {what} {name}"),
                        error,
                    ));
                }
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests;
