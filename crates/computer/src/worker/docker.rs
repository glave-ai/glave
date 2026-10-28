//! Workers on Docker: a container per worker from the environment's pinned image, with CPU,
//! memory and process limits, no new privileges, and either no network at all or the default
//! bridge with glave's port published on 127.0.0.1 only. Every worker's standard input is
//! attached and held by the glave that started it: when that glave ends, however it ends, Docker
//! closes it and the worker's glave ends, which stops the container (a later glave removes it).
//! glave goes in as a file before the container starts (a Linux build for the engine's
//! processor, from a path the person sets), with the TLS identity and token the harness side
//! made for this worker.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use bytes::Bytes;
use glave_ports::BoxFuture;
use glave_ports::computer::ComputerError;
use http::{Method, StatusCode};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::backend::{Backend, COPIED_WITHIN, Link, Listed, PORT, STARTED_BY, Spec, Started};
use super::engine::{ANSWERS_WITHIN, Engine, PULL_QUIET, Within, demultiplex, query};
use super::tar::{Entry, archive};
use crate::token::Token;

/// Where glave and its identity go inside a worker.
const HOME: &str = "/opt/glave";
/// The label every worker container carries, so leftovers can be found.
pub(super) const LABEL: &str = "glave.worker";
/// Bytes a stream link holds ahead of a slow reader.
const PIPE: usize = 256 * 1024;
/// The most processes one worker may run at once.
const PIDS: i64 = 4096;

/// How to reach Docker, and the glave that goes into workers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockerSettings {
    /// The Engine's Unix socket; found from `DOCKER_HOST` or the usual places when `None`.
    pub socket: Option<PathBuf>,
    /// A Linux build of glave for the engine's processor.
    pub glave: PathBuf,
}

/// Docker on this machine as a place workers run.
#[derive(Debug, Clone)]
pub struct Docker {
    engine: Engine,
    glave: PathBuf,
    /// Labels put on every container besides its own, such as a test run's.
    labels: BTreeMap<String, String>,
}

impl Docker {
    /// Docker as `settings` say, with `home` the person's home folder for finding the socket.
    pub fn new(settings: DockerSettings, home: Option<&Path>) -> Self {
        let socket = settings.socket.unwrap_or_else(|| socket(home));
        Self {
            engine: Engine::new(socket),
            glave: settings.glave,
            labels: BTreeMap::new(),
        }
    }

    /// Puts `labels` on every worker container too, such as a test run's, to find them by.
    #[must_use]
    pub fn with_labels(mut self, labels: BTreeMap<String, String>) -> Self {
        self.labels = labels;
        self
    }

