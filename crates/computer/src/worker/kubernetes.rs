//! People's computers on Kubernetes: while awake, a Pod per person in one namespace, from the
//! pinned image, with CPU and memory requested and limited, a runtime class (gVisor or Kata)
//! when one is set, no service account token, no privilege, and a NetworkPolicy of its own.
//! The computer's token, and glave unless its image carries the very glave chosen, are copied
//! in through the API server once the Pod runs (an exec of `tar`), so the token is never
//! stored there. The Pod waits for them, then becomes glave, serving on a Unix socket. The
//! harness reaches it through the API server too, each connection an exec of a bridge to that
//! socket (`link`).

mod home;
mod link;
mod objects;
mod people;
mod served;
mod sizing;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use glave_ports::computer::{ComputerError, Isolation};
use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::api::networking::v1::NetworkPolicy;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Status;
use kube::api::{AttachParams, LogParams, Patch, PatchParams, PostParams};
use kube::client::UpgradeConnectionError;
use kube::config::{KubeConfigOptions, Kubeconfig};
use kube::{Api, Client, Config};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use crate::token::Token;
pub(crate) use home::PersonComputer;
use objects::{Home, Progress, Resized, Shape};
pub use people::{Holding, Leasing, People, person_sandbox};
use sizing::Size;

/// How long copying glave into a started computer may take: seconds through a cluster's API
/// server, a minute or two over a slow link to one.
const COPIED_WITHIN: Duration = Duration::from_mins(5);
/// How long a Pod has to start running: long enough to pull a large image.
const START_WAIT: Duration = Duration::from_mins(5);
/// How often a starting Pod is looked at.
const POLL: Duration = Duration::from_millis(500);
/// Bytes sent to `tar` at a time while copying glave in.
const CHUNK: usize = 1 << 20;
/// How long an in-place resize has to take effect before a person's computer is restarted
/// bigger instead.
const RESIZED_WITHIN: Duration = Duration::from_mins(1);

/// How a command fed through the API server ended, when the cluster says, and what it printed
/// on standard error.
type Fed = (Option<Status>, String);

/// A stream to glave in a computer: what it reads, and what it writes.
pub(crate) type Stream = (
    Box<dyn tokio::io::AsyncRead + Unpin + Send>,
    Box<dyn tokio::io::AsyncWrite + Unpin + Send>,
);

/// What one computer's Pod is started with: everything already checked against policy.
#[derive(Debug, Clone)]
pub(crate) struct Spec {
    /// An OCI image reference pinned by digest.
    pub(crate) image: String,
    /// Variables every command sees; never secrets.
    pub(crate) env: BTreeMap<String, String>,
    /// CPU, in thousandths of one.
    pub(crate) cpu_millis: Option<u32>,
    /// Memory, in MiB.
    pub(crate) memory_mib: Option<u32>,
    /// Whether commands may reach the network at all.
    pub(crate) network: bool,
    /// The short-lived token the harness presents first.
    pub(crate) token: Token,
}

/// Which cluster people's computers run on, and the glave that goes into them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KubernetesSettings {
    /// A kubeconfig file; when `None`, `KUBECONFIG`, `~/.kube/config`, or the service account
    /// of the Pod glave itself runs in.
    pub kubeconfig: Option<PathBuf>,
    /// The namespace people's computers run in; the kubeconfig's when `None`.
    pub namespace: Option<String>,
    /// The runtime class every computer runs under (gVisor's `runsc`, or Kata), for computers
    /// kept apart from the node by a kernel or a machine of their own.
    pub runtime_class: Option<String>,
    /// A Linux build of glave for the nodes' processor.
    pub glave: PathBuf,
}

/// A Kubernetes cluster as a place people's computers run.
#[derive(Clone)]
pub struct Kubernetes {
    client: Client,
    namespace: String,
    runtime_class: Option<String>,
    glave: PathBuf,
    /// Labels put on every object besides its own, such as a test run's.
    labels: BTreeMap<String, String>,
    /// The SHA-256 of the glave for computers, kept from the last time it was hashed with the
    /// file as it was then, shared by every copy: hashed again only once the file changes.
    hashed: Arc<Mutex<Option<(Stamp, String)>>>,
}

/// A file as it was: its path, its size and when it last changed.
type Stamp = (PathBuf, u64, Option<SystemTime>);

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
    running_as(pods, name).await.map(|uid| uid.is_some())
}

