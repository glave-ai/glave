//! glave's computers (`specs/computers.md`): this machine as a computer, the environments
//! computers are made from, workers (disposable computers on Docker or Kubernetes), and the JSON-RPC
//! protocol that serves a computer to a harness elsewhere and reaches one from here.
//!
//! [`LocalComputers`] runs work on the machine it is on. [`serve`](fn@serve) offers any computers over
//! one connection, and [`RemoteComputers`] is the other end of it: both pass the same
//! conformance suite, so a harness cannot tell a computer here from one across the wire.
//! [`WorkerComputers`] starts a worker per computer and reaches it over TLS ([`serve_tls`],
//! [`connect_tls`]) on Docker, or through a [`bridge`] to a Unix socket ([`serve_socket`]) on
//! Kubernetes, with a short-lived [`Token`].

mod cancel;
pub mod environment;
mod local;
mod remote;
mod serve;
mod tls;
mod token;
mod waking;
mod wire;
mod worker;
pub mod worktrees;

pub use cancel::CancelSwitch;
pub use environment::{EnvironmentError, Found, read_environment};
pub use local::{LocalComputers, glave_on_path, sandbox_available, sandbox_helper};
pub use remote::RemoteComputers;
pub use serve::{Idle, bridge, listen_socket, serve, serve_socket, serve_tcp, serve_tls};
pub use tls::{Fingerprint, Identity, connect_tls};
pub use token::{MIN_SECRET, Token, Tokens};
pub use waking::{Grow, Short, Wake, Waking};
pub use worker::{Docker, DockerSettings, WorkerComputers, WorkerCredentials, https_url};
#[cfg(feature = "kubernetes")]
pub use worker::{Holding, Kubernetes, KubernetesSettings, Leasing, People};
pub use worktrees::{Condition, Kept, Released, Trash};