    async fn ok(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, ComputerError> {
        let answer = self.engine.json(method, path, body).await?;
        if answer.status.is_success() {
            Ok(answer.json())
        } else {
            Err(ComputerError::Failed(format!(
                "Docker refused {path} ({}): {}",
                answer.status,
                answer.message()
            )))
        }
    }

    /// glave's bytes, checked to be a Linux build for the engine's processor.
    async fn glave_for_engine(&self) -> Result<Vec<u8>, ComputerError> {
        let version = self.ok(Method::GET, "/version", None).await?;
        let arch = version
            .get("Arch")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let bytes = tokio::fs::read(&self.glave).await.map_err(|error| {
            ComputerError::Refused(format!(
                "could not read the glave for workers at {}: {error}",
                self.glave.display()
            ))
        })?;
        let machine = elf_machine(&bytes);
        let fits = matches!(
            (arch, machine),
            ("arm64", Some(0xB7)) | ("amd64", Some(0x3E))
        );
        if fits {
            Ok(bytes)
        } else {
            Err(ComputerError::Refused(format!(
                "workers need a Linux build of glave for {arch}; {} is not one",
                self.glave.display()
            )))
        }
    }

    /// Pulls `image` unless the engine has it.
    async fn ensure_image(&self, image: &str) -> Result<(), ComputerError> {
        let have = self
            .engine
            .json(Method::GET, &format!("/images/{image}/json"), None)
            .await?;
        if have.status.is_success() {
            return Ok(());
        }
        let (name, digest) = image.rsplit_once('@').unwrap_or((image, ""));
        let path = format!(
            "/images/create?fromImage={}&tag={}",
            query(name),
            query(digest)
        );
        let pulled = self
            .engine
            .send(Method::POST, &path, None, Within::Each(PULL_QUIET))
            .await?;
        let failed = String::from_utf8_lossy(&pulled.body)
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .find_map(|line| line.get("error").and_then(Value::as_str).map(str::to_owned));
        match (pulled.status.is_success(), failed) {
            (true, None) => Ok(()),
            (_, failed) => Err(ComputerError::Failed(format!(
                "could not pull {image}: {}",
                failed.unwrap_or_else(|| pulled.message())
            ))),
        }
    }

    fn container(&self, name: &str, spec: &Spec) -> Value {
        let mut labels = self.labels.clone();
        labels.insert(LABEL.to_owned(), name.to_owned());
        if let Some(starter) = spec
            .starter
            .as_ref()
            .and_then(|starter| serde_json::to_string(starter).ok())
        {
            labels.insert(STARTED_BY.to_owned(), starter);
        }
        let command = if spec.network {
            json!([
                format!("{HOME}/glave"),
                "computer",
                "serve",
                "--tls",
                format!("0.0.0.0:{PORT}"),
                format!("{HOME}/tls"),
                "--until-input-ends"
            ])
        } else {
            json!([format!("{HOME}/glave"), "computer", "serve", "--stdio"])
        };
        let env: Vec<String> = spec
            .env
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect();
        let memory = spec
            .memory_gb
            .map(|gb| i64::from(gb).saturating_mul(1 << 30));
        let mut host = serde_json::Map::new();
        host.insert(
            "NetworkMode".into(),
            json!(if spec.network { "bridge" } else { "none" }),
        );
        host.insert("PidsLimit".into(), json!(PIDS));
        host.insert("SecurityOpt".into(), json!(["no-new-privileges"]));
        host.insert("CapDrop".into(), json!(["NET_RAW", "MKNOD", "AUDIT_WRITE"]));
        host.insert("Init".into(), json!(true));
        if let Some(cpus) = spec.cpus {
            host.insert(
                "NanoCpus".into(),
                json!(i64::from(cpus).saturating_mul(1_000_000_000)),
            );
        }
        if let Some(memory) = memory {
            host.insert("Memory".into(), json!(memory));
            host.insert("MemorySwap".into(), json!(memory));
        }
        let port = format!("{PORT}/tcp");
        if spec.network {
            host.insert(
                "PortBindings".into(),
                json!({ port.clone(): [{"HostIp": "127.0.0.1", "HostPort": ""}] }),
            );
        }
        json!({
            "Image": spec.image,
            "Cmd": command,
            "Env": env,
            "Labels": labels,
            "ExposedPorts": if spec.network { json!({ port: {} }) } else { json!({}) },
            // Every worker's standard input is held by the glave that started it, and closed
            // when that glave ends, however it ends: the worker's glave then ends too.
            "OpenStdin": true,
            "StdinOnce": true,
            "AttachStdin": true,
            "AttachStdout": !spec.network,
            "AttachStderr": !spec.network,
            "HostConfig": host,
        })
    }

    /// glave and the worker's identity, as a tar archive put at `/`.
    fn files(glave: &[u8], spec: &Spec) -> Result<Vec<u8>, ComputerError> {
        let token = spec.token.to_file();
        archive(&[
            Entry::Folder {
                name: "opt",
                mode: 0o755,
            },
            Entry::Folder {
                name: "opt/glave",
                mode: 0o755,
            },
            Entry::File {
                name: "opt/glave/glave",
                mode: 0o755,
                content: glave,
            },
            Entry::Folder {
                name: "opt/glave/tls",
                mode: 0o700,
            },
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
        ])
        .map_err(|long| ComputerError::Failed(format!("{} is too long a name for tar", long.0)))
    }

    async fn start_named(&self, name: &str, spec: &Spec) -> Result<Link, ComputerError> {
        let glave = self.glave_for_engine().await?;
        self.start_with(name, spec, &glave).await
    }

    /// Starts the worker `name` from `spec` with `glave` as the program it serves with.
    pub(crate) async fn start_with(
        &self,
        name: &str,
        spec: &Spec,
        glave: &[u8],
    ) -> Result<Link, ComputerError> {
        self.ensure_image(&spec.image).await?;
        let made = self
            .engine
            .json(
                Method::POST,
                &format!("/containers/create?name={}", query(name)),
                Some(&self.container(name, spec)),
            )
            .await?;
        if !made.status.is_success() {
            return Err(ComputerError::Failed(format!(
                "Docker could not make the worker: {}",
                made.message()
            )));
        }
        let files = Bytes::from(Self::files(glave, spec)?);
        let put = self
            .engine
            .send(
                Method::PUT,
                &format!("/containers/{name}/archive?path=/"),
                Some((files, "application/x-tar")),
                Within::All(COPIED_WITHIN),
            )
            .await?;
        if !put.status.is_success() {
            return Err(ComputerError::Failed(format!(
                "could not put glave into the worker: {}",
                put.message()
            )));
        }
        // Attached before it starts, so its input is held from its first moment.
        let output = u8::from(!spec.network);
        let path =
            format!("/containers/{name}/attach?stream=1&stdin=1&stdout={output}&stderr={output}");
        let attached = self.engine.upgrade(&path).await?;
        self.ok(Method::POST, &format!("/containers/{name}/start"), None)
            .await?;
        if spec.network {
            self.published(name, spec, held(attached)).await
        } else {
            Ok(stream(attached))
        }
    }

    /// Where the worker's published port is, on 127.0.0.1; `holding` holds its input.
    async fn published(
        &self,
        name: &str,
        spec: &Spec,
        holding: tokio::task::JoinHandle<()>,
    ) -> Result<Link, ComputerError> {
        let inspected = self
            .ok(Method::GET, &format!("/containers/{name}/json"), None)
            .await?;
        let port = inspected
            .pointer(&format!("/NetworkSettings/Ports/{PORT}~1tcp/0/HostPort"))
            .and_then(Value::as_str)
            .and_then(|port| port.parse::<u16>().ok())
            .ok_or_else(|| {
                ComputerError::Failed(format!(
                    "the worker has no published port: {}",
                    Self::why_stopped(&inspected)
                ))
            });
        let port = match port {
            Ok(port) => port,
            Err(error) => {
                holding.abort();
                return Err(error);
            }
        };
        Ok(Link::Tls {
            address: std::net::SocketAddr::from(([127, 0, 0, 1], port)),
            fingerprint: spec.identity.fingerprint.clone(),
            forward: Some(holding),
        })
    }

    fn why_stopped(inspected: &Value) -> String {
        let state = inspected.get("State").cloned().unwrap_or(Value::Null);
        format!(
            "it is {} (exit code {}) {}",
            state
                .get("Status")
                .and_then(Value::as_str)
                .unwrap_or("unknown"),
            state
                .get("ExitCode")
                .and_then(Value::as_i64)
                .unwrap_or_default(),
            state
                .get("Error")
                .and_then(Value::as_str)
                .unwrap_or_default()
        )
    }

    /// The last lines a worker printed, for saying why it did not start.
    pub(crate) async fn logs(&self, name: &str) -> String {
        let path = format!("/containers/{name}/logs?stdout=1&stderr=1&tail=20");
        match self
            .engine
            .send(Method::GET, &path, None, Within::All(ANSWERS_WITHIN))
            .await
        {
            Ok(answer) if answer.status == StatusCode::OK => {
                let (out, err) = demultiplex(&answer.body);
                format!(
                    "{}{}",
                    String::from_utf8_lossy(&out),
                    String::from_utf8_lossy(&err)
                )
            }
            _ => String::new(),
        }
    }
}

/// The ELF machine of a 64-bit little-endian executable; `None` for anything else.
pub(crate) fn elf_machine(bytes: &[u8]) -> Option<u16> {
    let head = bytes.first_chunk::<20>()?;
    let [0x7f, b'E', b'L', b'F', 2, 1, .., low, high] = *head else {
        return None;
    };
    Some(u16::from_le_bytes([low, high]))
}

/// The usual place of Docker's socket.
fn socket(home: Option<&Path>) -> PathBuf {
    if let Some(path) = std::env::var("DOCKER_HOST")
        .ok()
        .and_then(|host| host.strip_prefix("unix://").map(PathBuf::from))
    {
        return path;
    }
    let linux = PathBuf::from("/var/run/docker.sock");
    match home.map(|home| home.join(".docker/run/docker.sock")) {
        Some(desktop) if !linux.exists() && desktop.exists() => desktop,
        _ => linux,
    }
}

/// Holds a worker's input, attached with no output, until the worker ends or the task is
/// stopped: when the glave holding it ends, the connection closes and Docker closes the
/// worker's input.
fn held(
    attached: hyper_util::rt::TokioIo<hyper::upgrade::Upgraded>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut attached = attached;
        let mut read = [0_u8; 64];
        // Nothing comes back but the end, when the worker stops.
        while matches!(attached.read(&mut read).await, Ok(n) if n > 0) {}
    })
}