/// The uid of the Pod `name` while it runs, as [`running`] says it does: which Pod of that name
/// it is, since a computer woken again is a new Pod under the same name.
async fn running_as(pods: &Api<Pod>, name: &str) -> Result<Option<String>, ComputerError> {
    let pod = pods.get_opt(name).await.map_err(|error| {
        ComputerError::Unreachable(format!("Kubernetes could not read {name}: {error}"))
    })?;
    Ok(pod.and_then(|pod| {
        let runs = pod.metadata.deletion_timestamp.is_none()
            && objects::progress(&serde_json::to_value(&pod).unwrap_or_default())
                == Progress::Running;
        if runs { pod.metadata.uid } else { None }
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
        Ok(Self {
            client,
            namespace,
            runtime_class: settings.runtime_class,
            glave: settings.glave,
            labels: BTreeMap::new(),
            hashed: Arc::default(),
        })
    }

    /// Puts `labels` on every object a computer is made of too, such as a test run's.
    #[must_use]
    pub fn with_labels(mut self, labels: BTreeMap<String, String>) -> Self {
        self.labels = labels;
        self
    }

    /// How computers here are kept apart from the machine they run on.
    pub(crate) fn isolation(&self) -> Isolation {
        if self.runtime_class.is_some() {
            Isolation::Vm
        } else {
            Isolation::Container
        }
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
                    "could not read the glave for people's computers at {}: {error}",
                    self.glave.display()
                )
                .into(),
            )
        })
    }

    /// The glave for computers and its SHA-256, which is hashed only the first time and when
    /// the file has changed since: a release's glave is tens of MB.
    async fn glave_and_digest(&self) -> Result<(Vec<u8>, String), ComputerError> {
        let stamp = tokio::fs::metadata(&self.glave)
            .await
            .ok()
            .map(|file| (self.glave.clone(), file.len(), file.modified().ok()));
        let glave = self.glave().await?;
        let kept = self.hashed.lock().ok().and_then(|hashed| hashed.clone());
        if let (Some(stamp), Some((was, digest))) = (&stamp, kept)
            && *stamp == was
        {
            return Ok((glave, digest));
        }
        // Off the runtime's threads.
        let (glave, digest) = tokio::task::spawn_blocking(move || {
            let digest = glave_ports::digest(&glave).as_str().to_owned();
            (glave, digest)
        })
        .await
        .map_err(|error| failed("could not hash the glave for computers", error))?;
        if let (Some(stamp), Ok(mut hashed)) = (stamp, self.hashed.lock()) {
            *hashed = Some((stamp, digest.clone()));
        }
        Ok((glave, digest))
    }

    /// Starts the person's computer `name` on their disk (`home`) from `spec`, with `glave`
    /// copied in unless its image carries that very glave. It is owned by no Pod, since a
    /// person's computer outlives any one glave. The uid of the Pod started.
    async fn start(
        &self,
        name: &str,
        spec: &Spec,
        glave: &[u8],
        home: Home<'_>,
    ) -> Result<String, ComputerError> {
        let arch = objects::arch(elf_machine(glave));
        let shape = Shape {
            name,
            spec,
            runtime_class: self.runtime_class.as_deref(),
            arch,
            labels: &self.labels,
            home,
        };
        // The policy first, so the Pod never runs without it.
        let policy: NetworkPolicy = serde_json::from_value(shape.network_policy())
            .map_err(|error| failed("network policy", error))?;
        self.policies()
            .create(&PostParams::default(), &policy)
            .await
            .map_err(|error| failed("refused the computer's network policy", error))?;
        let pod: Pod = serde_json::from_value(shape.pod()).map_err(|error| failed("pod", error))?;
        let pods = self.pods();
        let made = pods
            .create(&PostParams::default(), &pod)
            .await
            .map_err(|error| failed("refused the computer", error))?;
        let uid = made.metadata.uid.ok_or_else(|| {
            ComputerError::Failed(format!("the computer {name} was made with no uid"))
        })?;
        self.running(name).await?;
        if self.serves_its_image_glave(name, spec, &home).await? {
            return Ok(uid);
        }
        let files = objects::files(glave, spec).map_err(|long| {
            ComputerError::Failed(format!("{} is too long a name for tar", long.0))
        })?;
        self.copy_in(name, &files, "glave").await?;
        Ok(uid)
    }

    /// Waits for the computer's container to run.
    async fn running(&self, name: &str) -> Result<(), ComputerError> {
        let pods = self.pods();
        let mut waited = Duration::ZERO;
        loop {
            let pod = pods
                .get(name)
                .await
                .map_err(|error| failed("could not read the computer", error))?;
            let pod = serde_json::to_value(&pod).unwrap_or(Value::Null);
            match objects::progress(&pod) {
                Progress::Running => return Ok(()),
                Progress::Failed(why) => return Err(ComputerError::Failed(why)),
                Progress::Waiting(why) if waited >= START_WAIT => {
                    return Err(ComputerError::Failed(format!(
                        "the computer did not start in {} minutes: {why}",
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

    /// Puts the token into the person's computer `name` and has it serve with its image's
    /// glave, when that is the glave the server chose: whether it was, so nothing need be
    /// copied in.
    async fn serves_its_image_glave(
        &self,
        name: &str,
        spec: &Spec,
        home: &Home<'_>,
    ) -> Result<bool, ComputerError> {
        let token = super::tar::token_file(objects::TOKEN, &spec.token)?;
        let (ended, _) = self
            .fed(name, &home.image_glave(), &token, "the token")
            .await?;
        Ok(ended.is_some_and(|status| status.status.as_deref() == Some("Success")))
    }

    /// Copies `files` (a tar archive of `what`) into the running computer through the API
    /// server.
    async fn copy_in(&self, name: &str, files: &[u8], what: &str) -> Result<(), ComputerError> {
        let (ended, said) = self
            .fed(name, &["tar", "-x", "-f", "-", "-C", "/"], files, what)
            .await?;
        match ended {
            // A cluster too old to close standard input on its own says nothing; the wait in
            // the computer then tells whether everything arrived.
            None => Ok(()),
            Some(status) if status.status.as_deref() == Some("Success") => Ok(()),
            Some(status) => Err(ComputerError::Failed(format!(
                "copying {what} into the computer failed (the image needs tar): {} {}",
                status.message.unwrap_or_default(),
                said.trim()
            ))),
        }
    }

    /// Gives the running computer `name` a fresh `token`, replacing the one it was started
    /// with, so a connection that died can be opened again after the first token has expired.
    async fn renew(&self, name: &str, token: &Token) -> Result<(), ComputerError> {
        let file = super::tar::token_file(objects::TOKEN, token)?;
        self.copy_in(name, &file, "the token").await
    }

    /// Runs `command` in the running computer `name` through the API server with `input`,
    /// which is `what`, on its standard input, within [`COPIED_WITHIN`]: how it ended, when the
    /// cluster says, and what it printed on standard error.
    async fn fed(
        &self,
        name: &str,
        command: &[&str],
        input: &[u8],
        what: &str,
    ) -> Result<Fed, ComputerError> {
        tokio::time::timeout(COPIED_WITHIN, self.feeding(name, command, input, what))
            .await
            .unwrap_or_else(|_| {
                Err(ComputerError::Failed(format!(
                    "copying {what} into the computer did not finish within {}",
                    glave_ports::spoken(COPIED_WITHIN)
                )))
            })
    }

    async fn feeding(
        &self,
        name: &str,
        command: &[&str],
        input: &[u8],
        what: &str,
    ) -> Result<Fed, ComputerError> {
        let could_not = format!("could not copy {what} into the computer");
        let params = AttachParams::default()
            .container(objects::CONTAINER)
            .stdin(true)
            .stdout(false)
            .stderr(true)
            .max_stdin_buf_size(CHUNK);
        let pods = self.pods();
        let mut process = match pods.exec(name, command.iter().copied(), &params).await {
            Ok(process) => process,
            Err(error) => return Err(exec_failed(&pods, name, &could_not, &error).await),
        };
        let (Some(mut stdin), Some(mut errors)) = (process.stdin(), process.stderr()) else {
            return Err(ComputerError::Failed(format!(
                "copying {what} in gave no standard input"
            )));
        };
        let status = process.take_status();
        let write = async move {
            for chunk in input.chunks(CHUNK) {
                stdin.write_all(chunk).await?;
            }
            stdin.shutdown().await
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
        written.map_err(|error| failed(&could_not, error))?;
        Ok((ended, said))
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

    /// A new bridge into the running computer `name`: a stream to glave there, a connection of
    /// its own each time.
    async fn bridge(&self, name: &str) -> Result<Stream, ComputerError> {
        link::bridge(self.pods(), name).await
    }

    /// The last lines a computer printed, for saying why it did not start.
    async fn logs(&self, name: &str) -> String {
        let params = LogParams {
            container: Some(objects::CONTAINER.into()),
            tail_lines: Some(20),
            ..LogParams::default()
        };
        self.pods().logs(name, &params).await.unwrap_or_default()
    }
}

/// The ELF machine of a 64-bit little-endian executable; `None` for anything else.
fn elf_machine(bytes: &[u8]) -> Option<u16> {
    let head = bytes.first_chunk::<20>()?;
    let [0x7f, b'E', b'L', b'F', 2, 1, .., low, high] = *head else {
        return None;
    };
    Some(u16::from_le_bytes([low, high]))
}

#[cfg(test)]
mod tests;
