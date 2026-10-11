//! glave's computers (`specs/computers.md`): this machine as a computer, the environments
//! computers are made from, workers (disposable computers on Docker), and the JSON-RPC
//! protocol that serves a computer to a harness elsewhere and reaches one from here.
//!
//! [`LocalComputers`] runs work on the machine it is on. [`serve`] offers any computers over
//! one connection, and [`RemoteComputers`] is the other end of it: both pass the same
//! conformance suite, so a harness cannot tell a computer here from one across the wire.
//! [`WorkerComputers`] starts a worker per computer and reaches it over TLS ([`serve_tls`],
//! [`connect_tls`]) with a short-lived [`Token`].

mod cancel;
pub mod environment;
mod local;
mod remote;
mod serve;
mod tls;
mod token;
mod wire;
mod worker;
pub mod worktrees;

pub use cancel::Switch;
pub use environment::{EnvironmentError, Found, read_environment};
pub use local::{LocalComputers, sandbox_available};
pub use remote::RemoteComputers;
pub use serve::{serve, serve_tcp, serve_tls};
pub use tls::{Fingerprint, Identity, connect_tls};
pub use token::{MIN_SECRET, Token};
pub use worker::{Clock, Credentials, Docker, DockerSettings, Limits, WorkerComputers, https_url};
pub use worktrees::{Condition, Kept, Released, Trash};