/// A stream link over an attached container: its multiplexed output split so glave's standard
/// output reaches the client, and the client's bytes go straight to its standard input.
fn stream(attached: hyper_util::rt::TokioIo<hyper::upgrade::Upgraded>) -> Link {
    let (mut from, to) = tokio::io::split(attached);
    let (mut ours, theirs) = tokio::io::duplex(PIPE);
    let task = tokio::spawn(async move {
        let mut head = [0u8; 8];
        while from.read_exact(&mut head).await.is_ok() {
            let [kind, _, _, _, a, b, c, d] = head;
            let length = u64::from(u32::from_be_bytes([a, b, c, d]));
            let mut frame = Vec::new();
            if (&mut from)
                .take(length)
                .read_to_end(&mut frame)
                .await
                .is_err()
            {
                break;
            }
            if kind == 1 && ours.write_all(&frame).await.is_err() {
                break;
            }
        }
    });
    Link::Stream {
        read: Box::new(theirs),
        write: Box::new(to),
        task,
    }
}

impl Backend for Docker {
    fn start<'a>(&'a self, spec: &'a Spec) -> BoxFuture<'a, Result<Started, ComputerError>> {
        Box::pin(async move {
            let name = format!("glave-worker-{}", crate::local::new_id()?.simple());
            match self.start_named(&name, spec).await {
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
        Box::pin(async move {
            let files = Bytes::from(super::tar::token_file(token)?);
            let put = self
                .engine
                .send(
                    Method::PUT,
                    &format!("/containers/{name}/archive?path=/"),
                    Some((files, "application/x-tar")),
                    Within::All(ANSWERS_WITHIN),
                )
                .await?;
            if put.status.is_success() {
                Ok(())
            } else {
                Err(ComputerError::Failed(format!(
                    "could not give the worker a new token: {}",
                    put.message()
                )))
            }
        })
    }

    fn list(&self) -> BoxFuture<'_, Result<Vec<Listed>, ComputerError>> {
        Box::pin(async move {
            let labels: Vec<String> = std::iter::once(LABEL.to_owned())
                .chain(
                    self.labels
                        .iter()
                        .map(|(key, value)| format!("{key}={value}")),
                )
                .collect();
            let filter = query(&json!({ "label": labels }).to_string());
            let found = self
                .ok(
                    Method::GET,
                    &format!("/containers/json?all=1&filters={filter}"),
                    None,
                )
                .await?;
            Ok(found
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|container| {
                    let labels = container.get("Labels")?;
                    Some(Listed {
                        name: labels.get(LABEL)?.as_str()?.to_owned(),
                        started_by: labels
                            .get(STARTED_BY)
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                    })
                })
                .collect())
        })
    }

    fn remove<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<(), ComputerError>> {
        Box::pin(async move {
            let answer = self
                .engine
                .json(
                    Method::DELETE,
                    &format!("/containers/{name}?force=1&v=1"),
                    None,
                )
                .await?;
            if answer.status.is_success() || answer.status == StatusCode::NOT_FOUND {
                Ok(())
            } else {
                Err(ComputerError::Failed(format!(
                    "Docker could not remove {name}: {}",
                    answer.message()
                )))
            }
        })
    }
}

#[cfg(test)]
mod tests;
