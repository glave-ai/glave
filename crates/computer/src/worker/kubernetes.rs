//! Workers on Kubernetes: a Pod per worker in one namespace, from the environment's pinned
//! image, with CPU and memory requested and limited, a runtime class (gVisor or Kata) when one
//! is set, no service account token, no privilege, and a NetworkPolicy of its own. glave and
//! the worker's TLS identity are copied in through the API server once the Pod runs (an exec
//! of `tar`), so the key and token are never stored there; the Pod waits for them, then
//! becomes glave. The harness reaches it through the API server too (`link`).

mod link;
mod objects;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use glave_ports::BoxFuture;
use glave_ports::computer::{ComputerError, Isolation};
use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::api::networking::v1::NetworkPolicy;
use kube::api::{AttachParams, DeleteParams, LogParams, PostParams};
use kube::config::{KubeConfigOptions, Kubeconfig};
use kube::{Api, Client, Config};
use serde_json::Value;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::backend::{Backend, Link, Spec, Started};
use super::docker::elf_machine;
use crate::token::Token;
use objects::{Progress, Shape};

/// How long a Pod has to start running: long enough to pull a large image.
const START_WAIT: Duration = Duration::from_mins(5);
/// How often a starting Pod is looked at.
const POLL: Duration = Duration::from_millis(500);
/// Bytes sent to `tar` at a time while copying glave in.
const CHUNK: usize = 1 << 20;
/// How long copying glave into a running worker has: seconds through a cluster's API server,
/// a minute or two over a slow link to it.
const COPIED_WITHIN: Duration = Duration::from_mins(5);

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

impl Kubernetes {
    /// The cluster `settings` name, with a client signed in as the kubeconfig (or the service
    /// account) says.
    pub async fn connect(settings: KubernetesSettings) -> Result<Self, ComputerError> {
        let config = match &settings.kubeconfig {
            Some(path) => {
                let text = tokio::fs::read_to_string(path).await.map_err(|error| {
                    ComputerError::Refused(format!(
                        "could not read the kubeconfig {}: {error}",
                        path.display()
                    ))
                })?;
                let kubeconfig = Kubeconfig::from_yaml(&text).map_err(|error| {
                    ComputerError::Refused(format!(
                        "{} is not a kubeconfig: {error}",
                        path.display()
                    ))
                })?;
                Config::from_custom_kubeconfig(kubeconfig, &KubeConfigOptions::default())
                    .await
                    .map_err(|error| {
                        ComputerError::Refused(format!(
                            "the kubeconfig {} cannot be used: {error}",
                            path.display()
                        ))
                    })?
            }
            None => Config::infer().await.map_err(|error| {
                ComputerError::Refused(format!(
                    "no Kubernetes cluster found (KUBECONFIG, ~/.kube/config, or a service \
                     account): {error}"
                ))
            })?,
        };
        let namespace = settings
            .namespace
            .clone()
            .unwrap_or_else(|| config.default_namespace.clone());
        let client = Client::try_from(config).map_err(|error| failed("client", error))?;
        Ok(Self {
            client,
            namespace,
            runtime_class: settings.runtime_class,
            glave: settings.glave,
            labels: BTreeMap::new(),
        })
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
            ComputerError::Refused(format!(
                "could not read the glave for workers at {}: {error}",
                self.glave.display()
            ))
        })
    }

    /// Starts the worker `name` from `spec`, with `glave` as the program it serves with.
    pub(crate) async fn start_with(
        &self,
        name: &str,
        spec: &Spec,
        glave: &[u8],
    ) -> Result<Link, ComputerError> {
        let arch = objects::arch(elf_machine(glave));
        let shape = Shape {
            name,
            spec,
            runtime_class: self.runtime_class.as_deref(),
            arch,
            labels: &self.labels,
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
        if spec.network {
            link::forwarded(pods, name, spec.identity.fingerprint.clone()).await
        } else {
            link::attached(&pods, name).await
        }
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
        let mut process = self
            .pods()
            .exec(name, ["tar", "-x", "-f", "-", "-C", "/"], &params)
            .await
            .map_err(|error| failed("could not copy glave into the worker", error))?;
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
                    if said.trim().is_empty() {
                        Err(error)
                    } else {
                        Err(ComputerError::Failed(format!(
                            "{}; it said: {}",
                            error.message(),
                            said.trim()
                        )))
                    }
                }
            }
        })
    }

    fn renew<'a>(
        &'a self,
        name: &'a str,
        token: &'a Token,
    ) -> BoxFuture<'a, Result<(), ComputerError>> {
        Box::pin(async move { self.copy_in(name, &super::tar::token_file(token)?).await })
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
