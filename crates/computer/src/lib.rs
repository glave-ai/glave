//! glave's computers (`specs/computers.md`): this machine as a computer, people's computers on
//! Kubernetes (the `kubernetes` feature), and the JSON-RPC protocol that serves a computer to a
//! harness elsewhere and reaches one from here.
//!
//! [`LocalComputers`] runs work on the machine it is on. [`serve`](fn@serve) offers any computers over
//! one connection, and [`RemoteComputers`] is the other end of it: both pass the same
//! conformance suite, so a harness cannot tell a computer here from one across the wire. On
//! Kubernetes, a computer in a Pod serves on a Unix socket ([`serve_socket`]) and is reached
//! through a [`bridge`] to it, with a short-lived [`Token`].

mod cancel;
mod disk;
mod local;
mod remote;
mod serve;
mod tls;
mod token;
mod waking;
mod wire;
#[cfg(feature = "kubernetes")]
mod worker;
pub mod worktrees;

pub use cancel::CancelSwitch;
pub use disk::{Keeper, Largest};
pub use local::{
    LocalComputers, ask_repository, glave_on_path, sandbox_available, sandbox_helper,
    terminal_helper,
};
pub use remote::RemoteComputers;
pub use serve::{Idle, bridge, listen_socket, serve, serve_socket, serve_tcp, serve_tls};
pub use tls::{Fingerprint, Identity, connect_tls};
pub use token::{MIN_SECRET, Token, Tokens};
pub use waking::{Grow, Peek, Short, Wake, Waking};
#[cfg(feature = "kubernetes")]
pub use worker::{
    Holding, Kubernetes, KubernetesSettings, Leasing, People, WorkerComputers, person_sandbox,
};
pub use worktrees::{Condition, Kept, Released, Trash};
