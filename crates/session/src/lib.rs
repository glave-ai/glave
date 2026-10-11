//! glave's session API: the one boundary every surface uses to run threads and turns. The
//! server keeps the methods and notifications of `specs/session.md`; each transport, from
//! in-process to the network, carries the same messages. Network transports need a sign-in.

mod approvals;
pub mod cleanup;
mod goal;
mod held;
pub mod http;
mod hub;
mod keyed;
pub mod protocol;
pub mod rpc;
mod server;
pub mod signin;
mod standing;
mod turn;
mod worktrees;

pub use approvals::Answering;
pub use glave_harness::Caller;
pub use server::{Server, Wiring};
pub use turn::{TurnItem, TurnStream};
pub use worktrees::info as worktree_info;
