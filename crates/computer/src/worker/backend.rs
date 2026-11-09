//! What a worker backend does: start a machine from a pinned image with glave in it serving
//! the computer protocol, bridge into it, list its workers, and remove them. Kubernetes's
//! disposable workers and a person's computer on it are the two; the trait stays inside the
//! crate until a protocol for it is needed.

use std::collections::BTreeMap;
use std::time::Duration;

use glave_ports::BoxFuture;
use glave_ports::computer::{ComputerError, Isolation};
use glave_process::Glave;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::token::Token;

/// How long copying glave into a started worker may take: seconds into a container here or
/// through a cluster's API server, a minute or two over a slow link to one.
pub(crate) const COPIED_WITHIN: Duration = Duration::from_mins(5);

/// What one worker is started with: everything already checked against policy.
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
    /// The glave starting it, which the worker is labelled with, so that a glave starting
    /// later removes it once that one is gone; none where it cannot be told.
    pub(crate) starter: Option<Glave>,
}

/// The label (an annotation on Kubernetes) naming the glave that started a worker, as JSON.
pub(crate) const STARTED_BY: &str = "glave.started-by";

/// A worker a backend holds, as listed: its name, and the glave that started it as its label
/// says, if it says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Listed {
    pub(crate) name: String,
    pub(crate) started_by: Option<String>,
}

/// A stream to glave in a worker: what it reads, and what it writes.
pub(crate) type Stream = (
    Box<dyn AsyncRead + Unpin + Send>,
    Box<dyn AsyncWrite + Unpin + Send>,
);

/// `error`, with what a worker that did not start `said` last, when it said anything.
pub(crate) fn saying(error: ComputerError, said: &str) -> ComputerError {
    if said.trim().is_empty() {
        error
    } else {
        ComputerError::Failed(format!("{}; it said: {}", error.message(), said.trim()))
    }
}

/// A worker the backend started, by its name there: reached through bridges into it
/// ([`Backend::bridge`]), each signed in with its token.
#[derive(Debug)]
pub(crate) struct Started {
    pub(crate) name: String,
}

/// A place workers run: a Kubernetes cluster, or a person's computer on one.
pub(crate) trait Backend: Send + Sync {
    /// How workers here are kept apart from the machine they run on.
    fn isolation(&self) -> Isolation {
        Isolation::Container
    }

    /// Starts a worker from `spec`, with glave serving in it. A worker that cannot start is
    /// removed before this returns.
    fn start<'a>(&'a self, spec: &'a Spec) -> BoxFuture<'a, Result<Started, ComputerError>>;

    /// Gives the running worker `name` a fresh `token`, replacing the one it was started with,
    /// so a connection that died can be opened again after the first token has expired.
    fn renew<'a>(
        &'a self,
        name: &'a str,
        token: &'a Token,
    ) -> BoxFuture<'a, Result<(), ComputerError>>;

    /// A new bridge into the running worker `name`: a stream to glave there, a connection of
    /// its own each time.
    fn bridge<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Stream, ComputerError>>;

    /// Every worker here, with this backend's labels, whoever started it.
    fn list(&self) -> BoxFuture<'_, Result<Vec<Listed>, ComputerError>>;

    /// Removes the worker named `name` and its disk. A worker already gone is not an error.
    fn remove<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<(), ComputerError>>;
}
