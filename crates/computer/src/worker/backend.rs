//! What a worker backend does: start a disposable machine from a pinned image with glave in
//! it serving the computer protocol, say how to reach it, and remove it. Docker and Kubernetes
//! are the two; the trait stays inside the crate until a protocol for it is needed.

use std::collections::BTreeMap;
use std::fmt;
use std::net::SocketAddr;

use glave_ports::{BoxFuture, ComputerError};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::task::JoinHandle;

use super::limits::Isolation;
use crate::tls::{Fingerprint, Identity};
use crate::token::Token;

/// The port glave serves on inside a worker that has a network.
pub(crate) const PORT: u16 = 7070;

/// What one worker is started with: everything already checked against policy.
#[derive(Debug, Clone)]
pub(crate) struct Spec {
    /// An OCI image reference pinned by digest.
    pub(crate) image: String,
    /// Variables every command sees; never secrets.
    pub(crate) env: BTreeMap<String, String>,
    pub(crate) cpus: Option<u32>,
    pub(crate) memory_gb: Option<u32>,
    /// Whether commands may reach the network at all.
    pub(crate) network: bool,
    /// The worker's TLS identity, made for it by the harness side.
    pub(crate) identity: Identity,
    /// The short-lived token the harness presents first.
    pub(crate) token: Token,
}

/// How the harness reaches a started worker.
pub(crate) enum Link {
    /// Over TLS at `address`, trusting only the certificate with `fingerprint`. `forward`,
    /// when the backend reaches the worker through a forwarder of its own (Kubernetes'
    /// port-forward), moves the bytes, and stops when the worker is removed.
    Tls {
        address: SocketAddr,
        fingerprint: Fingerprint,
        forward: Option<JoinHandle<()>>,
    },
    /// Over a stream the backend opened to glave's standard input and output, for a worker
    /// with no network. `task` moves its bytes, and stops when the worker is removed.
    Stream {
        read: Box<dyn AsyncRead + Unpin + Send>,
        write: Box<dyn AsyncWrite + Unpin + Send>,
        task: JoinHandle<()>,
    },
}

impl fmt::Debug for Link {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tls {
                address,
                fingerprint,
                ..
            } => write!(f, "Tls({address}, {fingerprint:?})"),
            Self::Stream { .. } => f.write_str("Stream"),
        }
    }
}

/// A worker the backend started: its name there, and how to reach it.
#[derive(Debug)]
pub(crate) struct Started {
    pub(crate) name: String,
    pub(crate) link: Link,
}

/// A place workers run: Docker on this machine, or a Kubernetes cluster.
pub(crate) trait Backend: Send + Sync {
    /// How workers here are kept apart from the machine they run on.
    fn isolation(&self) -> Isolation {
        Isolation::Container
    }

    /// Starts a worker from `spec`, with glave serving in it. A worker that cannot start is
    /// removed before this returns.
    fn start<'a>(&'a self, spec: &'a Spec) -> BoxFuture<'a, Result<Started, ComputerError>>;

    /// Removes the worker named `name` and its disk. A worker already gone is not an error.
    fn remove<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<(), ComputerError>>;
}
